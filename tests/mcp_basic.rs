mod common;

use common::daemon::TestDaemonBuilder;
use serde_json::json;

// ---------------------------------------------------------------------------
// Helper: create a daemon with auth, login, create API key, return (daemon, raw_key)
// ---------------------------------------------------------------------------

fn daemon_with_api_key() -> (common::daemon::TestDaemon, String) {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let session = daemon.mock_login("alice@co.com", "Alice");

    let key_resp: serde_json::Value = daemon
        .auth_post("/auth/api-keys", &session, json!({"label": "mcp-test"}))
        .into_json()
        .unwrap();
    let raw_key = key_resp["raw_key"].as_str().unwrap().to_string();

    (daemon, raw_key)
}

fn mcp_call(
    base_url: &str,
    raw_key: &str,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    mcp_call_at(base_url, "/mcp", raw_key, method, params)
}

fn mcp_call_at(
    base_url: &str,
    path: &str,
    raw_key: &str,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let resp = ureq::post(&format!("{base_url}{path}"))
        .set("Authorization", &format!("Bearer {raw_key}"))
        .send_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .unwrap();
    resp.into_json().unwrap()
}

fn mcp_call_no_auth(
    base_url: &str,
    path: &str,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let resp = ureq::post(&format!("{base_url}{path}"))
        .send_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .unwrap();
    resp.into_json().unwrap()
}

// ---------------------------------------------------------------------------
// Test 1: initialize
// ---------------------------------------------------------------------------

#[test]
fn mcp_initialize() {
    let (daemon, raw_key) = daemon_with_api_key();

    let body = mcp_call(&daemon.base_url, &raw_key, "initialize", json!({}));

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);
    assert_eq!(body["result"]["serverInfo"]["name"], "ray-exomem");
    assert!(
        body["result"]["protocolVersion"].is_string(),
        "protocolVersion missing: {body}"
    );
    assert!(
        body["result"]["capabilities"]["tools"].is_object(),
        "capabilities.tools missing: {body}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: tools/list
// ---------------------------------------------------------------------------

#[test]
fn mcp_tools_list() {
    let (daemon, raw_key) = daemon_with_api_key();

    let body = mcp_call(&daemon.base_url, &raw_key, "tools/list", json!({}));

    assert!(body["error"].is_null(), "unexpected error: {body}");
    let tools = body["result"]["tools"]
        .as_array()
        .expect("tools should be array");
    assert!(
        tools.len() >= 10,
        "expected at least 10 tools, got {}",
        tools.len()
    );

    // Check that key tool names are present.
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in &[
        "query",
        "assert_fact",
        "list_exoms",
        "exom_status",
        "eval",
        "explain",
        "fact_history",
        "list_branches",
        "create_branch",
        "session_new",
        "schema",
        "export",
    ] {
        assert!(
            names.contains(expected),
            "tool '{}' missing from tools/list; got: {:?}",
            expected,
            names
        );
    }

    // Check inputSchema present on each tool.
    for tool in tools {
        assert!(
            tool["inputSchema"]["type"].as_str() == Some("object"),
            "tool {} missing inputSchema.type: {}",
            tool["name"],
            tool
        );
    }
}

#[test]
fn mcp_sse_path_accepts_streamable_post() {
    let (daemon, raw_key) = daemon_with_api_key();

    let body = mcp_call_at(
        &daemon.base_url,
        "/mcp/sse",
        &raw_key,
        "initialize",
        json!({}),
    );

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);
    assert_eq!(body["result"]["serverInfo"]["name"], "ray-exomem");
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
}

#[test]
fn mcp_single_user_mode_accepts_streamable_post_without_auth() {
    let daemon = common::daemon::TestDaemon::start();

    let body = mcp_call_no_auth(&daemon.base_url, "/mcp/sse", "initialize", json!({}));

    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);
    assert_eq!(body["result"]["serverInfo"]["name"], "ray-exomem");
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
}

#[test]
fn mcp_notification_returns_accepted_without_json_rpc_body() {
    let (daemon, raw_key) = daemon_with_api_key();

    let resp = ureq::post(&format!("{}/mcp", daemon.base_url))
        .set("Authorization", &format!("Bearer {raw_key}"))
        .send_json(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }))
        .expect("notification should be accepted");

    assert_eq!(resp.status(), 202);
    let body = resp
        .into_string()
        .expect("response body should be readable");
    assert!(
        body.is_empty(),
        "notification response body should be empty"
    );
}

// ---------------------------------------------------------------------------
// Test 3: requires auth
// ---------------------------------------------------------------------------

#[test]
fn mcp_requires_auth() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    // POST /mcp without Bearer token should fail with 401.
    let result = ureq::post(&format!("{}/mcp", daemon.base_url)).send_json(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }));

    match result {
        Err(ureq::Error::Status(401, _)) => { /* expected */ }
        Ok(r) => panic!("expected 401 without auth, got {}", r.status()),
        Err(e) => panic!("unexpected error: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Test 4: unknown method returns error
// ---------------------------------------------------------------------------

#[test]
fn mcp_unknown_method() {
    let (daemon, raw_key) = daemon_with_api_key();

    let body = mcp_call(&daemon.base_url, &raw_key, "nonexistent/method", json!({}));

    assert!(body["result"].is_null(), "should have no result: {body}");
    assert_eq!(body["error"]["code"], -32601);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("method not found"),
        "error message should mention method not found: {}",
        body["error"]["message"]
    );
}

// ---------------------------------------------------------------------------
// Test 5: tools/call with unknown tool
// ---------------------------------------------------------------------------

#[test]
fn mcp_unknown_tool() {
    let (daemon, raw_key) = daemon_with_api_key();

    let body = mcp_call(
        &daemon.base_url,
        &raw_key,
        "tools/call",
        json!({
            "name": "nonexistent_tool",
            "arguments": {}
        }),
    );

    assert!(body["result"].is_null(), "should have no result: {body}");
    assert_eq!(body["error"]["code"], -32602);
}

// ---------------------------------------------------------------------------
// Test 6: tools/call list_exoms
// ---------------------------------------------------------------------------

#[test]
fn mcp_tool_list_exoms() {
    let (daemon, raw_key) = daemon_with_api_key();

    let body = mcp_call(
        &daemon.base_url,
        &raw_key,
        "tools/call",
        json!({
            "name": "list_exoms",
            "arguments": {}
        }),
    );

    assert!(body["error"].is_null(), "unexpected error: {body}");
    let content = body["result"]["content"]
        .as_array()
        .expect("content should be array");
    assert!(!content.is_empty(), "content should not be empty");
    assert_eq!(content[0]["type"], "text");
    // The text should be parseable JSON containing an "exoms" key.
    let text = content[0]["text"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(text).expect("text should be JSON");
    assert!(inner["exoms"].is_array(), "exoms key missing from: {inner}");
}

// ---------------------------------------------------------------------------
// Test 7: tools/call eval
// ---------------------------------------------------------------------------

#[test]
fn mcp_tool_eval() {
    let (daemon, raw_key) = daemon_with_api_key();

    let body = mcp_call(
        &daemon.base_url,
        &raw_key,
        "tools/call",
        json!({
            "name": "eval",
            "arguments": { "source": "(+ 1 2)" }
        }),
    );

    assert!(body["error"].is_null(), "unexpected error: {body}");
    let content = body["result"]["content"].as_array().unwrap();
    let text = content[0]["text"].as_str().unwrap();
    assert!(
        text.contains('3'),
        "eval of (+ 1 2) should contain 3, got: {text}"
    );
}

// ---------------------------------------------------------------------------
// Test 8: tools/call tree on an unmapped path returns `unknown_path:` rather
// than a generic `io: missing` (F4 — distinguish "not allocated yet" from real
// I/O failure so callers can pattern-match on the canonical substring).
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Typed-value routing via MCP assert_fact (regression for stress-test finding #3).
//
// Verify the schema-tagged value pipeline lands FactValue variants in the
// right typed EDB, and that the defensive parser recovers structured types
// from stringified inputs without misclassifying plain strings.
// ---------------------------------------------------------------------------

fn call_mcp_tool(base_url: &str, name: &str, arguments: serde_json::Value) -> serde_json::Value {
    mcp_call_no_auth(
        base_url,
        "/mcp",
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
}

fn init_test_exom(base_url: &str, path: &str) {
    let body = call_mcp_tool(base_url, "init", json!({ "path": path }));
    assert!(body["error"].is_null(), "init failed: {body}");
}

fn assert_fact_value(
    base_url: &str,
    exom: &str,
    fact_id: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    call_mcp_tool(
        base_url,
        "assert_fact",
        json!({
            "exom": exom,
            "predicate": fact_id,
            "fact_id": fact_id,
            "value": value,
        }),
    )
}

fn fact_history_value(base_url: &str, exom: &str, fact_id: &str) -> serde_json::Value {
    let body = call_mcp_tool(
        base_url,
        "fact_history",
        json!({ "exom": exom, "id": fact_id }),
    );
    let text = body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("fact_history returned no text: {body}"));
    let inner: serde_json::Value =
        serde_json::from_str(text).expect("fact_history text should be JSON");
    inner["history"][0]["value"].clone()
}

#[test]
fn tool_assert_fact_routes_json_number_to_i64() {
    let daemon = common::daemon::TestDaemon::start();
    init_test_exom(&daemon.base_url, "test/typed-i64");
    let exom = "test/typed-i64/main";

    let resp = assert_fact_value(&daemon.base_url, exom, "test/age", json!(84));
    assert!(resp["error"].is_null(), "assert_fact failed: {resp}");

    let value = fact_history_value(&daemon.base_url, exom, "test/age");
    // FactValue::I64 serializes as a JSON number under #[serde(untagged)].
    assert_eq!(value, json!(84), "JSON number must round-trip as I64");
}

#[test]
fn tool_assert_fact_recovers_stringified_sym_object() {
    let daemon = common::daemon::TestDaemon::start();
    init_test_exom(&daemon.base_url, "test/typed-sym");
    let exom = "test/typed-sym/main";

    let resp = assert_fact_value(
        &daemon.base_url,
        exom,
        "test/status",
        json!("{\"$sym\":\"active\"}"),
    );
    assert!(
        resp["error"].is_null(),
        "stringified sym must be recovered, got: {resp}"
    );

    let value = fact_history_value(&daemon.base_url, exom, "test/status");
    assert_eq!(
        value,
        json!({"$sym": "active"}),
        "recovered Sym should serialize as $sym object"
    );
}

#[test]
fn tool_assert_fact_recovers_stringified_array_rejected() {
    let daemon = common::daemon::TestDaemon::start();
    init_test_exom(&daemon.base_url, "test/typed-arr");
    let exom = "test/typed-arr/main";

    let resp = assert_fact_value(&daemon.base_url, exom, "test/list", json!("[1,2,3]"));
    let msg = resp["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("invalid 'value'"),
        "recovered array should be rejected with `invalid 'value'`, got: {resp}"
    );
}

#[test]
fn tool_assert_fact_leaves_plain_string_alone() {
    let daemon = common::daemon::TestDaemon::start();
    init_test_exom(&daemon.base_url, "test/typed-str");
    let exom = "test/typed-str/main";

    let resp = assert_fact_value(&daemon.base_url, exom, "test/label", json!("75"));
    assert!(resp["error"].is_null(), "assert_fact failed: {resp}");

    let value = fact_history_value(&daemon.base_url, exom, "test/label");
    // Plain string "75" must NOT be promoted to I64(75) — it stays Str("75").
    assert_eq!(
        value,
        json!("75"),
        "plain string must stay as Str, not be coerced to I64"
    );
}

// ---------------------------------------------------------------------------
// Hyphenated-predicate pinning round-trip (regression for the rayforce
// `parse_symbol` char-class gap). The query lowerer rewrites a string
// literal in a sym-encoded slot — `(fact-row ?f "profile/last-name" ?v)` —
// to a quoted symbol — `(fact-row ?f 'profile/last-name ?v)` — before the
// engine ever sees it. Until rayforce's `parse_symbol` learned to accept
// `-`, the engine read that as `'profile/last` followed by an undefined
// `-name` token and the query silently returned zero rows.
// ---------------------------------------------------------------------------
#[test]
fn tool_query_pins_hyphenated_predicate_literal() {
    let daemon = common::daemon::TestDaemon::start();
    init_test_exom(&daemon.base_url, "test/hyphen-pin");
    let exom = "test/hyphen-pin/main";

    let asserted = assert_fact_value(&daemon.base_url, exom, "profile/last-name", json!("Ng"));
    assert!(
        asserted["error"].is_null(),
        "assert_fact failed: {asserted}"
    );

    let query_body = call_mcp_tool(
        &daemon.base_url,
        "query",
        json!({
            "exom": exom,
            "query": format!(
                "(query {exom} (find ?f ?v) (where (fact-row ?f \"profile/last-name\" ?v)))"
            ),
        }),
    );
    let text = query_body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("query returned no text: {query_body}"));
    let inner: serde_json::Value = serde_json::from_str(text).expect("query text should be JSON");
    let rows = inner["rows"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        rows.len(),
        1,
        "hyphenated-predicate pin must match exactly one row, got rows={rows:?} from {inner}"
    );
    assert_eq!(rows[0][1], json!("Ng"));
}

#[test]
fn mcp_tool_tree_returns_unknown_path_for_unmapped_path() {
    let (daemon, raw_key) = daemon_with_api_key();

    let body = mcp_call(
        &daemon.base_url,
        &raw_key,
        "tools/call",
        json!({
            "name": "tree",
            "arguments": { "path": "public/never-existed" }
        }),
    );

    assert!(body["result"].is_null(), "should have no result: {body}");
    let msg = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("unknown_path: public/never-existed"),
        "expected unknown_path error for unmapped path, got: {msg}"
    );
}
