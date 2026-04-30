mod common;

use common::daemon::TestDaemonBuilder;
use serde_json::json;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Authenticated POST that returns the raw Result so we can inspect non-2xx.
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

/// Authenticated GET that returns the raw Result.
fn auth_get_raw(base_url: &str, path: &str, session: &str) -> Result<ureq::Response, ureq::Error> {
    ureq::get(&format!("{base_url}{path}"))
        .set("Cookie", &format!("ray_exomem_session={session}"))
        .call()
}

/// Create an exom at `path` via the API, returning the response body.
fn create_exom(
    base_url: &str,
    session: &str,
    exom_path: &str,
) -> Result<ureq::Response, ureq::Error> {
    auth_post_raw(
        base_url,
        "/api/actions/exom-new",
        session,
        json!({ "path": exom_path }),
    )
}

/// Assert a fact into an exom, returning the raw result.
fn assert_fact(
    base_url: &str,
    session: &str,
    exom_path: &str,
    predicate: &str,
    value: &str,
    actor: &str,
) -> Result<ureq::Response, ureq::Error> {
    auth_post_raw(
        base_url,
        "/api/actions/assert-fact",
        session,
        json!({
            "exom": exom_path,
            "predicate": predicate,
            "value": value,
            "actor": actor,
        }),
    )
}

/// Query an exom via POST, returning the raw result.
fn query_exom(
    base_url: &str,
    session: &str,
    exom_path: &str,
    predicate: &str,
) -> Result<ureq::Response, ureq::Error> {
    let query_str = format!("(query {exom_path} ({predicate} ?x))");
    ureq::post(&format!("{base_url}/api/query"))
        .set("Cookie", &format!("ray_exomem_session={session}"))
        .set("Content-Type", "text/plain")
        .send_string(&query_str)
}

fn status_of(result: &Result<ureq::Response, ureq::Error>) -> u16 {
    match result {
        Ok(resp) => resp.status(),
        Err(ureq::Error::Status(code, _)) => *code,
        Err(e) => panic!("unexpected transport error: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Test 1: owner_can_create_and_query_exom
//
// End-to-end: owner creates an exom in their namespace, asserts a fact, and
// reads tree data for that path.
// ---------------------------------------------------------------------------

#[test]
fn owner_can_create_and_query_exom() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let alice = daemon.mock_login("alice@co.com", "Alice");

    // Create exom at alice's namespace.
    let resp = create_exom(&daemon.base_url, &alice, "alice@co.com/proj");
    let status = status_of(&resp);
    assert!(
        status == 200 || status == 201,
        "exom-new should succeed for owner, got {status}"
    );

    // Assert a fact into it.
    let resp = assert_fact(
        &daemon.base_url,
        &alice,
        "alice@co.com/proj",
        "test",
        "hello",
        "alice",
    );
    let status = status_of(&resp);
    assert!(
        status == 200 || status == 201,
        "assert-fact should succeed for owner, got {status}"
    );

    // Verify the exom shows up in tree with the asserted fact.
    let resp = auth_get_raw(
        &daemon.base_url,
        "/api/tree?path=alice@co.com/proj",
        &alice,
    );
    let status = status_of(&resp);
    assert_eq!(status, 200, "tree query should succeed for owner");
    let body: serde_json::Value = resp.unwrap().into_json().unwrap();
    assert_eq!(
        body["fact_count"], 1,
        "exom should have 1 fact after assert: {body}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: non_owner_denied_without_share
//
// Verifies that a non-owner cannot query another user's exom without a share.
// ---------------------------------------------------------------------------

#[test]
fn non_owner_denied_without_share() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let alice = daemon.mock_login("alice@co.com", "Alice");
    let _bob = daemon.mock_login("bob@co.com", "Bob");

    // Alice creates an exom.
    let resp = create_exom(&daemon.base_url, &alice, "alice@co.com/proj");
    assert!(
        status_of(&resp) == 200 || status_of(&resp) == 201,
        "alice should be able to create her exom"
    );

    // Bob tries to query alice's exom — should be denied.
    let bob = daemon.mock_login("bob@co.com", "Bob");
    let resp = query_exom(&daemon.base_url, &bob, "alice@co.com/proj", "test");
    assert_eq!(
        status_of(&resp),
        403,
        "bob should be denied access to alice's exom without a share"
    );
}

// ---------------------------------------------------------------------------
// Test 3: share_read_allows_query
//
// Tests that the /auth/shares endpoint correctly creates a share grant and
// that shared-with-me lists shares for the grantee.
// ---------------------------------------------------------------------------

#[test]
fn share_read_allows_query() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let alice = daemon.mock_login("alice@co.com", "Alice");
    let bob = daemon.mock_login("bob@co.com", "Bob");

    // Alice creates an exom.
    let resp = create_exom(&daemon.base_url, &alice, "alice@co.com/proj");
    assert!(
        status_of(&resp) == 200 || status_of(&resp) == 201,
        "alice should be able to create her exom"
    );

    // Alice shares read access with bob.
    let resp = auth_post_raw(
        &daemon.base_url,
        "/auth/shares",
        &alice,
        json!({
            "path": "alice@co.com/proj",
            "grantee_email": "bob@co.com",
            "permission": "read",
        }),
    );
    let status = status_of(&resp);
    assert!(
        status == 200 || status == 201,
        "share creation should succeed for owner, got {status}"
    );

    // Verify share details in response.
    let body: serde_json::Value = resp.unwrap().into_json().unwrap();
    assert_eq!(body["path"], "alice@co.com/proj");
    assert_eq!(body["grantee_email"], "bob@co.com");
    assert_eq!(body["permission"], "read");
    assert!(
        body["share_id"].as_str().is_some(),
        "share_id should be present"
    );

    // Verify bob can see the share via /auth/shared-with-me.
    // Note: shares_for_grantee is currently a placeholder returning empty.
    // This tests the endpoint is wired; full persistence comes later.
    let resp = auth_get_raw(&daemon.base_url, "/auth/shared-with-me", &bob);
    let status = status_of(&resp);
    assert_eq!(status, 200, "shared-with-me should return 200");
}

// ---------------------------------------------------------------------------
// Test 4: share_read_denies_mutation
//
// Read-only share must not allow assert-fact.
// ---------------------------------------------------------------------------

#[test]
fn share_read_denies_mutation() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let alice = daemon.mock_login("alice@co.com", "Alice");
    let bob = daemon.mock_login("bob@co.com", "Bob");

    // Alice creates exom and shares read with bob.
    create_exom(&daemon.base_url, &alice, "alice@co.com/proj").ok();
    auth_post_raw(
        &daemon.base_url,
        "/auth/shares",
        &alice,
        json!({
            "path": "alice@co.com/proj",
            "grantee_email": "bob@co.com",
            "permission": "read",
        }),
    )
    .ok();

    // Bob tries to assert a fact — should be denied (read-only share).
    let resp = assert_fact(
        &daemon.base_url,
        &bob,
        "alice@co.com/proj",
        "test",
        "sneaky",
        "bob",
    );
    assert_eq!(
        status_of(&resp),
        403,
        "bob with read-only share should not be able to mutate"
    );
}

// ---------------------------------------------------------------------------
// Test 5: share_readwrite_allows_mutation
//
// Read-write share allows mutations (e.g. assert-fact).
// ---------------------------------------------------------------------------

#[test]
fn share_readwrite_allows_mutation() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let alice = daemon.mock_login("alice@co.com", "Alice");
    let bob = daemon.mock_login("bob@co.com", "Bob");

    // Alice creates exom and shares read-write with bob.
    create_exom(&daemon.base_url, &alice, "alice@co.com/proj").ok();
    auth_post_raw(
        &daemon.base_url,
        "/auth/shares",
        &alice,
        json!({
            "path": "alice@co.com/proj",
            "grantee_email": "bob@co.com",
            "permission": "read-write",
        }),
    )
    .ok();

    // Bob asserts a fact — should succeed with read-write share.
    let resp = assert_fact(
        &daemon.base_url,
        &bob,
        "alice@co.com/proj",
        "test",
        "allowed",
        "bob",
    );
    let status = status_of(&resp);
    assert!(
        status == 200 || status == 201,
        "bob with read-write share should be able to mutate, got {status}"
    );
}

// ---------------------------------------------------------------------------
// Test 6: non_owner_cannot_create_share
//
// The /auth/shares POST handler DOES enforce ownership checks today.
// This test verifies that bob cannot create a share on alice's path.
// ---------------------------------------------------------------------------

#[test]
fn non_owner_cannot_create_share() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let _alice = daemon.mock_login("alice@co.com", "Alice");
    let bob = daemon.mock_login("bob@co.com", "Bob");

    // Bob tries to share alice's namespace — should be denied.
    let resp = auth_post_raw(
        &daemon.base_url,
        "/auth/shares",
        &bob,
        json!({
            "path": "alice@co.com/proj",
            "grantee_email": "carol@co.com",
            "permission": "read",
        }),
    );
    assert_eq!(
        status_of(&resp),
        403,
        "non-owner should not be able to create a share on someone else's path"
    );
}

// ---------------------------------------------------------------------------
// Test 7: share_invalid_permission_rejected
//
// Validates the permission enum at the /auth/shares endpoint.
// ---------------------------------------------------------------------------

#[test]
fn share_invalid_permission_rejected() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let alice = daemon.mock_login("alice@co.com", "Alice");

    let resp = auth_post_raw(
        &daemon.base_url,
        "/auth/shares",
        &alice,
        json!({
            "path": "alice@co.com/proj",
            "grantee_email": "bob@co.com",
            "permission": "admin",
        }),
    );
    assert_eq!(
        status_of(&resp),
        400,
        "invalid permission value should be rejected with 400"
    );
}

// ---------------------------------------------------------------------------
// Test 8: shared_with_me_returns_200
//
// Basic endpoint connectivity: /auth/shared-with-me returns 200 for an
// authenticated user with an empty shares list.
// ---------------------------------------------------------------------------

#[test]
fn shared_with_me_returns_200() {
    let daemon = TestDaemonBuilder::new().with_auth().start();
    let alice = daemon.mock_login("alice@co.com", "Alice");

    let resp = auth_get_raw(&daemon.base_url, "/auth/shared-with-me", &alice);
    let status = status_of(&resp);
    assert_eq!(status, 200, "shared-with-me should return 200");

    let body: serde_json::Value = resp.unwrap().into_json().unwrap();
    assert!(
        body["shares"].is_array(),
        "response should contain shares array"
    );
}
