//! MCP (Model Context Protocol) JSON-RPC server.
//!
//! Exposes ray-exomem capabilities as MCP tools over a single POST /mcp
//! endpoint using the JSON-RPC 2.0 transport.

use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

use crate::auth::User;
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
        "tools/list" => Ok(handle_tools_list()),
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
    Json(resp)
}

// ---------------------------------------------------------------------------
// initialize
// ---------------------------------------------------------------------------

fn handle_initialize() -> Result<serde_json::Value, JsonRpcError> {
    Ok(serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "ray-exomem",
            "version": crate::frontend_version()
        }
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
            "description": "Assert or replace a fact in an exom",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "exom": { "type": "string", "description": "Exom name" },
                    "predicate": { "type": "string", "description": "Fact predicate" },
                    "value": { "type": "string", "description": "Fact value" },
                    "fact_id": { "type": "string", "description": "Fact ID (defaults to predicate)" }
                },
                "required": ["exom", "predicate", "value"]
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
            "name": "start_session",
            "description": "Start a new session",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_path": { "type": "string", "description": "Project path" },
                    "session_type": { "type": "string", "description": "Session type" },
                    "label": { "type": "string", "description": "Session label" }
                },
                "required": ["project_path"]
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
        "query" => tool_query(state, &arguments),
        "assert_fact" => tool_assert_fact(state, &arguments),
        "list_exoms" => tool_list_exoms(state),
        "exom_status" => tool_exom_status(state, &arguments),
        "eval" => tool_eval(state, &arguments),
        "explain" => tool_explain(state, &arguments),
        "fact_history" => tool_fact_history(state, &arguments),
        "list_branches" => tool_list_branches(state, &arguments),
        "create_branch" => tool_create_branch(state, &arguments),
        "start_session" => tool_start_session(state, &arguments),
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
                        let _ = state.engine.bind_named_db(
                            crate::storage::sym_intern(exom_slash),
                            &es.datoms,
                        );
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

    match state.engine.eval_raw(&expanded.expanded_query) {
        Ok(raw) => {
            let output = if unsafe { crate::ffi::ray_obj_type(raw.as_ptr()) }
                == crate::ffi::RAY_TABLE
            {
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
                    Err(e) => return Err(JsonRpcError { code: -32000, message: e.to_string() }),
                }
            } else {
                match state.engine.format_obj(&raw) {
                    Ok(s) => s,
                    Err(e) => return Err(JsonRpcError { code: -32000, message: e.to_string() }),
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

fn tool_assert_fact(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let predicate = require_str(args, "predicate")?;
    let value = require_str(args, "value")?;
    let fact_id = get_str(args, "fact_id")
        .unwrap_or(predicate)
        .to_string();

    let ctx = crate::context::MutationContext {
        actor: "mcp".into(),
        session: None,
        model: None,
    };

    let result = crate::server::mutate_exom(state, &exom_slash, |es| {
        es.brain.assert_fact(
            &fact_id,
            predicate,
            value,
            1.0,
            "mcp",
            None,
            None,
            &ctx,
        )
    });

    match result {
        Ok(tx_id) => Ok(serde_json::json!({
            "ok": true,
            "tx_id": tx_id,
            "fact_id": fact_id,
            "predicate": predicate,
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
        return Ok(
            serde_json::json!({ "predicate": predicate, "facts": facts }).to_string()
        );
    }

    Err(JsonRpcError {
        code: -32602,
        message: "either predicate or fact_id is required".into(),
    })
}

fn tool_fact_history(
    state: &AppState,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
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

fn tool_list_branches(
    state: &AppState,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
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

fn tool_create_branch(
    state: &AppState,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let branch_name = require_str(args, "branch_name")?;

    let ctx = crate::context::MutationContext {
        actor: "mcp".into(),
        session: None,
        model: None,
    };

    let result = crate::server::mutate_exom(state, &exom_slash, |es| {
        es.brain
            .create_branch(branch_name, branch_name, &ctx)
    });

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

fn tool_start_session(
    _state: &AppState,
    args: &serde_json::Value,
) -> Result<String, JsonRpcError> {
    let project_path = require_str(args, "project_path")?;
    let session_type = get_str(args, "session_type");
    let label = get_str(args, "label");

    Ok(serde_json::json!({
        "status": "stub",
        "message": "start_session via MCP not yet wired to full session lifecycle",
        "project_path": project_path,
        "session_type": session_type,
        "label": label,
    })
    .to_string())
}

fn tool_schema(state: &AppState, args: &serde_json::Value) -> Result<String, JsonRpcError> {
    let exom_slash = exom_slug(args);
    let mut exoms = state.exoms.lock().unwrap();
    let es = load_exom(state, &mut exoms, &exom_slash)?;
    let ontology =
        crate::system_schema::build_exom_ontology(&exom_slash, &es.brain, &es.rules);
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
                .map(|f| serde_json::json!({
                    "fact_id": f.fact_id,
                    "predicate": f.predicate,
                    "value": f.value,
                    "confidence": f.confidence,
                    "valid_from": f.valid_from,
                    "valid_to": f.valid_to,
                }).to_string())
                .collect();
            Ok(lines.join("\n"))
        }
        _ => {
            let json_facts: Vec<serde_json::Value> = facts
                .iter()
                .map(|f| serde_json::json!({
                    "fact_id": f.fact_id,
                    "predicate": f.predicate,
                    "value": f.value,
                    "confidence": f.confidence,
                    "valid_from": f.valid_from,
                    "valid_to": f.valid_to,
                }))
                .collect();
            Ok(serde_json::json!({
                "exom": exom_slash,
                "facts": json_facts,
            }).to_string())
        }
    }
}
