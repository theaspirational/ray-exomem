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

fn schema_relation_values(base_url: &str, session: &str, exom: &str, relation: &str) -> Vec<String> {
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
        .filter_map(|row| row.get(0).and_then(|value| value.as_str()).map(str::to_string))
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
            .join("alice@co.com/personal/health/main/exom.json")
            .exists(),
        "health project should be scaffolded on login"
    );
    assert!(
        daemon.tree_root().join("alice@co.com/work/main/exom.json").exists(),
        "work/main should be scaffolded on login"
    );
    assert!(
        daemon
            .tree_root()
            .join("alice@co.com/work/example/main/exom.json")
            .exists(),
        "work/example/main should be scaffolded on login"
    );

    let owned_status = auth_get_raw(
        &daemon.base_url,
        "/ray-exomem/api/status?exom=alice%40co.com%2Fwork%2Fmain",
        &alice_session,
    )
    .expect("owner should access status for their work/main");
    assert_eq!(owned_status.status(), 200);
    let body: serde_json::Value = owned_status.into_json().unwrap();
    assert_eq!(body["exom"], "alice@co.com/work/main");

    match auth_get_raw(&daemon.base_url, "/ray-exomem/api/status?exom=main", &alice_session) {
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

    let health_before = export_json(
        &daemon.base_url,
        &alice_session,
        "alice@co.com/personal/health/main",
    );
    assert_eq!(health_before["facts"].as_array().unwrap().len(), 6);
    // Health bootstrap rules were removed in Task T2 — they will return
    // once the auxiliary binding for the water-band / step-band
    // derivations lands (see auth/routes.rs::health_bootstrap_rules).
    assert!(health_before["rules"].as_array().unwrap().is_empty());

    assert_fact_with_id(
        &daemon.base_url,
        &alice_session,
        "alice@co.com/work/main",
        "workspace/custom_note",
        "workspace/custom_note",
        "preserve me",
    );
    let work_before = export_json(&daemon.base_url, &alice_session, "alice@co.com/work/main");
    assert_eq!(work_before["facts"].as_array().unwrap().len(), 4);
    assert!(work_before["rules"].as_array().unwrap().is_empty());

    let alice_second_session = daemon.mock_login("alice@co.com", "Alice");
    let work_after = export_json(
        &daemon.base_url,
        &alice_second_session,
        "alice@co.com/work/main",
    );
    assert_eq!(
        work_after["facts"].as_array().unwrap().len(),
        work_before["facts"].as_array().unwrap().len()
    );
    assert_eq!(
        work_after["rules"].as_array().unwrap().len(),
        work_before["rules"].as_array().unwrap().len()
    );
    assert!(work_after["facts"].as_array().unwrap().iter().any(|fact| {
        fact["fact_id"] == "workspace/custom_note" && fact["value"] == "preserve me"
    }));

    let health_after = export_json(
        &daemon.base_url,
        &alice_second_session,
        "alice@co.com/personal/health/main",
    );
    assert_eq!(
        health_after["facts"].as_array().unwrap().len(),
        health_before["facts"].as_array().unwrap().len()
    );
    assert_eq!(
        health_after["rules"].as_array().unwrap().len(),
        health_before["rules"].as_array().unwrap().len()
    );
}

// FIXME(phase-b-typed-cmp): native_derived_relations is currently disabled
// at the Rayfall-rule layer (see system_schema.rs). Rules with a string
// literal in the head position started tripping rayforce2's type inference
// (`error:type`) on any subsequent query once the FactValue refactor moved
// numeric values onto their own tag. Re-enable after Phase B reintroduces
// native derivations via a Rust-computed read path (or a typed relation).
#[test]
#[ignore]
fn health_bootstrap_derivations_follow_thresholds() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    let _admin_session = daemon.mock_login("admin@co.com", "Admin");
    let alice_session = daemon.mock_login("alice@co.com", "Alice");
    let health_exom = "alice@co.com/personal/health/main";
    let expanded = expand_query(
        &daemon.base_url,
        &alice_session,
        "(query alice@co.com/personal/health/main (find ?value) (where (health/recommended-water-ml ?value)))",
    );
    assert!(
        expanded["expanded_query"]
            .as_str()
            .unwrap()
            .contains(r#"(health/water-band "medium")"#)
    );

    assert_eq!(
        schema_relation_values(
            &daemon.base_url,
            &alice_session,
            health_exom,
            "health/recommended-water-ml",
        ),
        vec!["2500".to_string()]
    );
    assert_eq!(
        schema_relation_values(
            &daemon.base_url,
            &alice_session,
            health_exom,
            "health/recommended-steps-per-day",
        ),
        vec!["9000".to_string()]
    );

    assert_fact_with_id(
        &daemon.base_url,
        &alice_session,
        health_exom,
        "health/profile/age",
        "profile/age",
        "29",
    );
    assert_eq!(
        schema_relation_values(
            &daemon.base_url,
            &alice_session,
            health_exom,
            "health/recommended-steps-per-day",
        ),
        vec!["10000".to_string()]
    );

    assert_fact_with_id(
        &daemon.base_url,
        &alice_session,
        health_exom,
        "health/profile/age",
        "profile/age",
        "50",
    );
    assert_eq!(
        schema_relation_values(
            &daemon.base_url,
            &alice_session,
            health_exom,
            "health/recommended-steps-per-day",
        ),
        vec!["7500".to_string()]
    );

    assert_fact_with_id(
        &daemon.base_url,
        &alice_session,
        health_exom,
        "health/profile/height_cm",
        "profile/height_cm",
        "169",
    );
    assert_fact_with_id(
        &daemon.base_url,
        &alice_session,
        health_exom,
        "health/profile/weight_kg",
        "profile/weight_kg",
        "59",
    );
    assert_eq!(
        schema_relation_values(
            &daemon.base_url,
            &alice_session,
            health_exom,
            "health/recommended-water-ml",
        ),
        vec!["2000".to_string()]
    );

    assert_fact_with_id(
        &daemon.base_url,
        &alice_session,
        health_exom,
        "health/profile/weight_kg",
        "profile/weight_kg",
        "85",
    );
    assert_eq!(
        schema_relation_values(
            &daemon.base_url,
            &alice_session,
            health_exom,
            "health/recommended-water-ml",
        ),
        vec!["3000".to_string()]
    );
}

// FIXME(phase-b-typed-cmp): see comment on `health_bootstrap_derivations_follow_thresholds`.
#[test]
#[ignore]
fn health_schema_samples_include_native_helpers_and_recommendations() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    let _admin_session = daemon.mock_login("admin@co.com", "Admin");
    let alice_session = daemon.mock_login("alice@co.com", "Alice");
    let schema = schema_with_samples(
        &daemon.base_url,
        &alice_session,
        "alice@co.com/personal/health/main",
    );

    assert_eq!(
        find_relation(&schema, "health/water-band")["sample_tuples"],
        json!([["medium"]])
    );
    assert_eq!(
        find_relation(&schema, "health/step-band")["sample_tuples"],
        json!([["medium"]])
    );
    assert_eq!(
        find_relation(&schema, "health/recommended-water-ml")["sample_tuples"],
        json!([["2500"]])
    );
    assert_eq!(
        find_relation(&schema, "health/recommended-steps-per-day")["sample_tuples"],
        json!([["9000"]])
    );
    assert!(
        schema["ontology"]["user_predicates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "health/water-band")
    );
    assert!(
        schema["ontology"]["user_predicates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "health/step-band")
    );
}
