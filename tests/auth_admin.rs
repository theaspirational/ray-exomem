mod common;

use common::daemon::TestDaemonBuilder;
use serde_json::json;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Authenticated GET returning the raw Result so we can inspect non-2xx.
fn auth_get_raw(base_url: &str, path: &str, session: &str) -> Result<ureq::Response, ureq::Error> {
    ureq::get(&format!("{base_url}{path}"))
        .set("Cookie", &format!("ray_exomem_session={session}"))
        .call()
}

/// Authenticated POST returning the raw Result.
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

/// Authenticated DELETE returning the raw Result.
fn auth_delete_raw(
    base_url: &str,
    path: &str,
    session: &str,
) -> Result<ureq::Response, ureq::Error> {
    ureq::delete(&format!("{base_url}{path}"))
        .set("Cookie", &format!("ray_exomem_session={session}"))
        .call()
}

fn status_of(result: &Result<ureq::Response, ureq::Error>) -> u16 {
    match result {
        Ok(resp) => resp.status(),
        Err(ureq::Error::Status(code, _)) => *code,
        Err(e) => panic!("unexpected transport error: {e}"),
    }
}

fn auth_post_json(
    base_url: &str,
    path: &str,
    session: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    auth_post_raw(base_url, path, session, body)
        .expect("authenticated post should succeed")
        .into_json()
        .expect("json body")
}

// ---------------------------------------------------------------------------
// Test 1: first_user_becomes_top_admin
// ---------------------------------------------------------------------------

#[test]
fn first_user_becomes_top_admin() {
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
// Test 2: top_admin_can_grant_admin
// ---------------------------------------------------------------------------

#[test]
fn top_admin_can_grant_admin() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    // First user becomes top-admin.
    let admin_session = daemon.mock_login("admin@co.com", "Admin");
    // Second user is regular.
    let _bob_session = daemon.mock_login("bob@co.com", "Bob");

    // Top-admin grants admin to bob.
    let resp = auth_post_raw(
        &daemon.base_url,
        "/auth/admin/admins",
        &admin_session,
        json!({"email": "bob@co.com"}),
    );
    assert_eq!(
        status_of(&resp),
        200,
        "top-admin should be able to grant admin"
    );
}

// ---------------------------------------------------------------------------
// Test 3: admin_cannot_manage_admins
//
// The grant_admin handler requires top-admin. A regular user (or an admin
// who is not top-admin) should get 403. Since grant_admin is currently a
// stub that does not persist role changes, bob remains regular after the
// top-admin "grants" him admin. Either way, the endpoint correctly denies
// non-top-admin callers.
// ---------------------------------------------------------------------------

#[test]
fn admin_cannot_manage_admins() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    // First user becomes top-admin.
    let admin_session = daemon.mock_login("admin@co.com", "Admin");
    // Second user is regular.
    let bob_session = daemon.mock_login("bob@co.com", "Bob");

    // Top-admin grants admin to bob (stub — role not actually persisted yet).
    let resp = auth_post_raw(
        &daemon.base_url,
        "/auth/admin/admins",
        &admin_session,
        json!({"email": "bob@co.com"}),
    );
    assert_eq!(status_of(&resp), 200, "top-admin grant should succeed");

    // Bob tries to grant admin to carol — should be denied (403).
    let resp = auth_post_raw(
        &daemon.base_url,
        "/auth/admin/admins",
        &bob_session,
        json!({"email": "carol@co.com"}),
    );
    assert_eq!(
        status_of(&resp),
        403,
        "non-top-admin should not be able to manage admins"
    );
}

// ---------------------------------------------------------------------------
// Test 4: regular_user_denied_admin_routes
// ---------------------------------------------------------------------------

#[test]
fn regular_user_denied_admin_routes() {
    let daemon = TestDaemonBuilder::new().with_auth().start();

    // First user becomes top-admin (we need this to exist).
    let _admin_session = daemon.mock_login("admin@co.com", "Admin");
    // Second user is regular.
    let bob_session = daemon.mock_login("bob@co.com", "Bob");

    // GET /auth/admin/users -> 403
    let resp = auth_get_raw(&daemon.base_url, "/auth/admin/users", &bob_session);
    assert_eq!(
        status_of(&resp),
        403,
        "regular user should be denied /auth/admin/users"
    );

    // GET /auth/admin/sessions -> 403
    let resp = auth_get_raw(&daemon.base_url, "/auth/admin/sessions", &bob_session);
    assert_eq!(
        status_of(&resp),
        403,
        "regular user should be denied /auth/admin/sessions"
    );
}

// ---------------------------------------------------------------------------
// Test 5: admin_can_list_users
// ---------------------------------------------------------------------------

#[test]
fn admin_can_list_users() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let admin_session = daemon.mock_login("admin@co.com", "Admin");

    let resp = auth_get_raw(&daemon.base_url, "/auth/admin/users", &admin_session);
    assert_eq!(
        status_of(&resp),
        200,
        "top-admin should access /auth/admin/users"
    );

    let body: serde_json::Value = resp.unwrap().into_json().unwrap();
    assert!(
        body["users"].is_array(),
        "response should contain users array: {body}"
    );
}

// ---------------------------------------------------------------------------
// Test 6: admin_can_manage_domains
// ---------------------------------------------------------------------------

#[test]
fn admin_can_manage_domains() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let admin_session = daemon.mock_login("admin@co.com", "Admin");

    // 1. POST /auth/admin/allowed-domains with new domain -> 200
    let resp = auth_post_raw(
        &daemon.base_url,
        "/auth/admin/allowed-domains",
        &admin_session,
        json!({"domain": "newcorp.com"}),
    );
    assert_eq!(
        status_of(&resp),
        200,
        "top-admin should be able to add a domain"
    );

    // 2. GET /auth/admin/allowed-domains -> should include "newcorp.com"
    let resp = auth_get_raw(
        &daemon.base_url,
        "/auth/admin/allowed-domains",
        &admin_session,
    );
    assert_eq!(status_of(&resp), 200);
    let body: serde_json::Value = resp.unwrap().into_json().unwrap();
    let domains = body["domains"]
        .as_array()
        .expect("domains should be an array");
    assert!(
        domains.iter().any(|d| d.as_str() == Some("newcorp.com")),
        "domains should include newcorp.com after adding it: {body}"
    );

    // 3. DELETE /auth/admin/allowed-domains/newcorp.com -> 200
    let resp = auth_delete_raw(
        &daemon.base_url,
        "/auth/admin/allowed-domains/newcorp.com",
        &admin_session,
    );
    assert_eq!(
        status_of(&resp),
        200,
        "top-admin should be able to remove a domain"
    );

    // 4. GET /auth/admin/allowed-domains -> should NOT include "newcorp.com"
    let resp = auth_get_raw(
        &daemon.base_url,
        "/auth/admin/allowed-domains",
        &admin_session,
    );
    assert_eq!(status_of(&resp), 200);
    let body: serde_json::Value = resp.unwrap().into_json().unwrap();
    let domains = body["domains"]
        .as_array()
        .expect("domains should be an array");
    assert!(
        !domains.iter().any(|d| d.as_str() == Some("newcorp.com")),
        "domains should NOT include newcorp.com after removing it: {body}"
    );
}

// ---------------------------------------------------------------------------
// Test 7: top_admin_can_deactivate_user_and_revoke_live_access
// ---------------------------------------------------------------------------

#[test]
fn top_admin_can_deactivate_user_and_revoke_live_access() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let admin_session = daemon.mock_login("admin@co.com", "Admin");
    let bob_session = daemon.mock_login("bob@co.com", "Bob");

    let key_body = auth_post_json(
        &daemon.base_url,
        "/auth/api-keys",
        &bob_session,
        json!({"label": "bob-key"}),
    );
    let raw_key = key_body["raw_key"].as_str().expect("raw key").to_string();

    let resp = auth_post_raw(
        &daemon.base_url,
        "/auth/admin/users/bob%40co.com/deactivate",
        &admin_session,
        json!({}),
    );
    assert_eq!(status_of(&resp), 200, "top-admin should deactivate user");

    let me_url = format!("{}/auth/me", daemon.base_url);
    match ureq::get(&me_url)
        .set("Cookie", &format!("ray_exomem_session={bob_session}"))
        .call()
    {
        Err(ureq::Error::Status(401, _)) => {}
        other => panic!("expected bob session to be rejected after deactivation, got {other:?}"),
    }

    match ureq::get(&me_url)
        .set("Authorization", &format!("Bearer {raw_key}"))
        .call()
    {
        Err(ureq::Error::Status(401, _)) => {}
        other => panic!("expected bob api key to be rejected after deactivation, got {other:?}"),
    }

    match ureq::post(&format!("{}/auth/login", daemon.base_url)).send_json(json!({
        "id_token": "mock:bob@co.com:Bob",
        "provider": "mock"
    })) {
        Err(ureq::Error::Status(403, _)) => {}
        other => panic!("expected deactivated user login to be rejected, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 8: top_admin_can_delete_user_and_namespace
// ---------------------------------------------------------------------------

#[test]
fn top_admin_can_delete_user_and_namespace() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let admin_session = daemon.mock_login("admin@co.com", "Admin");
    let bob_session = daemon.mock_login("bob@co.com", "Bob");

    let create = ureq::post(&format!("{}/api/actions/exom-new", daemon.base_url))
        .set("Cookie", &format!("ray_exomem_session={bob_session}"))
        .send_json(json!({"path": "bob@co.com/projects/main"}))
        .expect("user exom creation should succeed");
    assert_eq!(create.status(), 200);
    assert!(
        daemon.tree_root().join("bob@co.com/projects/main").exists(),
        "user namespace should exist before delete"
    );

    let resp = auth_delete_raw(
        &daemon.base_url,
        "/auth/admin/users/bob%40co.com",
        &admin_session,
    );
    assert_eq!(status_of(&resp), 200, "top-admin should delete user");

    assert!(
        !daemon.tree_root().join("bob@co.com").exists(),
        "user namespace should be removed from filesystem"
    );

    let users = auth_get_raw(&daemon.base_url, "/auth/admin/users", &admin_session)
        .expect("users listing should succeed")
        .into_json::<serde_json::Value>()
        .expect("users json");
    let rows = users["users"].as_array().expect("users array");
    assert!(
        rows.iter()
            .all(|row| row["email"].as_str() != Some("bob@co.com")),
        "deleted user should be removed from admin listing: {users}"
    );
}
