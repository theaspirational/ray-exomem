#![cfg(feature = "postgres")]

// Run with: TEST_DATABASE_URL=postgres://localhost:5432/ray_exomem_test cargo test --features postgres -- --ignored

use chrono::{Duration, Utc};
use ray_exomem::auth::UserRole;
use ray_exomem::db::{
    create_pool, pg_auth::PgAuthDb, AuthDb, SessionRow, ShareGrant, StoredApiKey, StoredUser,
};

fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost:5432/ray_exomem_test".to_string())
}

async fn connect_auth() -> PgAuthDb {
    let url = test_database_url();
    let pool = create_pool(&url)
        .await
        .expect("create_pool + migrate; ensure Postgres is running and TEST_DATABASE_URL is set");
    PgAuthDb::new(pool)
}

#[tokio::test]
#[ignore]
#[cfg(feature = "postgres")]
async fn pg_auth_user_round_trip() {
    let db = connect_auth().await;
    let suf = uuid::Uuid::new_v4();
    let email = format!("pgtest.user.{suf}@example.com");

    let user = StoredUser {
        email: email.clone(),
        display_name: "PG Test User".to_string(),
        provider: "test".to_string(),
        role: UserRole::Regular,
        active: true,
        created_at: Utc::now().to_rfc3339(),
        last_login: None,
    };

    db.upsert_user(&user).await.expect("upsert_user");

    let got = db
        .get_user(&email)
        .await
        .expect("get_user")
        .expect("user should exist");
    assert_eq!(got.email, email);
    assert_eq!(got.display_name, "PG Test User");
    assert_eq!(got.provider, "test");
    assert_eq!(got.role, UserRole::Regular);
    assert!(got.active);

    let listed = db.list_users().await.expect("list_users");
    assert!(
        listed.iter().any(|u| u.email == email),
        "list_users should include new user"
    );

    db.set_role(&email, UserRole::Admin)
        .await
        .expect("set_role");

    let admin = db
        .get_user(&email)
        .await
        .expect("get_user after set_role")
        .expect("user still exists");
    assert_eq!(admin.role, UserRole::Admin);
}

#[tokio::test]
#[ignore]
#[cfg(feature = "postgres")]
async fn pg_auth_session_persistence() {
    let db = connect_auth().await;
    let suf = uuid::Uuid::new_v4();
    let email = format!("pgtest.session.{suf}@example.com");

    db.upsert_user(&StoredUser {
        email: email.clone(),
        display_name: "Session User".to_string(),
        provider: "test".to_string(),
        role: UserRole::Regular,
        active: true,
        created_at: Utc::now().to_rfc3339(),
        last_login: None,
    })
    .await
    .expect("upsert_user");

    let sid = format!("sess-{suf}");
    let expires = (Utc::now() + Duration::hours(1)).to_rfc3339();
    let session = SessionRow {
        session_id: sid.clone(),
        email: email.clone(),
        created_at: Utc::now().to_rfc3339(),
        expires_at: expires,
    };

    db.create_session(&session).await.expect("create_session");

    let got = db
        .get_session(&sid)
        .await
        .expect("get_session")
        .expect("active session");
    assert_eq!(got.session_id, sid);
    assert_eq!(got.email, email);

    db.delete_session(&sid).await.expect("delete_session");
    assert!(
        db.get_session(&sid).await.expect("get_session").is_none(),
        "session should be gone"
    );

    // Expired row: still inserted, not returned by get_session, removed by cleanup
    let expired_sid = format!("sess-expired-{suf}");
    let past = (Utc::now() - Duration::hours(2)).to_rfc3339();
    db.create_session(&SessionRow {
        session_id: expired_sid.clone(),
        email: email.clone(),
        created_at: Utc::now().to_rfc3339(),
        expires_at: past,
    })
    .await
    .expect("create_session expired");

    assert!(
        db.get_session(&expired_sid)
            .await
            .expect("get_session")
            .is_none(),
        "expired session invisible to get_session"
    );

    let n = db
        .cleanup_expired_sessions()
        .await
        .expect("cleanup_expired_sessions");
    assert!(n >= 1, "cleanup should delete at least the expired session");
}

#[tokio::test]
#[ignore]
#[cfg(feature = "postgres")]
async fn pg_auth_api_key_round_trip() {
    let db = connect_auth().await;
    let suf = uuid::Uuid::new_v4();
    let email = format!("pgtest.apikey.{suf}@example.com");

    db.upsert_user(&StoredUser {
        email: email.clone(),
        display_name: "API Key User".to_string(),
        provider: "test".to_string(),
        role: UserRole::Regular,
        active: true,
        created_at: Utc::now().to_rfc3339(),
        last_login: None,
    })
    .await
    .expect("upsert_user");

    let key_hash = format!("hash-{suf}");
    let key = StoredApiKey {
        key_id: format!("keyid-{suf}"),
        key_hash: key_hash.clone(),
        email: email.clone(),
        label: "integration".to_string(),
        created_at: Utc::now().to_rfc3339(),
    };

    db.store_api_key(&key).await.expect("store_api_key");

    let by_hash = db
        .get_api_key_by_hash(&key_hash)
        .await
        .expect("get_api_key_by_hash")
        .expect("key present");
    assert_eq!(by_hash.key_id, key.key_id);
    assert_eq!(by_hash.email, email);
    assert_eq!(by_hash.user.email, email);

    let listed = db.list_api_keys().await.expect("list_api_keys");
    assert!(
        listed.iter().any(|k| k.key_id == key.key_id),
        "list_api_keys should include key"
    );

    let for_user = db
        .list_api_keys_for_user(&email)
        .await
        .expect("list_api_keys_for_user");
    assert!(
        for_user.iter().any(|k| k.key_id == key.key_id),
        "list_api_keys_for_user should include key"
    );

    assert!(
        db.revoke_api_key(&key.key_id)
            .await
            .expect("revoke_api_key"),
        "revoke should delete row"
    );
    assert!(
        db.get_api_key_by_hash(&key_hash)
            .await
            .expect("get_api_key_by_hash")
            .is_none(),
        "key gone after revoke"
    );
}

#[tokio::test]
#[ignore]
#[cfg(feature = "postgres")]
async fn pg_auth_share_round_trip() {
    let db = connect_auth().await;
    let suf = uuid::Uuid::new_v4();
    let owner = format!("pgtest.owner.{suf}@example.com");
    let grantee = format!("pgtest.grantee.{suf}@example.com");

    for (em, name) in [(owner.as_str(), "Owner"), (grantee.as_str(), "Grantee")] {
        db.upsert_user(&StoredUser {
            email: em.to_string(),
            display_name: name.to_string(),
            provider: "test".to_string(),
            role: UserRole::Regular,
            active: true,
            created_at: Utc::now().to_rfc3339(),
            last_login: None,
        })
        .await
        .expect("upsert_user");
    }

    let share_id = format!("share-{suf}");
    let path = format!("/exom/prefix-{suf}/item");
    let grant = ShareGrant {
        share_id: share_id.clone(),
        owner_email: owner.clone(),
        path: path.clone(),
        grantee_email: grantee.clone(),
        permission: "read".to_string(),
        created_at: Utc::now().to_rfc3339(),
    };

    db.add_share(&grant).await.expect("add_share");

    let by_grantee = db
        .shares_for_grantee(&grantee)
        .await
        .expect("shares_for_grantee");
    assert!(
        by_grantee.iter().any(|g| g.share_id == share_id),
        "grantee should see share"
    );

    let by_owner = db.shares_for_owner(&owner).await.expect("shares_for_owner");
    assert!(
        by_owner.iter().any(|g| g.share_id == share_id),
        "owner should see share"
    );

    let old_prefix = format!("/exom/prefix-{suf}");
    let new_prefix = format!("/exom/moved-{suf}");
    let n = db
        .update_share_paths(&old_prefix, &new_prefix)
        .await
        .expect("update_share_paths");
    assert_eq!(n, 1, "one path should be rewritten");

    let updated = db
        .shares_for_owner(&owner)
        .await
        .expect("shares_for_owner after move");
    let g = updated
        .iter()
        .find(|x| x.share_id == share_id)
        .expect("share still exists");
    assert_eq!(g.path, format!("{new_prefix}/item"));

    assert!(
        db.revoke_share(&share_id).await.expect("revoke_share"),
        "revoke should delete"
    );
    assert!(
        db.shares_for_grantee(&grantee)
            .await
            .expect("shares_for_grantee")
            .iter()
            .all(|g| g.share_id != share_id),
        "share gone for grantee"
    );
}
