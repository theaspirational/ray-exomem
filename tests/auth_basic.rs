mod common;

use common::daemon::TestDaemonBuilder;
use serde_json::json;

fn auth_get_raw(base_url: &str, path: &str, session: &str) -> Result<ureq::Response, ureq::Error> {
    ureq::get(&format!("{base_url}{path}"))
        .set("Cookie", &format!("ray_exomem_session={session}"))
        .call()
}

fn auth_post_raw(
    base_url: &str,
    path: &str,
    session: &str,
    body: serde_json::Value,
) -> Result<ureq::Response, ureq::Error> {
    ureq::post(&format!("{base_url}{path}"))
        .set("Cookie", &format!("ray_exomem_session={session}"))
        .send_json(body)
}

fn encode_query_value(value: &str) -> String {
    value.replace('@', "%40").replace('/', "%2F")
}

fn export_json(base_url: &str, session: &str, exom: &str) -> serde_json::Value {
    auth_get_raw(
        base_url,
        &format!(
            "/ray-exomem/api/actions/export-json?exom={}",
            encode_query_value(exom)
        ),
        session,
    )
    .expect("export-json should succeed")
    .into_json()
    .expect("export-json body should be json")
}

fn schema_relation_values(
    base_url: &str,
    session: &str,
    exom: &str,
    relation: &str,
) -> Vec<String> {
    let resp = ureq::get(&format!(
        "{base_url}/ray-exomem/api/schema?include_samples=true&sample_limit=20&exom={}&relation={}",
        encode_query_value(exom),
        encode_query_value(relation),
    ))
    .set("Cookie", &format!("ray_exomem_session={session}"))
    .call()
    .expect("schema relation request should succeed");
    let body: serde_json::Value = resp.into_json().expect("schema body should be json");
    body["relations"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|relation_row| relation_row["name"] == relation)
        .flat_map(|relation_row| {
            relation_row["sample_tuples"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|row| {
            row.get(0)
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .collect()
}

fn expand_query(base_url: &str, session: &str, query: &str) -> serde_json::Value {
    ureq::post(&format!("{base_url}/ray-exomem/api/expand-query"))
        .set("Cookie", &format!("ray_exomem_session={session}"))
        .set("Content-Type", "text/plain")
        .send_string(query)
        .expect("expand-query should succeed")
        .into_json()
        .expect("expand-query body should be json")
}

fn assert_fact_with_id(
    base_url: &str,
    session: &str,
    exom: &str,
    fact_id: &str,
    predicate: &str,
    value: &str,
) {
    let resp = auth_post_raw(
        base_url,
        "/ray-exomem/api/actions/assert-fact",
        session,
        json!({
            "exom": exom,
            "fact_id": fact_id,
            "predicate": predicate,
            "value": value,
        }),
    )
    .expect("assert-fact should succeed");
    assert_eq!(resp.status(), 200);
}

#[allow(dead_code)]
fn assert_fact_with_id_i64(
    base_url: &str,
    session: &str,
    exom: &str,
    fact_id: &str,
    predicate: &str,
    value: i64,
) {
    let resp = auth_post_raw(
        base_url,
        "/ray-exomem/api/actions/assert-fact",
        session,
        json!({
            "exom": exom,
            "fact_id": fact_id,
            "predicate": predicate,
            "value": value,
        }),
    )
    .expect("assert-fact should succeed");
    assert_eq!(resp.status(), 200);
}

fn schema_with_samples(base_url: &str, session: &str, exom: &str) -> serde_json::Value {
    auth_get_raw(
        base_url,
        &format!(
            "/ray-exomem/api/schema?include_samples=true&sample_limit=20&exom={}",
            encode_query_value(exom)
        ),
        session,
    )
    .expect("schema should succeed")
    .into_json()
    .expect("schema body should be json")
}

fn find_relation<'a>(schema: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    schema["relations"]
        .as_array()
        .expect("relations should be an array")
        .iter()
        .find(|relation| relation["name"] == name)
        .unwrap_or_else(|| panic!("missing relation {name} in schema: {schema}"))
}

// ---------------------------------------------------------------------------
// Test 1: login_and_me
// ---------------------------------------------------------------------------

#[test]
fn login_and_me() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let session = daemon.mock_login("alice@co.com", "Alice");

    let resp = daemon.auth_get("/auth/me", &session);
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.into_json().unwrap();
    assert_eq!(body["email"], "alice@co.com");
    assert_eq!(body["display_name"], "Alice");
    assert!(
        body["provider"].is_string(),
        "provider field missing: {body}"
    );
    assert!(body["role"].is_string(), "role field missing: {body}");
}

// ---------------------------------------------------------------------------
// Test 2: logout_invalidates_session
// ---------------------------------------------------------------------------

#[test]
fn logout_invalidates_session() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let session = daemon.mock_login("alice@co.com", "Alice");

    // Session works before logout.
    let resp = daemon.auth_get("/auth/me", &session);
    assert_eq!(resp.status(), 200);

    // Logout.
    daemon.auth_post("/auth/logout", &session, json!({}));

    // Old session should now be rejected with 401.
    let url = format!("{}/auth/me", daemon.base_url);
    match ureq::get(&url)
        .set("Cookie", &format!("ray_exomem_session={session}"))
        .call()
    {
        Err(ureq::Error::Status(401, _)) => { /* expected */ }
        Ok(r) => panic!("expected 401 after logout, got {}", r.status()),
        Err(e) => panic!("unexpected error after logout: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Test 3: unauthenticated_me_returns_401
// ---------------------------------------------------------------------------

#[test]
fn unauthenticated_me_returns_401() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    let url = format!("{}/auth/me", daemon.base_url);
    match ureq::get(&url).call() {
        Err(ureq::Error::Status(401, _)) => { /* expected */ }
        Ok(r) => panic!("expected 401 without auth, got {}", r.status()),
        Err(e) => panic!("unexpected error: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Test 4: api_key_create_and_use
// ---------------------------------------------------------------------------

#[test]
fn api_key_create_and_use() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let session = daemon.mock_login("alice@co.com", "Alice");

    // Create an API key.
    let resp = daemon.auth_post("/auth/api-keys", &session, json!({"label": "test-key"}));
    assert!(
        resp.status() == 200 || resp.status() == 201,
        "expected 200 or 201 for api-key creation, got {}",
        resp.status()
    );

    let body: serde_json::Value = resp.into_json().unwrap();
    let key_id = body["key_id"].as_str().expect("key_id missing");
    let raw_key = body["raw_key"].as_str().expect("raw_key missing");
    assert!(!key_id.is_empty(), "key_id should not be empty");
    assert!(!raw_key.is_empty(), "raw_key should not be empty");
    assert!(
        body["mcp_config_snippet"].is_object(),
        "mcp_config_snippet missing: {body}"
    );
    assert!(
        body["mcp_config_snippet"]["mcpServers"]["ray-exomem"]["url"]
            .as_str()
            .is_some_and(|url| url.ends_with("/mcp")),
        "mcp_config_snippet should point at /mcp: {body}"
    );
    assert_eq!(
        body["mcp_config_snippet"]["mcpServers"]["ray-exomem"]["headers"]["Authorization"],
        format!("Bearer {raw_key}")
    );

    // Use the raw key as Bearer token to access /auth/me.
    let url = format!("{}/auth/me", daemon.base_url);
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {raw_key}"))
        .call()
        .expect("bearer auth should succeed");
    assert_eq!(resp.status(), 200);

    let me: serde_json::Value = resp.into_json().unwrap();
    assert_eq!(me["email"], "alice@co.com");
}

// ---------------------------------------------------------------------------
// Test 5: first_user_is_top_admin
// ---------------------------------------------------------------------------

#[test]
fn first_user_is_top_admin() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let session = daemon.mock_login("admin@co.com", "Admin");

    let resp = daemon.auth_get("/auth/me", &session);
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.into_json().unwrap();
    assert_eq!(
        body["role"], "top-admin",
        "first user should be top-admin, got: {}",
        body["role"]
    );
}

// ---------------------------------------------------------------------------
// Test 6: second_user_is_regular
// ---------------------------------------------------------------------------

#[test]
fn second_user_is_regular() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    // First user becomes top-admin.
    let _admin_session = daemon.mock_login("admin@co.com", "Admin");

    // Second user should be regular.
    let bob_session = daemon.mock_login("bob@co.com", "Bob");
    let resp = daemon.auth_get("/auth/me", &bob_session);
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.into_json().unwrap();
    assert_eq!(
        body["role"], "regular",
        "second user should be regular, got: {}",
        body["role"]
    );
}

// ---------------------------------------------------------------------------
// Test 7: login_bootstraps_user_namespace_and_owned_status
// ---------------------------------------------------------------------------

#[test]
fn login_bootstraps_user_namespace_and_owned_status() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    let _admin_session = daemon.mock_login("admin@co.com", "Admin");
    let alice_session = daemon.mock_login("alice@co.com", "Alice");

    assert!(
        daemon
            .tree_root()
            .join("alice@co.com/main/exom.json")
            .exists(),
        "user brain dashboard should be scaffolded on login"
    );
    assert!(
        daemon
            .tree_root()
            .join("alice@co.com/work/main/exom.json")
            .exists(),
        "work/main should be scaffolded on login"
    );
    assert!(
        daemon
            .tree_root()
            .join("alice@co.com/work/platform/memory-daemon/main/exom.json")
            .exists(),
        "memory-daemon project should be scaffolded on login"
    );
    assert!(
        daemon
            .tree_root()
            .join("alice@co.com/work/platform/native-ui/main/exom.json")
            .exists(),
        "native-ui project should be scaffolded on login"
    );

    let owned_status = auth_get_raw(
        &daemon.base_url,
        "/ray-exomem/api/status?exom=alice%40co.com%2Fmain",
        &alice_session,
    )
    .expect("owner should access status for their brain dashboard");
    assert_eq!(owned_status.status(), 200);
    let body: serde_json::Value = owned_status.into_json().unwrap();
    assert_eq!(body["exom"], "alice@co.com/main");

    match auth_get_raw(
        &daemon.base_url,
        "/ray-exomem/api/status?exom=main",
        &alice_session,
    ) {
        Err(ureq::Error::Status(403, _)) => {}
        Ok(resp) => panic!("expected 403 for legacy bare main, got {}", resp.status()),
        Err(err) => panic!("unexpected transport error: {err}"),
    }
}

#[test]
fn login_bootstrap_is_idempotent_and_preserves_existing_content() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    let _admin_session = daemon.mock_login("admin@co.com", "Admin");
    let alice_session = daemon.mock_login("alice@co.com", "Alice");

    let dashboard_before = export_json(&daemon.base_url, &alice_session, "alice@co.com/main");
    assert!(
        dashboard_before["facts"].as_array().unwrap().len() >= 80,
        "dashboard seed should be rich enough to exercise native facts/graph/history"
    );
    assert_eq!(
        dashboard_before["observations"].as_array().unwrap().len(),
        7
    );
    assert_eq!(dashboard_before["beliefs"].as_array().unwrap().len(), 6);
    assert_eq!(dashboard_before["branches"].as_array().unwrap().len(), 4);
    assert_eq!(dashboard_before["rules"].as_array().unwrap().len(), 6);

    assert_fact_with_id(
        &daemon.base_url,
        &alice_session,
        "alice@co.com/main",
        "workspace/custom_note",
        "workspace/custom_note",
        "preserve me",
    );
    let after_custom = export_json(&daemon.base_url, &alice_session, "alice@co.com/main");
    assert_eq!(
        after_custom["facts"].as_array().unwrap().len(),
        dashboard_before["facts"].as_array().unwrap().len() + 1
    );

    let alice_second_session = daemon.mock_login("alice@co.com", "Alice");
    let dashboard_after = export_json(&daemon.base_url, &alice_second_session, "alice@co.com/main");
    assert_eq!(
        dashboard_after["facts"].as_array().unwrap().len(),
        after_custom["facts"].as_array().unwrap().len()
    );
    assert_eq!(
        dashboard_after["rules"].as_array().unwrap().len(),
        after_custom["rules"].as_array().unwrap().len()
    );
    assert!(dashboard_after["facts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|fact| {
            fact["fact_id"] == "workspace/custom_note" && fact["value"] == "preserve me"
        }));
}

#[test]
fn brain_bootstrap_derivations_identify_operational_memory() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    let _admin_session = daemon.mock_login("admin@co.com", "Admin");
    let alice_session = daemon.mock_login("alice@co.com", "Alice");
    let dashboard_exom = "alice@co.com/main";
    let expanded = expand_query(
        &daemon.base_url,
        &alice_session,
        "(query alice@co.com/main (find ?id) (where (high_priority ?id)))",
    );
    assert!(
        expanded["expanded_query"]
            .as_str()
            .unwrap()
            .contains(r#"(facts_i64 ?id 'project/priority ?p)"#),
        "expanded query should inline the project priority rule body"
    );

    let high_priority = schema_relation_values(
        &daemon.base_url,
        &alice_session,
        dashboard_exom,
        "high_priority",
    );
    assert!(
        high_priority.contains(&"project/ray-exomem#priority".to_string()),
        "ray-exomem should derive as high priority: {high_priority:?}"
    );
    assert!(
        high_priority.contains(&"project/native-ui#priority".to_string()),
        "native UI should derive as high priority: {high_priority:?}"
    );

    assert!(
        schema_relation_values(
            &daemon.base_url,
            &alice_session,
            dashboard_exom,
            "stale_open_question",
        )
        .contains(&"question/branch-merge#age".to_string()),
        "old open branch question should derive as stale"
    );
    assert!(
        schema_relation_values(
            &daemon.base_url,
            &alice_session,
            dashboard_exom,
            "decision_review_due",
        )
        .contains(&"decision/valid-time#review".to_string()),
        "valid-time decision should derive as due for review"
    );
}

#[test]
fn brain_schema_and_graph_samples_include_native_seed_layers() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    let _admin_session = daemon.mock_login("admin@co.com", "Admin");
    let alice_session = daemon.mock_login("alice@co.com", "Alice");
    let exom = "alice@co.com/main";
    let schema = schema_with_samples(&daemon.base_url, &alice_session, exom);

    assert!(
        find_relation(&schema, "project/priority")["cardinality"]
            .as_u64()
            .unwrap()
            >= 5
    );
    assert!(
        find_relation(&schema, "observation")["cardinality"]
            .as_u64()
            .unwrap()
            >= 7
    );
    assert!(
        find_relation(&schema, "belief")["cardinality"]
            .as_u64()
            .unwrap()
            >= 5
    );

    let high_priority = find_relation(&schema, "high_priority")["sample_tuples"]
        .as_array()
        .unwrap();
    assert!(
        high_priority
            .iter()
            .any(|row| row.get(0).and_then(|v| v.as_str()) == Some("project/ray-exomem#priority")),
        "schema should sample derived high-priority rows: {schema}"
    );

    let graph_resp = auth_get_raw(
        &daemon.base_url,
        "/ray-exomem/api/relation-graph?exom=alice%40co.com%2Fmain",
        &alice_session,
    )
    .expect("relation graph should succeed");
    assert_eq!(graph_resp.status(), 200);
    let graph: serde_json::Value = graph_resp.into_json().unwrap();
    assert!(graph["summary"]["node_count"].as_u64().unwrap() > 30);
    assert!(graph["summary"]["edge_count"].as_u64().unwrap() > 70);
    assert!(graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| { node["id"] == "project/ray-exomem" }));
    assert!(graph["edges"].as_array().unwrap().iter().any(|edge| {
        edge["source"] == "project/ray-exomem" && edge["predicate"] == "project/status"
    }));
}
