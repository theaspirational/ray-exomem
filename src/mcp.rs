//! MCP (Model Context Protocol) JSON-RPC server.
//!
//! Exposes ray-exomem capabilities over the Streamable HTTP transport on
//! `/mcp`. This server is stateless, but it still provides the GET/POST/DELETE
//! surface many MCP clients expect.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    Json,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::IntervalStream;

use crate::auth::{middleware::MaybeUser, User};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    fn ok(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Option<serde_json::Value>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn mcp_handler(
    State(state): State<Arc<AppState>>,
    user: User,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let id = req.id.clone();
    let result = match req.method.as_str() {
        "initialize" => handle_initialize(),
        "notifications/initialized" => Ok(serde_json::json!({})),
        "tools/list" => Ok(handle_tools_list()),
        "resources/list" => Ok(handle_resources_list()),
        "resources/read" => handle_resources_read(req.params),
        "prompts/list" => Ok(serde_json::json!({ "prompts": [] })),
        "tools/call" => handle_tool_call(&state, &user, req.params).await,
        _ => Err(JsonRpcError {
            code: -32601,
            message: format!("method not found: {}", req.method),
        }),
    };
    let resp = match result {
        Ok(value) => JsonRpcResponse::ok(id, value),
        Err(error) => JsonRpcResponse::err(id, error),
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        "mcp-protocol-version",
        HeaderValue::from_static("2024-11-05"),
    );
    (headers, Json(resp)).into_response()
}

pub async fn mcp_stream_handler(
    _state: State<Arc<AppState>>,
    _maybe_user: MaybeUser,
) -> impl IntoResponse {
    let events = IntervalStream::new(tokio::time::interval(Duration::from_secs(15)))
        .map(|_| Ok::<Event, std::convert::Infallible>(Event::default().comment("keepalive")));
    let mut headers = HeaderMap::new();
    headers.insert(
        "mcp-protocol-version",
        HeaderValue::from_static("2024-11-05"),
    );
    (headers, Sse::new(events)).into_response()
}

pub async fn mcp_delete_handler(_maybe_user: MaybeUser) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        "mcp-protocol-version",
        HeaderValue::from_static("2024-11-05"),
    );
    (StatusCode::NO_CONTENT, headers).into_response()
}

// ---------------------------------------------------------------------------
// initialize
// ---------------------------------------------------------------------------

fn handle_initialize() -> Result<serde_json::Value, JsonRpcError> {
    Ok(serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {},
            "resources": {},
            "prompts": {}
        },
        "serverInfo": {
            "name": "ray-exomem",
            "version": crate::frontend_version()
        }
    }))
}

// ---------------------------------------------------------------------------
// resources/list + resources/read
// ---------------------------------------------------------------------------

const AGENT_GUIDE_URI: &str = "exomem://docs/agent_guide";
const AGENT_GUIDE_BODY: &str = include_str!("../docs/agent_guide.md");

fn handle_resources_list() -> serde_json::Value {
    serde_json::json!({
        "resources": [{
            "uri": AGENT_GUIDE_URI,
            "name": "Ray-exomem agent guide",
            "description": "How to use the ray-exomem MCP — tree model, typed values, tool reference, common errors.",
            "mimeType": "text/markdown"
        }]
    })
}

fn handle_resources_read(
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, JsonRpcError> {
    let uri = params
        .as_ref()
        .and_then(|p| p.get("uri"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "missing 'uri' parameter".into(),
        })?;
    if uri != AGENT_GUIDE_URI {
        return Err(JsonRpcError {
            code: -32602,
            message: format!("unknown resource: {uri}"),
        });
    }
    Ok(serde_json::json!({
        "contents": [{
            "uri": AGENT_GUIDE_URI,
            "mimeType": "text/markdown",
            "text": AGENT_GUIDE_BODY,
        }]
    }))
}

// ---------------------------------------------------------------------------
// tools/list
// ---------------------------------------------------------------------------

fn handle_tools_list() -> serde_json::Value {
    serde_json::json!({ "tools": tool_definitions() })
}

fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "guide",
            "description": "Return the ray-exomem agent guide (markdown). Read this first in a fresh session — covers the tree model, typed values, write/read patterns, and the full tool reference.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        serde_json::json!({
            "name": "query",
            "description": "Run a Rayfall query against an exom",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Rayfall query expression" },
                    "exom": { "type": "string", "description": "Exom name (defaults to main)" },
                    "branch": { "type": "string", "description": "Branch to query (defaults to the exom's current branch). Switches the brain's view for the duration of the query, then restores. Use to inspect tx/facts/observations/beliefs on sub-agent branches without persistently changing the cursor." }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "assert_fact",
            "description": "Assert or replace a fact in an exom. Re-asserting an existing fact_id supersedes the previous tuple.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name (slash or :: form)." },
                    "predicate": { "type": "string", "description": "Fact predicate, e.g. `entity/name`." },
                    "value": { "description": "Typed value. JSON number → I64, JSON string → auto-detected (numeric round-trip → I64, else Str), `{\"$sym\": \"...\"}` → Sym." },
                    "fact_id": { "type": "string", "description": "Stable fact id; defaults to the predicate. Use `<entity>#<property>` for multi-instance entities." },
                    "confidence": { "type": "number", "description": "0.0..1.0; defaults to 1.0." },
                    "source": { "type": "string", "description": "Provenance tag (where this fact came from). Defaults to 'mcp'." },
                    "valid_from": { "type": "string", "description": "ISO-8601 wall-clock timestamp the fact starts being true. Defaults to now." },
                    "valid_to": { "type": "string", "description": "ISO-8601 wall-clock timestamp the fact stops being true. Open-ended if omitted." },
                    "agent": { "type": "string", "description": "Tool/integration making the call (e.g. `cursor`, `claude-code-cli`). Falls back to the API key's label. Recorded on the tx and rendered as `via <agent>`." },
                    "model": { "type": "string", "description": "LLM identity (e.g. `claude-opus-4-7`). Explicit only — no fallback. Rendered as `using <model>`." },
                    "branch": { "type": "string", "description": "Target branch for the write. Defaults to the exom's current branch (usually `main`). The exom is restored to its prior branch after the write." }
                },
                "required": ["exom", "predicate", "value"]
            }
        }),
        serde_json::json!({
            "name": "retract_fact",
            "description": "Retract the active tuple for `fact_id`. Closes valid_to to now and marks the fact revoked. History is preserved; fact_history still returns the closed tuple.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name." },
                    "fact_id": { "type": "string", "description": "Fact id to retract." },
                    "agent": { "type": "string", "description": "Tool/integration making the call. Falls back to the API key's label." },
                    "model": { "type": "string", "description": "LLM identity (e.g. `claude-opus-4-7`). Explicit only." },
                    "branch": { "type": "string", "description": "Target branch. Defaults to the exom's current branch." }
                },
                "required": ["exom", "fact_id"]
            }
        }),
        serde_json::json!({
            "name": "observe",
            "description": "Record an observation — a raw piece of evidence captured from a source (a doc, a chat, a code file). Cheaper than asserting a fact: observations don't claim truth, they record what was seen.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name." },
                    "obs_id": { "type": "string", "description": "Stable observation id. Convention: `obs/<source>#<topic>`." },
                    "source_type": { "type": "string", "description": "Source category, e.g. `notion-page`, `github-pr`, `chat`, `manual`." },
                    "source_ref": { "type": "string", "description": "Stable reference within that source, e.g. a Notion page id, PR number, message id." },
                    "content": { "type": "string", "description": "The observed content itself (a quote, summary, or paste)." },
                    "confidence": { "type": "number", "description": "0.0..1.0; how confident the agent is in this observation. Defaults to 0.8." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Free-form tags to aid retrieval." },
                    "valid_from": { "type": "string", "description": "ISO-8601; when the observed thing started being true. Defaults to now." },
                    "valid_to": { "type": "string", "description": "ISO-8601; when it stopped. Open-ended if omitted." },
                    "agent": { "type": "string", "description": "Tool/integration making the call. Falls back to the API key's label." },
                    "model": { "type": "string", "description": "LLM identity (e.g. `claude-opus-4-7`). Explicit only." },
                    "branch": { "type": "string", "description": "Target branch. Defaults to the exom's current branch." }
                },
                "required": ["exom", "obs_id", "source_type", "content"]
            }
        }),
        serde_json::json!({
            "name": "believe",
            "description": "Record (or revise) a belief — a claim the agent considers true, with confidence and rationale. Re-believing the same `claim_text` supersedes the prior active belief.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name." },
                    "belief_id": { "type": "string", "description": "Stable belief id. Convention: `belief/<topic>#<rev>`." },
                    "claim_text": { "type": "string", "description": "Natural-language claim, e.g. `service-foo will hit GA in Q3`." },
                    "confidence": { "type": "number", "description": "0.0..1.0. Defaults to 0.7." },
                    "rationale": { "type": "string", "description": "Why the agent holds this belief. Defaults to empty." },
                    "supports": { "type": "array", "items": { "type": "string" }, "description": "Fact ids or observation ids that support the claim." },
                    "valid_from": { "type": "string", "description": "ISO-8601; when the claim starts being true. Defaults to now." },
                    "valid_to": { "type": "string", "description": "ISO-8601; when it stops. Open-ended if omitted." },
                    "agent": { "type": "string", "description": "Tool/integration making the call. Falls back to the API key's label." },
                    "model": { "type": "string", "description": "LLM identity (e.g. `claude-opus-4-7`). Explicit only." },
                    "branch": { "type": "string", "description": "Target branch. Defaults to the exom's current branch." }
                },
                "required": ["exom", "belief_id", "claim_text"]
            }
        }),
        serde_json::json!({
            "name": "revoke_belief",
            "description": "Withdraw an active belief without supplying a replacement. Sets status to revoked, closes valid_to to now. History preserved; belief-row exposes it with status=\"revoked\". Use re-believe with a new claim_text instead if you have a replacement.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name." },
                    "belief_id": { "type": "string", "description": "Belief id to revoke. Must currently be active on the target branch." },
                    "agent": { "type": "string", "description": "Tool/integration making the call. Falls back to the API key's label." },
                    "model": { "type": "string", "description": "LLM identity (e.g. `claude-opus-4-7`). Explicit only." },
                    "branch": { "type": "string", "description": "Target branch. Defaults to the exom's current branch." }
                },
                "required": ["exom", "belief_id"]
            }
        }),
        serde_json::json!({
            "name": "list_exoms",
            "description": "List accessible exoms via the tree",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        serde_json::json!({
            "name": "exom_status",
            "description": "Get exom health and stats",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name" }
                },
                "required": ["exom"]
            }
        }),
        serde_json::json!({
            "name": "eval",
            "description": "Execute raw Rayfall source",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Rayfall source to evaluate" },
                    "exom": { "type": "string", "description": "Exom name (defaults to main)" },
                    "branch": { "type": "string", "description": "Branch to evaluate against (defaults to the exom's current branch). See `query` for semantics." }
                },
                "required": ["source"]
            }
        }),
        serde_json::json!({
            "name": "explain",
            "description": "Explain a predicate or fact by ID",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name" },
                    "predicate": { "type": "string", "description": "Predicate to explain" },
                    "fact_id": { "type": "string", "description": "Fact ID to explain" }
                },
                "required": ["exom"]
            }
        }),
        serde_json::json!({
            "name": "fact_history",
            "description": "Get fact detail and history",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name" },
                    "id": { "type": "string", "description": "Fact ID" }
                },
                "required": ["exom", "id"]
            }
        }),
        serde_json::json!({
            "name": "list_branches",
            "description": "List branches for an exom",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name" }
                },
                "required": ["exom"]
            }
        }),
        serde_json::json!({
            "name": "create_branch",
            "description": "Create a new branch in an exom",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name" },
                    "branch_name": { "type": "string", "description": "New branch name" },
                    "agent": { "type": "string", "description": "Tool/integration making the call. Falls back to the API key's label." },
                    "model": { "type": "string", "description": "LLM identity. Explicit only." }
                },
                "required": ["exom", "branch_name"]
            }
        }),
        serde_json::json!({
            "name": "session_new",
            "description": "Create a new session exom under <project>/sessions/<id>. The orchestrator (the authenticated user) gets the `main` branch with its claim recorded under `claimed_by_user_email` + `claimed_by_agent` + `claimed_by_model`; remaining agents each get a pre-allocated unclaimed branch named after their `agent_label`. Returns the new session exom path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_path": { "type": "string", "description": "Project path (must already be initialised). Slash or :: form." },
                    "session_type": { "type": "string", "enum": ["single", "multi"], "description": "`multi` pre-allocates one branch per agent (and `main` for the orchestrator). `single` only creates `main`." },
                    "label": { "type": "string", "description": "Display label. Must be non-empty and free of '/', '::', and whitespace." },
                    "agents": { "type": "array", "items": { "type": "string" }, "description": "Sub-agent labels. Each gets a pre-allocated branch (named after the label) which they later claim via `session_join`. Ignored for `single` sessions." },
                    "agent": { "type": "string", "description": "Tool/integration the orchestrator is using. Falls back to the API key's label. Recorded on the `main` branch." },
                    "model": { "type": "string", "description": "LLM the orchestrator is running. Explicit only." }
                },
                "required": ["project_path", "session_type", "label"]
            }
        }),
        serde_json::json!({
            "name": "session_join",
            "description": "Claim a pre-allocated sub-agent branch in a multi-agent session under TOFU (first writer wins). Branch is claimed by the authenticated user with the supplied `agent`/`model` recorded for audit. Returns the branch name claimed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_path": { "type": "string", "description": "Full path to the session exom, e.g. `public/work/x/y/sessions/<id>`." },
                    "agent_label": { "type": "string", "description": "Sub-agent branch label within the session to claim (must match one of the labels passed to `session_new`)." },
                    "agent": { "type": "string", "description": "Tool/integration the claimer is using. Falls back to the API key's label." },
                    "model": { "type": "string", "description": "LLM the claimer is running. Explicit only." }
                },
                "required": ["session_path", "agent_label"]
            }
        }),
        serde_json::json!({
            "name": "session_close",
            "description": "Close a session — sets `session/closed_at = now`, after which the brain rejects all writes against the session exom. History is preserved.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_path": { "type": "string", "description": "Full path to the session exom." },
                    "agent": { "type": "string", "description": "Tool/integration making the call. Falls back to the API key's label." },
                    "model": { "type": "string", "description": "LLM identity (e.g. `claude-opus-4-7`). Explicit only." }
                },
                "required": ["session_path"]
            }
        }),
        serde_json::json!({
            "name": "schema",
            "description": "Get the schema for an exom",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name" }
                },
                "required": ["exom"]
            }
        }),
        serde_json::json!({
            "name": "export",
            "description": "Export exom data",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name" },
                    "format": { "type": "string", "description": "Export format (jsonl, json)" }
                },
                "required": ["exom"]
            }
        }),
        serde_json::json!({
            "name": "init",
            "description": "Scaffold a new project at <path>: creates `<path>/main` (the project's main exom) plus an empty `<path>/sessions/` folder. Idempotent — re-running on an already-initialised project is a no-op.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Tree path for the project (slash or :: form), e.g. `public/example/getting-started` or `work::team::project`." }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "exom_new",
            "description": "Create a bare exom at the given tree path. Use for free-standing exoms not attached to a project (e.g. scratch, ad-hoc namespaces). For project scaffolding use `init` instead. Idempotent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Full tree path for the new exom (slash or :: form)." }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "tree",
            "description": "Walk the auth-aware tree for the calling user. Returns the user's own namespace, plus any namespaces they have shares for, plus the public/* subtree. Use to discover which projects/sessions exist before calling `init`/`session_new`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Optional sub-path; if omitted, walks the full visible root." },
                    "depth": { "type": "integer", "description": "Optional max depth. Defaults to unbounded." },
                    "include_archived": { "type": "boolean", "description": "Include archived nodes (default false)." },
                    "include_branches": { "type": "boolean", "description": "Include per-exom branch summaries (default false)." }
                },
                "required": []
            }
        }),
        serde_json::json!({
            "name": "merge_branch",
            "description": "Merge `branch` into the exom's current branch using the supplied policy. `last-writer-wins` overwrites conflicting target facts; `keep-target` skips conflicts; `manual` returns the conflict list without writing. Returns added fact ids, conflicts, and the merge tx id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name." },
                    "branch": { "type": "string", "description": "Source branch to merge from." },
                    "policy": { "type": "string", "enum": ["last-writer-wins", "keep-target", "manual"], "description": "Conflict-resolution policy. Defaults to `last-writer-wins`." },
                    "agent": { "type": "string", "description": "Tool/integration making the call. Falls back to the API key's label." },
                    "model": { "type": "string", "description": "LLM identity. Explicit only." }
                },
                "required": ["exom", "branch"]
            }
        }),
        serde_json::json!({
            "name": "archive_branch",
            "description": "Soft-delete a branch (sets archived=true). Cannot archive `main`. The branch's history is preserved; subsequent reads filter it out.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name." },
                    "branch": { "type": "string", "description": "Branch to archive." }
                },
                "required": ["exom", "branch"]
            }
        }),
    ]
}

// ---------------------------------------------------------------------------
// tools/call dispatch
// ---------------------------------------------------------------------------

async fn handle_tool_call(
    state: &AppState,
    user: &User,
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError {
        code: -32602,
        message: "missing params".into(),
    })?;

    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "missing tool name in params".into(),
        })?;

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let content = match tool_name {
        "guide" => Ok(AGENT_GUIDE_BODY.to_string()),
        "query" => tool_query(state, &arguments),
        "assert_fact" => tool_assert_fact(state, user, &arguments).await,
        "retract_fact" => tool_retract_fact(state, user, &arguments).await,
        "observe" => tool_observe(state, user, &arguments).await,
        "believe" => tool_believe(state, user, &arguments).await,
        "revoke_belief" => tool_revoke_belief(state, user, &arguments).await,
        "list_exoms" => tool_list_exoms(state),
        "exom_status" => tool_exom_status(state, &arguments),
        "eval" => tool_eval(state, &arguments),
        "explain" => tool_explain(state, &arguments),
        "fact_history" => tool_fact_history(state, &arguments),
        "list_branches" => tool_list_branches(state, &arguments),
        "create_branch" => tool_create_branch(state, user, &arguments).await,
        "session_new" => tool_session_new(state, user, &arguments),
        "session_join" => tool_session_join(state, user, &arguments),
        "session_close" => tool_session_close(state, user, &arguments).await,
        "schema" => tool_schema(state, &arguments),
        "export" => tool_export(state, &arguments),
        "init" => tool_init(state, user, &arguments).await,
        "exom_new" => tool_exom_new(state, user, &arguments).await,
        "tree" => tool_tree(state, user, &arguments).await,
        "merge_branch" => tool_merge_branch(state, user, &arguments).await,
        "archive_branch" => tool_archive_branch(state, user, &arguments).await,
        _ => Err(JsonRpcError {
            code: -32602,
            message: format!("unknown tool: {tool_name}"),
        }),
    }?;

    Ok(serde_json::json!({ "content": [{ "type": "text", "text": content }] }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_str<'a>(args: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn require_str<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, JsonRpcError> {
    get_str(args, key).ok_or_else(|| JsonRpcError {
        code: -32602,
        message: format!("missing required parameter: {key}"),
    })
}

fn exom_slug(args: &serde_json::Value) -> String {
    let raw = get_str(args, "exom").unwrap_or(crate::server::DEFAULT_EXOM);
    match raw.parse::<crate::path::TreePath>() {
        Ok(tp) => tp.to_slash_string(),
        Err(_) => raw.to_string(),
    }
}

/// Build a write-side `MutationContext` for an authenticated MCP call.
///
/// Three-axis attribution:
/// * `user_email` — authenticated user (always set; load-bearing for
///   permission checks and UI display).
/// * `agent` — explicit `agent` arg wins, otherwise falls back to the
///   authenticated user's `api_key_label` (Bearer auth) or `None` (cookie).
/// * `model` — explicit `model` arg only; no fallback.
fn mcp_mutation_ctx(user: &User, args: &serde_json::Value) -> crate::context::MutationContext {
    let agent = get_str(args, "agent")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| user.api_key_label.clone());
    let model = get_str(args, "model")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    crate::context::MutationContext {
        user_email: Some(user.email.clone()),
        agent,
        model,
        session: None,
    }
}

/// Run a write closure under an optional branch override. If `branch` is
/// supplied, switch the exom's `current_branch` for the duration of `f`,
/// then restore — even on error — so concurrent readers never observe the
/// switched cursor. Safe because `mutate_exom` holds an exclusive lock on
/// `state.exoms` for the entire closure.
fn with_optional_branch<R>(
    es: &mut crate::server::ExomState,
    branch: Option<&str>,
    f: impl FnOnce(&mut crate::server::ExomState) -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    let prev = match branch {
        Some(b) if b != es.brain.current_branch_id() => {
            let p = es.brain.current_branch_id().to_string();
            es.brain.switch_branch(b)?;
            Some(p)
        }
        _ => None,
    };
    let res = f(es);
    if let Some(p) = prev {
        let _ = es.brain.switch_branch(&p);
    }
    res
}

fn load_exom<'a>(
    state: &AppState,
    exoms: &'a mut std::collections::HashMap<String, crate::server::ExomState>,
    exom_slash: &str,
) -> Result<&'a mut crate::server::ExomState, JsonRpcError> {
    if !exoms.contains_key(exom_slash) {
        let tree_root = state.tree_root.as_deref();
        let sym_path = state.sym_path.as_deref();
        // Try to lazy-load. get_or_load_exom is private to server, so we
        // replicate the contains-key + insert pattern here. We'll just check
        // if the exom exists after the lock.
        if let (Some(tr), Some(sp)) = (tree_root, sym_path) {
            let disk = tr.join(exom_slash);
            let meta_p = disk.join(crate::exom::META_FILENAME);
            if meta_p.exists() {
                match crate::server::load_exom_from_tree_path(&disk, sp, exom_slash) {
                    Ok(es) => {
                        let _ = state
                            .engine
                            .bind_named_db(crate::storage::sym_intern(exom_slash), &es.datoms);
                        exoms.insert(exom_slash.to_string(), es);
                    }
                    Err(e) => {
                        return Err(JsonRpcError {
                            code: -32000,
                            message: format!("failed to load exom '{}': {}", exom_slash, e),
                        });
                    }
                }
            }
        }
    }
    exoms.get_mut(exom_slash).ok_or_else(|| JsonRpcError {
        code: -32000,
        message: format!("unknown exom '{}'", exom_slash),
    })
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

fn tool_query(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let query_str = require_str(args, "query")?;
    let exom_slash = exom_slug(args);
    let target_branch = get_str(args, "branch")
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut exoms = state.exoms.lock().unwrap();

    // Optional branch switch: save prev cursor, switch the brain, rebind the
    // engine's datoms to the target branch's view. Restored after the query
    // (best-effort) so the exom's cursor looks unchanged to other callers.
    let saved_branch = swap_to_branch(state, &mut exoms, &exom_slash, target_branch.as_deref())?;

    let outcome = run_query_body(state, &exoms, &exom_slash, query_str);

    if let Some(prev) = saved_branch {
        restore_branch(state, &mut exoms, &exom_slash, &prev);
    }

    outcome
}

/// Switch the exom's brain to `target` if supplied, returning the previous
/// branch cursor for later restoration. `Ok(None)` means no switch was
/// needed. Errors map to `unknown_branch`/`unknown_exom`.
fn swap_to_branch(
    state: &AppState,
    exoms: &mut std::collections::HashMap<String, crate::server::ExomState>,
    exom_slash: &str,
    target: Option<&str>,
) -> Result<Option<String>, JsonRpcError> {
    let Some(target) = target else { return Ok(None) };
    let prev = {
        let es = exoms.get_mut(exom_slash).ok_or_else(|| JsonRpcError {
            code: -32000,
            message: format!("unknown exom '{}'", exom_slash),
        })?;
        let prev = es.brain.current_branch_id().to_string();
        if prev == target {
            return Ok(None);
        }
        es.brain.switch_branch(target).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("unknown_branch: {e}"),
        })?;
        prev
    };
    crate::server::rebind_datoms_only(state, exoms, exom_slash).map_err(|e| JsonRpcError {
        code: -32000,
        message: format!("rebind failed: {e}"),
    })?;
    Ok(Some(prev))
}

/// Best-effort cursor restoration. Logs (silently) if rebind fails — the
/// query already returned successfully and we don't want to clobber that.
fn restore_branch(
    state: &AppState,
    exoms: &mut std::collections::HashMap<String, crate::server::ExomState>,
    exom_slash: &str,
    prev: &str,
) {
    if let Some(es) = exoms.get_mut(exom_slash) {
        let _ = es.brain.switch_branch(prev);
    }
    let _ = crate::server::rebind_datoms_only(state, exoms, exom_slash);
}

fn run_query_body(
    state: &AppState,
    exoms: &std::collections::HashMap<String, crate::server::ExomState>,
    _exom_slash: &str,
    query_str: &str,
) -> Result<String, JsonRpcError> {
    let expanded = crate::server::expand_query_validated(
        exoms,
        &state.engine,
        query_str,
        None,
        "mcp/query",
    )
    .map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
    })?;

    crate::server::bind_typed_facts_for_exom(&state.engine, exoms, &expanded.exom_name).map_err(
        |e| JsonRpcError {
            code: -32000,
            message: format!("failed to bind typed facts: {e}"),
        },
    )?;

    match state.engine.eval_raw(&expanded.expanded_query) {
        Ok(raw) => {
            let output =
                if unsafe { crate::ffi::ray_obj_type(raw.as_ptr()) } == crate::ffi::RAY_TABLE {
                    match crate::storage::decode_query_table(&raw, &expanded.normalized_query) {
                        Ok(d) => {
                            let formatted = crate::storage::format_decoded_query_table(&d);
                            if let Some(obj) = d.as_object() {
                                serde_json::to_string_pretty(&obj).unwrap_or(formatted)
                            } else {
                                formatted
                            }
                        }
                        Err(e) => {
                            return Err(JsonRpcError {
                                code: -32000,
                                message: e.to_string(),
                            })
                        }
                    }
                } else {
                    match state.engine.format_obj(&raw) {
                        Ok(s) => s,
                        Err(e) => {
                            return Err(JsonRpcError {
                                code: -32000,
                                message: e.to_string(),
                            })
                        }
                    }
                };
            Ok(output)
        }
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

async fn tool_assert_fact(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let predicate = require_str(args, "predicate")?.to_string();
    // MCP accepts typed JSON values (20 / "text" / {"$sym": "foo"}) plus bare
    // strings for legacy clients. Bare strings run through `FactValue::auto`
    // so numeric input like "75" is stored as I64, enabling datalog cmp rules
    // over the fact.
    let value_raw = args.get("value").cloned().unwrap_or(serde_json::Value::Null);
    let value = match &value_raw {
        serde_json::Value::Null => {
            return Err(JsonRpcError {
                code: -32602,
                message: "missing required argument 'value'".into(),
            });
        }
        serde_json::Value::String(s) => crate::fact_value::FactValue::auto(s),
        other => serde_json::from_value::<crate::fact_value::FactValue>(other.clone())
            .map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("invalid 'value': {e}"),
            })?,
    };
    let fact_id = get_str(args, "fact_id").unwrap_or(&predicate).to_string();

    let confidence = args.get("confidence").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let source = get_str(args, "source").unwrap_or("mcp").to_string();
    let valid_from = get_str(args, "valid_from").map(str::to_string);
    let valid_to = get_str(args, "valid_to").map(str::to_string);
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = mcp_mutation_ctx(user, args);

    let result = crate::server::mutate_exom_async(state, &exom_slash, |es| {
        with_optional_branch(es, branch.as_deref(), |es| {
            es.brain.assert_fact(
                &fact_id,
                &predicate,
                value.clone(),
                confidence,
                &source,
                valid_from.as_deref(),
                valid_to.as_deref(),
                &ctx,
            )
        })
    })
    .await;

    match result {
        Ok(tx_id) => Ok(serde_json::json!({
            "ok": true,
            "tx_id": tx_id,
            "fact_id": fact_id,
            "predicate": predicate,
            "confidence": confidence,
            "source": source,
            "branch": branch,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

async fn tool_retract_fact(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let fact_id = require_str(args, "fact_id")?.to_string();
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = mcp_mutation_ctx(user, args);

    let result = crate::server::mutate_exom_async(state, &exom_slash, |es| {
        with_optional_branch(es, branch.as_deref(), |es| {
            es.brain.retract_fact(&fact_id, &ctx)
        })
    })
    .await;

    match result {
        Ok(tx_id) => Ok(serde_json::json!({
            "ok": true,
            "tx_id": tx_id,
            "fact_id": fact_id,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

async fn tool_observe(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let obs_id = require_str(args, "obs_id")?.to_string();
    let source_type = require_str(args, "source_type")?.to_string();
    let source_ref = get_str(args, "source_ref").unwrap_or("").to_string();
    let content = require_str(args, "content")?.to_string();
    let confidence = args.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.8);
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let valid_from = get_str(args, "valid_from").map(str::to_string);
    let valid_to = get_str(args, "valid_to").map(str::to_string);
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = mcp_mutation_ctx(user, args);

    let result = crate::server::mutate_exom_async(state, &exom_slash, |es| {
        with_optional_branch(es, branch.as_deref(), |es| {
            es.brain.assert_observation(
                &obs_id,
                &source_type,
                &source_ref,
                &content,
                confidence,
                tags.clone(),
                valid_from.as_deref(),
                valid_to.as_deref(),
                &ctx,
            )
        })
    })
    .await;

    match result {
        Ok(tx_id) => Ok(serde_json::json!({
            "ok": true,
            "tx_id": tx_id,
            "obs_id": obs_id,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

async fn tool_believe(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let belief_id = require_str(args, "belief_id")?.to_string();
    let claim_text = require_str(args, "claim_text")?.to_string();
    let confidence = args.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.7);
    let rationale = get_str(args, "rationale").unwrap_or("").to_string();
    let supports: Vec<String> = args
        .get("supports")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let valid_from = get_str(args, "valid_from").map(str::to_string);
    let valid_to = get_str(args, "valid_to").map(str::to_string);
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = mcp_mutation_ctx(user, args);

    let result = crate::server::mutate_exom_async(state, &exom_slash, |es| {
        with_optional_branch(es, branch.as_deref(), |es| {
            es.brain.revise_belief(
                &belief_id,
                &claim_text,
                confidence,
                supports.clone(),
                &rationale,
                valid_from.as_deref(),
                valid_to.as_deref(),
                &ctx,
            )
        })
    })
    .await;

    match result {
        Ok(tx_id) => Ok(serde_json::json!({
            "ok": true,
            "tx_id": tx_id,
            "belief_id": belief_id,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

async fn tool_revoke_belief(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let belief_id = require_str(args, "belief_id")?.to_string();
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = mcp_mutation_ctx(user, args);

    let result = crate::server::mutate_exom_async(state, &exom_slash, |es| {
        with_optional_branch(es, branch.as_deref(), |es| {
            es.brain.revoke_belief(&belief_id, &ctx)
        })
    })
    .await;

    match result {
        Ok(tx_id) => Ok(serde_json::json!({
            "ok": true,
            "tx_id": tx_id,
            "belief_id": belief_id,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

fn tool_list_exoms(state: &AppState) -> Result<String, JsonRpcError> {
    let exoms = state.exoms.lock().unwrap();
    let names: Vec<&String> = exoms.keys().collect();
    Ok(serde_json::json!({ "exoms": names }).to_string())
}

fn tool_exom_status(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let mut exoms = state.exoms.lock().unwrap();
    let es = load_exom(state, &mut exoms, &exom_slash)?;
    let brain = &es.brain;
    let facts = brain.current_facts();
    let beliefs = brain.current_beliefs();
    Ok(serde_json::json!({
        "exom": exom_slash,
        "current_branch": brain.current_branch_id(),
        "facts": facts.len(),
        "beliefs": beliefs.len(),
        "transactions": brain.transactions().len(),
    })
    .to_string())
}

fn tool_eval(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let source = require_str(args, "source")?;
    let target_branch = get_str(args, "branch")
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // No branch arg: original fast path — eval against whatever the engine
    // currently has bound, no exom-level coordination.
    if target_branch.is_none() {
        return match state.engine.eval(source) {
            Ok(output) => Ok(output),
            Err(e) => Err(JsonRpcError {
                code: -32000,
                message: e.to_string(),
            }),
        };
    }

    // Branch arg supplied: must coordinate at exom granularity.
    let exom_slash = exom_slug(args);
    let mut exoms = state.exoms.lock().unwrap();
    let saved_branch = swap_to_branch(state, &mut exoms, &exom_slash, target_branch.as_deref())?;

    // Rebind typed-fact env names so rule bodies referencing facts_i64 etc.
    // see the target branch's view.
    let bind_result = crate::server::bind_typed_facts_for_exom(&state.engine, &exoms, &exom_slash)
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("failed to bind typed facts: {e}"),
        });

    let outcome = bind_result.and_then(|()| {
        state.engine.eval(source).map_err(|e| JsonRpcError {
            code: -32000,
            message: e.to_string(),
        })
    });

    if let Some(prev) = saved_branch {
        restore_branch(state, &mut exoms, &exom_slash, &prev);
    }

    outcome
}

fn tool_explain(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let mut exoms = state.exoms.lock().unwrap();
    let es = load_exom(state, &mut exoms, &exom_slash)?;
    let brain = &es.brain;

    if let Some(fact_id) = get_str(args, "fact_id") {
        let history = brain.explain(fact_id);
        let events: Vec<serde_json::Value> = history
            .iter()
            .map(|tx| {
                serde_json::json!({
                    "tx_id": tx.tx_id,
                    "action": tx.action.to_string(),
                })
            })
            .collect();
        return Ok(serde_json::json!({ "fact_id": fact_id, "events": events }).to_string());
    }

    if let Some(predicate) = get_str(args, "predicate") {
        let facts: Vec<serde_json::Value> = brain
            .current_facts()
            .into_iter()
            .filter(|f| f.predicate == predicate)
            .map(|f| {
                serde_json::json!({
                    "fact_id": f.fact_id,
                    "value": f.value,
                    "confidence": f.confidence,
                })
            })
            .collect();
        return Ok(serde_json::json!({ "predicate": predicate, "facts": facts }).to_string());
    }

    Err(JsonRpcError {
        code: -32602,
        message: "either predicate or fact_id is required".into(),
    })
}

fn tool_fact_history(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let id = require_str(args, "id")?;
    let mut exoms = state.exoms.lock().unwrap();
    let es = load_exom(state, &mut exoms, &exom_slash)?;
    let brain = &es.brain;
    let history = brain.fact_history(id);
    let tx_index: std::collections::HashMap<crate::brain::TxId, &crate::brain::Tx> =
        brain.transactions().iter().map(|t| (t.tx_id, t)).collect();
    let entries: Vec<serde_json::Value> = history
        .iter()
        .map(|f| {
            let tx = tx_index.get(&f.created_by_tx);
            serde_json::json!({
                "fact_id": f.fact_id,
                "predicate": f.predicate,
                "value": f.value,
                "confidence": f.confidence,
                "valid_from": f.valid_from,
                "valid_to": f.valid_to,
                "created_at": f.created_at,
                "tx_id": f.created_by_tx,
                "user_email": tx.and_then(|t| t.user_email.as_deref()),
                "agent": tx.and_then(|t| t.agent.as_deref()),
                "model": tx.and_then(|t| t.model.as_deref()),
            })
        })
        .collect();
    Ok(serde_json::json!({ "id": id, "history": entries }).to_string())
}

fn tool_list_branches(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let mut exoms = state.exoms.lock().unwrap();
    let es = load_exom(state, &mut exoms, &exom_slash)?;
    let branches: Vec<serde_json::Value> = es
        .brain
        .branches()
        .iter()
        .map(|b| {
            serde_json::json!({
                "branch_id": b.branch_id,
                "name": b.name,
                "parent_branch_id": b.parent_branch_id,
                "is_current": b.branch_id == es.brain.current_branch_id(),
                "claimed_by_user_email": b.claimed_by_user_email,
                "claimed_by_agent": b.claimed_by_agent,
                "claimed_by_model": b.claimed_by_model,
            })
        })
        .collect();
    Ok(serde_json::json!({ "branches": branches }).to_string())
}

async fn tool_create_branch(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let branch_name = require_str(args, "branch_name")?;

    let ctx = mcp_mutation_ctx(user, args);

    let result = crate::server::mutate_exom_async(state, &exom_slash, |es| {
        es.brain.create_branch(branch_name, branch_name, &ctx)
    })
    .await;

    match result {
        Ok(branch_id) => Ok(serde_json::json!({
            "ok": true,
            "branch_id": branch_id,
            "name": branch_name,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

fn tool_session_new(state: &AppState, user: &User, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let project_path_str = require_str(args, "project_path")?;
    let project_path: crate::path::TreePath =
        project_path_str.parse().map_err(|e: crate::path::PathError| JsonRpcError {
            code: -32602,
            message: format!("invalid project_path: {e}"),
        })?;
    let session_type = match require_str(args, "session_type")? {
        "multi" => crate::exom::SessionType::Multi,
        "single" => crate::exom::SessionType::Single,
        other => {
            return Err(JsonRpcError {
                code: -32602,
                message: format!("unknown session_type {:?}; use 'multi' or 'single'", other),
            });
        }
    };
    let label = require_str(args, "label")?;
    let agents: Vec<String> = args
        .get("agents")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let agent = get_str(args, "agent")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| user.api_key_label.clone());
    let model = get_str(args, "model")
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let tree_root = state.tree_root.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no tree_root configured".into(),
    })?;
    let sym_path = state.sym_path.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no sym_path configured".into(),
    })?;

    match crate::brain::session_new(
        tree_root,
        sym_path,
        &project_path,
        session_type,
        label,
        user.email.as_str(),
        agent.as_deref(),
        model.as_deref(),
        &agents,
    ) {
        Ok(session_path) => {
            let _ = state.sse_tx.send((
                None,
                r#"{"v":1,"kind":"tree-changed","op":"session_new"}"#.to_string(),
            ));
            Ok(serde_json::json!({
                "ok": true,
                "session_path": session_path.to_slash_string(),
            })
            .to_string())
        }
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

fn tool_session_join(state: &AppState, user: &User, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let session_path_str = require_str(args, "session_path")?;
    let session_path: crate::path::TreePath =
        session_path_str.parse().map_err(|e: crate::path::PathError| JsonRpcError {
            code: -32602,
            message: format!("invalid session_path: {e}"),
        })?;
    let agent_label = require_str(args, "agent_label")?;

    let agent = get_str(args, "agent")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| user.api_key_label.clone());
    let model = get_str(args, "model")
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let tree_root = state.tree_root.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no tree_root configured".into(),
    })?;
    let sym_path = state.sym_path.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no sym_path configured".into(),
    })?;

    match crate::brain::session_join(
        tree_root,
        sym_path,
        &session_path,
        agent_label,
        user.email.as_str(),
        agent.as_deref(),
        model.as_deref(),
    ) {
        Ok(branch) => {
            // session_join writes branch claim fields directly to disk via
            // brain::claim_branch. The cached ExomState (Brain + datoms) for
            // this exom is now stale — evict so the next read reloads from
            // disk and surfaces the claim triple to list_branches and EAV.
            state
                .exoms
                .lock()
                .unwrap()
                .remove(&session_path.to_slash_string());
            Ok(serde_json::json!({
                "ok": true,
                "session_path": session_path.to_slash_string(),
                "user_email": user.email,
                "branch": branch,
            })
            .to_string())
        }
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

async fn tool_session_close(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let session_path_str = require_str(args, "session_path")?;
    let session_path: crate::path::TreePath =
        session_path_str.parse().map_err(|e: crate::path::PathError| JsonRpcError {
            code: -32602,
            message: format!("invalid session_path: {e}"),
        })?;
    let exom_slash = session_path.to_slash_string();

    let ctx = mcp_mutation_ctx(user, args);

    let now = crate::brain::now_iso();
    let closed_at = now.clone();
    let result = crate::server::mutate_exom_async(state, &exom_slash, move |es| {
        es.brain.assert_fact(
            "session/closed_at",
            "session/closed_at",
            crate::fact_value::FactValue::Str(closed_at.clone()),
            1.0,
            "mcp",
            None,
            None,
            &ctx,
        )
    })
    .await;

    match result {
        Ok(tx_id) => Ok(serde_json::json!({
            "ok": true,
            "tx_id": tx_id,
            "session_path": exom_slash,
            "closed_at": now,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

fn tool_schema(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let mut exoms = state.exoms.lock().unwrap();
    let es = load_exom(state, &mut exoms, &exom_slash)?;
    let ontology = crate::system_schema::build_exom_ontology(&exom_slash, &es.brain, &es.rules);
    Ok(serde_json::to_string_pretty(&ontology).unwrap_or_else(|_| "{}".into()))
}

async fn tool_init(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let path_str = require_str(args, "path")?;
    let path: crate::path::TreePath = path_str.parse().map_err(|e: crate::path::PathError| {
        JsonRpcError {
            code: -32602,
            message: format!("invalid path: {e}"),
        }
    })?;
    let path_slash = path.to_slash_string();

    if let Some(ref auth_store) = state.auth_store {
        let level = crate::auth::access::resolve_access(user, &path_slash, auth_store).await;
        if !level.can_write() {
            return Err(JsonRpcError {
                code: -32000,
                message: format!("forbidden: write access denied to {}", path_slash),
            });
        }
    }

    let tree_root = state.tree_root.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no tree_root configured".into(),
    })?;

    crate::scaffold::init_project(tree_root, &path).map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
    })?;

    let _ = state.sse_tx.send((
        None,
        r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string(),
    ));

    Ok(serde_json::json!({
        "ok": true,
        "path": path_slash,
    })
    .to_string())
}

async fn tool_exom_new(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let path_str = require_str(args, "path")?;
    let path: crate::path::TreePath = path_str.parse().map_err(|e: crate::path::PathError| {
        JsonRpcError {
            code: -32602,
            message: format!("invalid path: {e}"),
        }
    })?;
    let path_slash = path.to_slash_string();

    if let Some(ref auth_store) = state.auth_store {
        let level = crate::auth::access::resolve_access(user, &path_slash, auth_store).await;
        if !level.can_write() {
            return Err(JsonRpcError {
                code: -32000,
                message: format!("forbidden: write access denied to {}", path_slash),
            });
        }
    }

    let tree_root = state.tree_root.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no tree_root configured".into(),
    })?;

    crate::scaffold::new_bare_exom(tree_root, &path).map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
    })?;

    let _ = state.sse_tx.send((
        None,
        r#"{"v":1,"kind":"tree-changed","op":"tree_changed"}"#.to_string(),
    ));

    Ok(serde_json::json!({
        "ok": true,
        "path": path_slash,
    })
    .to_string())
}

async fn tool_tree(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let depth = args.get("depth").and_then(|v| v.as_u64()).map(|n| n as usize);
    let include_archived = args
        .get("include_archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let include_branches = args
        .get("include_branches")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let opts = crate::tree::WalkOptions {
        depth: depth.or(Some(usize::MAX)),
        include_archived,
        include_branches,
        include_activity: false,
    };

    let node = match get_str(args, "path").filter(|s| !s.is_empty()) {
        None => crate::server::build_tree_root_for_user(state, user, &opts)
            .await
            .map_err(|e| JsonRpcError {
                code: -32000,
                message: format!("io: {e}"),
            })?,
        Some(p) => {
            let path: crate::path::TreePath =
                p.parse().map_err(|e: crate::path::PathError| JsonRpcError {
                    code: -32602,
                    message: format!("invalid path: {e}"),
                })?;
            let path_slash = path.to_slash_string();
            if let Some(ref auth_store) = state.auth_store {
                let level =
                    crate::auth::access::resolve_access(user, &path_slash, auth_store).await;
                if !level.can_read() {
                    return Err(JsonRpcError {
                        code: -32000,
                        message: format!("forbidden: read access denied to {}", path_slash),
                    });
                }
            }
            let tree_root = state.tree_root.as_deref().ok_or_else(|| JsonRpcError {
                code: -32000,
                message: "daemon has no tree_root configured".into(),
            })?;
            let sym_path = state.sym_path.as_deref().ok_or_else(|| JsonRpcError {
                code: -32000,
                message: "daemon has no sym_path configured".into(),
            })?;
            crate::tree::walk(tree_root, sym_path, &path, &opts).map_err(|e| JsonRpcError {
                code: -32000,
                message: format!("io: {e}"),
            })?
        }
    };

    serde_json::to_string_pretty(&node).map_err(|e| JsonRpcError {
        code: -32000,
        message: format!("serde: {e}"),
    })
}

async fn tool_merge_branch(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let source_branch = require_str(args, "branch")?.to_string();
    let policy = match get_str(args, "policy").unwrap_or("last-writer-wins") {
        "last-writer-wins" => crate::brain::MergePolicy::LastWriterWins,
        "keep-target" => crate::brain::MergePolicy::KeepTarget,
        "manual" => crate::brain::MergePolicy::Manual,
        other => {
            return Err(JsonRpcError {
                code: -32602,
                message: format!("unknown merge policy {:?}", other),
            });
        }
    };

    if let Some(ref auth_store) = state.auth_store {
        let level = crate::auth::access::resolve_access(user, &exom_slash, auth_store).await;
        if !level.can_write() {
            return Err(JsonRpcError {
                code: -32000,
                message: format!("forbidden: write access denied to {}", exom_slash),
            });
        }
    }

    let ctx = mcp_mutation_ctx(user, args);

    let result = crate::server::mutate_exom_async(state, &exom_slash, move |es| {
        let target = es.brain.current_branch_id().to_string();
        es.brain.merge_branch(&source_branch, &target, policy, &ctx)
    })
    .await;

    match result {
        Ok(merge_result) => Ok(serde_json::json!({
            "ok": true,
            "added": merge_result.added,
            "conflicts": merge_result.conflicts.iter().map(|c| serde_json::json!({
                "fact_id": c.fact_id,
                "predicate": c.predicate,
                "source_value": c.source_value,
                "target_value": c.target_value,
            })).collect::<Vec<_>>(),
            "tx_id": merge_result.tx_id,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

async fn tool_archive_branch(
    state: &AppState,
    user: &User,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let branch = require_str(args, "branch")?.to_string();

    if let Some(ref auth_store) = state.auth_store {
        let level = crate::auth::access::resolve_access(user, &exom_slash, auth_store).await;
        if !level.can_write() {
            return Err(JsonRpcError {
                code: -32000,
                message: format!("forbidden: write access denied to {}", exom_slash),
            });
        }
    }

    let bid = branch.clone();
    let result = crate::server::mutate_exom_async(state, &exom_slash, move |es| {
        es.brain.archive_branch(&bid)?;
        Ok(())
    })
    .await;

    match result {
        Ok(()) => Ok(serde_json::json!({
            "ok": true,
            "exom": exom_slash,
            "archived": branch,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

fn tool_export(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let format = get_str(args, "format").unwrap_or("json");
    let mut exoms = state.exoms.lock().unwrap();
    let es = load_exom(state, &mut exoms, &exom_slash)?;
    let brain = &es.brain;
    let facts = brain.current_facts();

    match format {
        "jsonl" => {
            let lines: Vec<String> = facts
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "fact_id": f.fact_id,
                        "predicate": f.predicate,
                        "value": f.value,
                        "confidence": f.confidence,
                        "valid_from": f.valid_from,
                        "valid_to": f.valid_to,
                    })
                    .to_string()
                })
                .collect();
            Ok(lines.join("\n"))
        }
        _ => {
            let json_facts: Vec<serde_json::Value> = facts
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "fact_id": f.fact_id,
                        "predicate": f.predicate,
                        "value": f.value,
                        "confidence": f.confidence,
                        "valid_from": f.valid_from,
                        "valid_to": f.valid_to,
                    })
                })
                .collect();
            Ok(serde_json::json!({
                "exom": exom_slash,
                "facts": json_facts,
            })
            .to_string())
        }
    }
}
