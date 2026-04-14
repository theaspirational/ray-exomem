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
    let resp = ureq::post(&format!("{base_url}/mcp"))
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
    let tools = body["result"]["tools"].as_array().expect("tools should be array");
    assert!(
        tools.len() >= 10,
        "expected at least 10 tools, got {}",
        tools.len()
    );

    // Check that key tool names are present.
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
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
        "start_session",
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

// ---------------------------------------------------------------------------
// Test 3: requires auth
// ---------------------------------------------------------------------------

#[test]
fn mcp_requires_auth() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    // POST /mcp without Bearer token should fail with 401.
    let result = ureq::post(&format!("{}/mcp", daemon.base_url))
        .send_json(json!({
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

    let body = mcp_call(
        &daemon.base_url,
        &raw_key,
        "nonexistent/method",
        json!({}),
    );

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
