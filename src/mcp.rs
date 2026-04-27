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
    _user: User,
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
        "tools/call" => handle_tool_call(&state, req.params).await,
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
                    "exom": { "type": "string", "description": "Exom name (defaults to main)" }
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
                    "actor": { "type": "string", "description": "Actor attribution. Defaults to the authenticated user's email, else 'mcp'." },
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
                    "actor": { "type": "string", "description": "Actor attribution. Defaults to authenticated user's email, else 'mcp'." },
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
                    "actor": { "type": "string", "description": "Actor attribution. Defaults to 'mcp'." },
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
                    "actor": { "type": "string", "description": "Actor attribution. Defaults to 'mcp'." },
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
                    "actor": { "type": "string", "description": "Actor attribution. Defaults to 'mcp'." },
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
                    "exom": { "type": "string", "description": "Exom name (defaults to main)" }
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
                    "branch_name": { "type": "string", "description": "New branch name" }
                },
                "required": ["exom", "branch_name"]
            }
        }),
        serde_json::json!({
            "name": "session_new",
            "description": "Create a new session exom under <project>/sessions/<id>. The orchestrator (`actor`) is implicitly added to `agents` and gets the `main` branch; remaining agents each get a pre-allocated branch named after their actor id. Returns the new session exom path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_path": { "type": "string", "description": "Project path (must already be initialised). Slash or :: form." },
                    "session_type": { "type": "string", "enum": ["single", "multi"], "description": "`multi` pre-allocates one branch per agent (and `main` for the orchestrator). `single` only creates `main`." },
                    "label": { "type": "string", "description": "Display label. Must be non-empty and free of '/', '::', and whitespace." },
                    "actor": { "type": "string", "description": "Orchestrator. Defaults to authenticated user's email." },
                    "agents": { "type": "array", "items": { "type": "string" }, "description": "Other agent ids to pre-allocate branches for. Ignored for `single` sessions." }
                },
                "required": ["project_path", "session_type", "label"]
            }
        }),
        serde_json::json!({
            "name": "session_join",
            "description": "Claim a pre-allocated branch in a multi-agent session under TOFU (first writer wins). Returns the branch name claimed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_path": { "type": "string", "description": "Full path to the session exom, e.g. `public/work/x/y/sessions/<id>`." },
                    "actor": { "type": "string", "description": "Agent claiming a branch. Defaults to authenticated user's email." }
                },
                "required": ["session_path"]
            }
        }),
        serde_json::json!({
            "name": "session_close",
            "description": "Close a session — sets `session/closed_at = now`, after which the brain rejects all writes against the session exom. History is preserved.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_path": { "type": "string", "description": "Full path to the session exom." },
                    "actor": { "type": "string", "description": "Actor attribution. Defaults to authenticated user's email, else 'mcp'." }
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
    ]
}

// ---------------------------------------------------------------------------
// tools/call dispatch
// ---------------------------------------------------------------------------

async fn handle_tool_call(
    state: &AppState,
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
        "assert_fact" => tool_assert_fact(state, &arguments).await,
        "retract_fact" => tool_retract_fact(state, &arguments).await,
        "observe" => tool_observe(state, &arguments).await,
        "believe" => tool_believe(state, &arguments).await,
        "revoke_belief" => tool_revoke_belief(state, &arguments).await,
        "list_exoms" => tool_list_exoms(state),
        "exom_status" => tool_exom_status(state, &arguments),
        "eval" => tool_eval(state, &arguments),
        "explain" => tool_explain(state, &arguments),
        "fact_history" => tool_fact_history(state, &arguments),
        "list_branches" => tool_list_branches(state, &arguments),
        "create_branch" => tool_create_branch(state, &arguments).await,
        "session_new" => tool_session_new(state, &arguments),
        "session_join" => tool_session_join(state, &arguments),
        "session_close" => tool_session_close(state, &arguments).await,
        "schema" => tool_schema(state, &arguments),
        "export" => tool_export(state, &arguments),
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

    let exoms = state.exoms.lock().unwrap();
    let expanded = crate::server::expand_query(&exoms, &state.engine, query_str, None, "mcp/query")
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: e.to_string(),
        })?;

    if let Err(e) = crate::server::bind_typed_facts_for_exom(
        &state.engine,
        &exoms,
        &expanded.exom_name,
    ) {
        return Err(JsonRpcError {
            code: -32000,
            message: format!("failed to bind typed facts: {e}"),
        });
    }

    match state.engine.eval_raw(&expanded.expanded_query) {
        Ok(raw) => {
            let output =
                if unsafe { crate::ffi::ray_obj_type(raw.as_ptr()) } == crate::ffi::RAY_TABLE {
                    match crate::storage::decode_query_table(&raw, &expanded.normalized_query) {
                        Ok(d) => {
                            let formatted = crate::storage::format_decoded_query_table(&d);
                            // Return the decoded JSON if available, otherwise the formatted string.
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
            let _ = exom_slash; // consumed above via expand_query
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
    let actor = get_str(args, "actor").unwrap_or("mcp").to_string();
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = crate::context::MutationContext {
        actor,
        session: None,
        model: None,
        user_email: None,
    };

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
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let fact_id = require_str(args, "fact_id")?.to_string();
    let actor = get_str(args, "actor").unwrap_or("mcp").to_string();
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = crate::context::MutationContext {
        actor,
        session: None,
        model: None,
        user_email: None,
    };

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
    let actor = get_str(args, "actor").unwrap_or("mcp").to_string();
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = crate::context::MutationContext {
        actor,
        session: None,
        model: None,
        user_email: None,
    };

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
    let actor = get_str(args, "actor").unwrap_or("mcp").to_string();
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = crate::context::MutationContext {
        actor,
        session: None,
        model: None,
        user_email: None,
    };

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
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let belief_id = require_str(args, "belief_id")?.to_string();
    let actor = get_str(args, "actor").unwrap_or("mcp").to_string();
    let branch = get_str(args, "branch").map(str::to_string);

    let ctx = crate::context::MutationContext {
        actor,
        session: None,
        model: None,
        user_email: None,
    };

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
    match state.engine.eval(source) {
        Ok(output) => Ok(output),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
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
    let entries: Vec<serde_json::Value> = history
        .iter()
        .map(|f| {
            serde_json::json!({
                "fact_id": f.fact_id,
                "predicate": f.predicate,
                "value": f.value,
                "confidence": f.confidence,
                "valid_from": f.valid_from,
                "valid_to": f.valid_to,
                "created_at": f.created_at,
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
            })
        })
        .collect();
    Ok(serde_json::json!({ "branches": branches }).to_string())
}

async fn tool_create_branch(
    state: &AppState,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let branch_name = require_str(args, "branch_name")?;

    let ctx = crate::context::MutationContext {
        actor: "mcp".into(),
        session: None,
        model: None,
        user_email: None,
    };

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

fn tool_session_new(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
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
    let actor = get_str(args, "actor").unwrap_or("mcp");
    let agents: Vec<String> = args
        .get("agents")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let tree_root = state.tree_root.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no tree_root configured".into(),
    })?;
    let sym_path = state.sym_path.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no sym_path configured".into(),
    })?;

    match crate::brain::session_new(
        tree_root, sym_path, &project_path, session_type, label, actor, &agents,
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

fn tool_session_join(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let session_path_str = require_str(args, "session_path")?;
    let session_path: crate::path::TreePath =
        session_path_str.parse().map_err(|e: crate::path::PathError| JsonRpcError {
            code: -32602,
            message: format!("invalid session_path: {e}"),
        })?;
    let actor = get_str(args, "actor").unwrap_or("mcp");

    let tree_root = state.tree_root.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no tree_root configured".into(),
    })?;
    let sym_path = state.sym_path.as_deref().ok_or_else(|| JsonRpcError {
        code: -32000,
        message: "daemon has no sym_path configured".into(),
    })?;

    match crate::brain::session_join(tree_root, sym_path, &session_path, actor) {
        Ok(branch) => Ok(serde_json::json!({
            "ok": true,
            "session_path": session_path.to_slash_string(),
            "actor": actor,
            "branch": branch,
        })
        .to_string()),
        Err(e) => Err(JsonRpcError {
            code: -32000,
            message: e.to_string(),
        }),
    }
}

async fn tool_session_close(
    state: &AppState,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let session_path_str = require_str(args, "session_path")?;
    let session_path: crate::path::TreePath =
        session_path_str.parse().map_err(|e: crate::path::PathError| JsonRpcError {
            code: -32602,
            message: format!("invalid session_path: {e}"),
        })?;
    let exom_slash = session_path.to_slash_string();
    let actor = get_str(args, "actor").unwrap_or("mcp").to_string();

    let ctx = crate::context::MutationContext {
        actor,
        session: None,
        model: None,
        user_email: None,
    };

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
