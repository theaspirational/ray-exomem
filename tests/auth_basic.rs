mod common;

use common::daemon::TestDaemonBuilder;
use serde_json::json;

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
    assert!(body["provider"].is_string(), "provider field missing: {body}");
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
    let resp = daemon.auth_post(
        "/auth/api-keys",
        &session,
        json!({"label": "test-key"}),
    );
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
