//! AuthStore — typed read/write over _system/auth exom.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dashmap::DashMap;

use crate::auth::{User, UserRole};
use crate::exom::{self, ExomMeta, META_FILENAME};

pub struct AuthStore {
    pub exom_disk: PathBuf,
    pub jsonl_path: PathBuf,
    pub session_cache: DashMap<String, User>,
    pub api_key_cache: DashMap<String, User>,
    pub allowed_domains: Mutex<Vec<String>>,
    /// In-memory share grants until persisted to the system exom (see TODO on create_share).
    pub share_grants: Mutex<Vec<ShareGrant>>,
    // Persistent indexes (populated from JSONL replay)
    pub users: Mutex<HashMap<String, StoredUser>>,
    pub api_keys: Mutex<HashMap<String, StoredApiKey>>,
    pub api_key_by_hash: Mutex<HashMap<String, String>>,
    pub top_admin: Mutex<Option<String>>,
    pub admins: Mutex<HashSet<String>>,
}

#[derive(Debug, Clone)]
pub struct ShareGrant {
    pub share_id: String,
    pub owner_email: String,
    pub path: String,
    pub grantee_email: String,
    pub permission: String, // "read" or "read-write"
    pub created_at: String,
}

/// Persistent user record (replayed from JSONL).
#[derive(Debug, Clone)]
pub struct StoredUser {
    pub email: String,
    pub display_name: String,
    pub provider: String,
    pub created_at: String,
    pub active: bool,
}

/// Persistent API key record (replayed from JSONL).
#[derive(Debug, Clone)]
pub struct StoredApiKey {
    pub key_id: String,
    pub key_hash: String,
    pub email: String,
    pub label: String,
    pub created_at: String,
}

impl AuthStore {
    /// Creates `_system/auth` directory + `exom.json` if not present,
    /// loads existing state, seeds domains on first boot only.
    pub fn bootstrap(tree_root: &Path, seed_domains: &[String]) -> anyhow::Result<Self> {
        let exom_disk = tree_root.join("_system").join("auth");
        let meta_file = exom_disk.join(META_FILENAME);
        let jsonl_path = exom_disk.join("auth.jsonl");

        let first_boot = !meta_file.exists();
        if first_boot {
            let meta = ExomMeta::new_bare();
            exom::write_meta(&exom_disk, &meta)?;
        }

        let store = Self {
            exom_disk,
            jsonl_path,
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(Vec::new()),
            share_grants: Mutex::new(Vec::new()),
            users: Mutex::new(HashMap::new()),
            api_keys: Mutex::new(HashMap::new()),
            api_key_by_hash: Mutex::new(HashMap::new()),
            top_admin: Mutex::new(None),
            admins: Mutex::new(HashSet::new()),
        };

        // Replay persisted state.
        store.replay_jsonl()?;

        // Seed domains on first boot only (if no domains were loaded from JSONL).
        if first_boot && store.allowed_domains.lock().unwrap().is_empty() {
            for d in seed_domains {
                store.add_domain(d);
            }
        }

        // Rebuild api_key_cache from persisted keys so Bearer auth works on restart.
        store.rebuild_api_key_cache();

        Ok(store)
    }

    /// Empty list = allow all; otherwise check email domain against allowed list.
    pub fn check_domain(&self, email: &str) -> bool {
        let domains = self.allowed_domains.lock().unwrap();
        if domains.is_empty() {
            return true;
        }
        let Some(domain) = email.rsplit('@').next() else {
            return false;
        };
        domains.iter().any(|d| d == domain)
    }

    /// Lookup user from session cache.
    pub fn get_user_by_session(&self, session_id: &str) -> Option<User> {
        self.session_cache.get(session_id).map(|r| r.clone())
    }

    /// Lookup user from API key cache.
    pub fn get_user_by_key_hash(&self, key_hash: &str) -> Option<User> {
        self.api_key_cache.get(key_hash).map(|r| r.clone())
    }

    /// Returns `(key_id, raw_key)`. Does NOT store — caller handles caching.
    pub fn generate_api_key(&self, _email: &str, _label: &str) -> (String, String) {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        use rand::RngCore;

        let key_id = uuid::Uuid::new_v4().to_string();

        let mut raw_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw_bytes);
        let raw_key = URL_SAFE_NO_PAD.encode(raw_bytes);

        (key_id, raw_key)
    }

    /// SHA-256 hash, hex-encoded.
    pub fn hash_api_key(raw_key: &str) -> String {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(raw_key.as_bytes());
        hex::encode(hash)
    }

    /// Remove from session cache.
    pub fn evict_session(&self, session_id: &str) {
        self.session_cache.remove(session_id);
    }

    /// Remove from API key cache.
    pub fn evict_api_key(&self, key_hash: &str) {
        self.api_key_cache.remove(key_hash);
    }

    /// Clone the current allowed-domains list.
    pub fn list_allowed_domains(&self) -> Vec<String> {
        self.allowed_domains.lock().unwrap().clone()
    }

    /// Resolve role from persisted data.
    pub fn resolve_role(&self, email: &str) -> UserRole {
        if self.top_admin.lock().unwrap().as_deref() == Some(email) {
            return UserRole::TopAdmin;
        }
        if self.admins.lock().unwrap().contains(email) {
            return UserRole::Admin;
        }
        UserRole::Regular
    }

    pub fn add_share_grant(&self, grant: ShareGrant) {
        let entry = serde_json::json!({
            "kind": "share",
            "share_id": grant.share_id,
            "owner_email": grant.owner_email,
            "path": grant.path,
            "grantee_email": grant.grantee_email,
            "permission": grant.permission,
            "created_at": grant.created_at,
        });
        let _ = self.append_entry(&entry);
        self.share_grants.lock().unwrap().push(grant);
    }

    pub fn shares_for_grantee(&self, grantee_email: &str) -> Vec<ShareGrant> {
        self.share_grants
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.grantee_email == grantee_email)
            .cloned()
            .collect()
    }

    /// Placeholder — returns empty.
    pub fn shares_for_path(&self, _path: &str) -> Vec<ShareGrant> {
        Vec::new()
    }

    /// Rewrite share grant paths when an exom or folder is renamed (`old_prefix` → `new_prefix`).
    pub fn update_share_paths(&self, old_prefix: &str, new_prefix: &str) {
        if old_prefix == new_prefix {
            return;
        }
        let prefix_slash = format!("{}/", old_prefix);
        let mut grants = self.share_grants.lock().unwrap();
        let mut to_log: Vec<(String, String)> = Vec::new();
        for grant in grants.iter_mut() {
            if grant.path == old_prefix || grant.path.starts_with(&prefix_slash) {
                let new_path = format!("{}{}", new_prefix, &grant.path[old_prefix.len()..]);
                if grant.path != new_path {
                    grant.path = new_path.clone();
                    to_log.push((grant.share_id.clone(), new_path));
                }
            }
        }
        drop(grants);
        for (share_id, new_path) in to_log {
            let entry = serde_json::json!({
                "kind": "share-path-update",
                "share_id": share_id,
                "old_path": old_prefix,
                "new_path": new_path,
            });
            let _ = self.append_entry(&entry);
        }
    }

    /// Replay the JSONL log to populate in-memory state.
    fn replay_jsonl(&self) -> anyhow::Result<()> {
        if !self.jsonl_path.exists() {
            return Ok(());
        }
        let file = std::fs::File::open(&self.jsonl_path)?;
        let reader = std::io::BufReader::new(file);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(entry) => self.apply_entry(&entry),
                Err(e) => {
                    eprintln!("auth: skipping malformed JSONL line: {e}");
                }
            }
        }
        Ok(())
    }

    /// Append one JSON line to the JSONL log.
    fn append_entry(&self, entry: &serde_json::Value) -> anyhow::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.jsonl_path)?;
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
        Ok(())
    }

    /// Apply a single JSONL entry to in-memory state.
    fn apply_entry(&self, entry: &serde_json::Value) {
        let Some(kind) = entry.get("kind").and_then(|v| v.as_str()) else {
            return;
        };
        match kind {
            "user" => {
                if let (Some(email), Some(display_name), Some(provider), Some(created_at)) = (
                    entry.get("email").and_then(|v| v.as_str()),
                    entry.get("display_name").and_then(|v| v.as_str()),
                    entry.get("provider").and_then(|v| v.as_str()),
                    entry.get("created_at").and_then(|v| v.as_str()),
                ) {
                    self.users.lock().unwrap().insert(
                        email.to_string(),
                        StoredUser {
                            email: email.to_string(),
                            display_name: display_name.to_string(),
                            provider: provider.to_string(),
                            created_at: created_at.to_string(),
                            active: true,
                        },
                    );
                }
            }
            "top-admin" => {
                if let Some(email) = entry.get("email").and_then(|v| v.as_str()) {
                    *self.top_admin.lock().unwrap() = Some(email.to_string());
                }
            }
            "admin" => {
                if let Some(email) = entry.get("email").and_then(|v| v.as_str()) {
                    self.admins.lock().unwrap().insert(email.to_string());
                }
            }
            "admin-revoke" => {
                if let Some(email) = entry.get("email").and_then(|v| v.as_str()) {
                    self.admins.lock().unwrap().remove(email);
                }
            }
            "api-key" => {
                if let (Some(key_id), Some(key_hash), Some(email), Some(label), Some(created_at)) = (
                    entry.get("key_id").and_then(|v| v.as_str()),
                    entry.get("key_hash").and_then(|v| v.as_str()),
                    entry.get("email").and_then(|v| v.as_str()),
                    entry.get("label").and_then(|v| v.as_str()),
                    entry.get("created_at").and_then(|v| v.as_str()),
                ) {
                    let stored = StoredApiKey {
                        key_id: key_id.to_string(),
                        key_hash: key_hash.to_string(),
                        email: email.to_string(),
                        label: label.to_string(),
                        created_at: created_at.to_string(),
                    };
                    self.api_key_by_hash
                        .lock()
                        .unwrap()
                        .insert(key_hash.to_string(), key_id.to_string());
                    self.api_keys
                        .lock()
                        .unwrap()
                        .insert(key_id.to_string(), stored);
                }
            }
            "api-key-revoke" => {
                if let Some(key_id) = entry.get("key_id").and_then(|v| v.as_str()) {
                    if let Some(removed) = self.api_keys.lock().unwrap().remove(key_id) {
                        self.api_key_by_hash.lock().unwrap().remove(&removed.key_hash);
                    }
                }
            }
            "share" => {
                if let (
                    Some(share_id),
                    Some(owner_email),
                    Some(path),
                    Some(grantee_email),
                    Some(permission),
                    Some(created_at),
                ) = (
                    entry.get("share_id").and_then(|v| v.as_str()),
                    entry.get("owner_email").and_then(|v| v.as_str()),
                    entry.get("path").and_then(|v| v.as_str()),
                    entry.get("grantee_email").and_then(|v| v.as_str()),
                    entry.get("permission").and_then(|v| v.as_str()),
                    entry.get("created_at").and_then(|v| v.as_str()),
                ) {
                    self.share_grants.lock().unwrap().push(ShareGrant {
                        share_id: share_id.to_string(),
                        owner_email: owner_email.to_string(),
                        path: path.to_string(),
                        grantee_email: grantee_email.to_string(),
                        permission: permission.to_string(),
                        created_at: created_at.to_string(),
                    });
                }
            }
            "share-revoke" => {
                if let Some(share_id) = entry.get("share_id").and_then(|v| v.as_str()) {
                    self.share_grants
                        .lock()
                        .unwrap()
                        .retain(|g| g.share_id != share_id);
                }
            }
            "share-path-update" => {
                let share_id = entry["share_id"].as_str().unwrap_or_default();
                let new_path = entry["new_path"].as_str().unwrap_or_default();
                let mut grants = self.share_grants.lock().unwrap();
                if let Some(grant) = grants.iter_mut().find(|g| g.share_id == share_id) {
                    grant.path = new_path.to_string();
                }
            }
            "domain" => {
                if let Some(domain) = entry.get("domain").and_then(|v| v.as_str()) {
                    let mut domains = self.allowed_domains.lock().unwrap();
                    if !domains.contains(&domain.to_string()) {
                        domains.push(domain.to_string());
                    }
                }
            }
            "domain-revoke" => {
                if let Some(domain) = entry.get("domain").and_then(|v| v.as_str()) {
                    self.allowed_domains
                        .lock()
                        .unwrap()
                        .retain(|d| d != domain);
                }
            }
            "user-deactivate" => {
                if let Some(email) = entry.get("email").and_then(|v| v.as_str()) {
                    if let Some(user) = self.users.lock().unwrap().get_mut(email) {
                        user.active = false;
                    }
                }
            }
            "user-activate" => {
                if let Some(email) = entry.get("email").and_then(|v| v.as_str()) {
                    if let Some(user) = self.users.lock().unwrap().get_mut(email) {
                        user.active = true;
                    }
                }
            }
            _ => {
                eprintln!("auth: unknown JSONL kind: {kind}");
            }
        }
    }

    /// Rebuild api_key_cache from persisted keys so Bearer auth works on restart.
    fn rebuild_api_key_cache(&self) {
        let keys = self.api_keys.lock().unwrap();
        let users = self.users.lock().unwrap();
        let top_admin = self.top_admin.lock().unwrap();
        let admins = self.admins.lock().unwrap();

        for stored_key in keys.values() {
            let role = if top_admin.as_deref() == Some(&stored_key.email) {
                UserRole::TopAdmin
            } else if admins.contains(&stored_key.email) {
                UserRole::Admin
            } else {
                UserRole::Regular
            };

            let user_info = users.get(&stored_key.email);
            let user = User {
                email: stored_key.email.clone(),
                display_name: user_info
                    .map(|u| u.display_name.clone())
                    .unwrap_or_else(|| stored_key.email.clone()),
                provider: user_info
                    .map(|u| u.provider.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                session_id: None,
                role,
            };
            self.api_key_cache
                .insert(stored_key.key_hash.clone(), user);
        }
    }

    /// Record a user login (persists to JSONL).
    pub fn record_user(&self, email: &str, display_name: &str, provider: &str) {
        let created_at = chrono::Utc::now().to_rfc3339();
        let entry = serde_json::json!({
            "kind": "user",
            "email": email,
            "display_name": display_name,
            "provider": provider,
            "created_at": created_at,
        });
        let _ = self.append_entry(&entry);
        self.apply_entry(&entry);
    }

    /// Set the top admin (persists to JSONL). Only call once for the first user.
    pub fn set_top_admin(&self, email: &str) {
        let entry = serde_json::json!({ "kind": "top-admin", "email": email });
        let _ = self.append_entry(&entry);
        self.apply_entry(&entry);
    }

    /// Grant admin role (persists to JSONL).
    pub fn grant_admin(&self, email: &str) {
        let entry = serde_json::json!({ "kind": "admin", "email": email });
        let _ = self.append_entry(&entry);
        self.apply_entry(&entry);
    }

    /// Revoke admin role (persists to JSONL).
    pub fn revoke_admin(&self, email: &str) {
        let entry = serde_json::json!({ "kind": "admin-revoke", "email": email });
        let _ = self.append_entry(&entry);
        self.apply_entry(&entry);
    }

    /// Record an API key (persists to JSONL).
    pub fn record_api_key(&self, key_id: &str, key_hash: &str, email: &str, label: &str) {
        let created_at = chrono::Utc::now().to_rfc3339();
        let entry = serde_json::json!({
            "kind": "api-key",
            "key_id": key_id,
            "key_hash": key_hash,
            "email": email,
            "label": label,
            "created_at": created_at,
        });
        let _ = self.append_entry(&entry);
        self.apply_entry(&entry);
    }

    /// Revoke an API key by key_id (persists to JSONL).
    pub fn revoke_api_key_by_id(&self, key_id: &str) -> bool {
        let keys = self.api_keys.lock().unwrap();
        if !keys.contains_key(key_id) {
            return false;
        }
        drop(keys);

        let entry = serde_json::json!({ "kind": "api-key-revoke", "key_id": key_id });
        let _ = self.append_entry(&entry);

        // Remove from api_keys and api_key_by_hash
        if let Some(removed) = self.api_keys.lock().unwrap().remove(key_id) {
            self.api_key_by_hash.lock().unwrap().remove(&removed.key_hash);
            // Also evict from runtime cache
            self.api_key_cache.remove(&removed.key_hash);
        }
        true
    }

    /// Revoke a share grant by share_id (persists to JSONL).
    pub fn revoke_share_by_id(&self, share_id: &str) -> bool {
        let mut grants = self.share_grants.lock().unwrap();
        let before = grants.len();
        grants.retain(|g| g.share_id != share_id);
        let removed = grants.len() < before;
        drop(grants);

        if removed {
            let entry = serde_json::json!({ "kind": "share-revoke", "share_id": share_id });
            let _ = self.append_entry(&entry);
        }
        removed
    }

    /// Add an allowed domain (persists to JSONL).
    pub fn add_domain(&self, domain: &str) {
        let domain = domain.trim().to_lowercase();
        if domain.is_empty() {
            return;
        }
        let entry = serde_json::json!({ "kind": "domain", "domain": domain });
        let _ = self.append_entry(&entry);
        self.apply_entry(&entry);
    }

    /// Remove an allowed domain (persists to JSONL).
    pub fn remove_domain(&self, domain: &str) {
        let entry = serde_json::json!({ "kind": "domain-revoke", "domain": domain });
        let _ = self.append_entry(&entry);
        self.apply_entry(&entry);
    }

    /// Deactivate a user (persists to JSONL).
    pub fn deactivate_user(&self, email: &str) {
        let entry = serde_json::json!({ "kind": "user-deactivate", "email": email });
        let _ = self.append_entry(&entry);
        self.apply_entry(&entry);
    }

    /// Activate a user (persists to JSONL).
    pub fn activate_user(&self, email: &str) {
        let entry = serde_json::json!({ "kind": "user-activate", "email": email });
        let _ = self.append_entry(&entry);
        self.apply_entry(&entry);
    }

    /// List all stored users.
    pub fn list_users(&self) -> Vec<StoredUser> {
        self.users.lock().unwrap().values().cloned().collect()
    }

    /// List all stored API keys (for admin listing).
    pub fn list_api_keys(&self) -> Vec<StoredApiKey> {
        self.api_keys.lock().unwrap().values().cloned().collect()
    }

    /// List API keys for a specific user.
    pub fn list_api_keys_for_user(&self, email: &str) -> Vec<StoredApiKey> {
        self.api_keys
            .lock()
            .unwrap()
            .values()
            .filter(|k| k.email == email)
            .cloned()
            .collect()
    }

    /// List all share grants (for admin listing).
    pub fn list_all_shares(&self) -> Vec<ShareGrant> {
        self.share_grants.lock().unwrap().clone()
    }

    /// List shares owned by a specific user.
    pub fn list_shares_for_owner(&self, email: &str) -> Vec<ShareGrant> {
        self.share_grants
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.owner_email == email)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_domain_empty_allows_all() {
        let store = make_test_store(&[]);
        assert!(store.check_domain("alice@anything.com"));
        assert!(store.check_domain("bob@other.org"));
    }

    #[test]
    fn check_domain_restricts() {
        let store = make_test_store(&["company.com".into(), "other.co".into()]);
        assert!(store.check_domain("alice@company.com"));
        assert!(store.check_domain("bob@other.co"));
        assert!(!store.check_domain("eve@evil.org"));
    }

    #[test]
    fn hash_api_key_deterministic() {
        let h1 = AuthStore::hash_api_key("test-key-123");
        let h2 = AuthStore::hash_api_key("test-key-123");
        assert_eq!(h1, h2);
        assert_ne!(h1, AuthStore::hash_api_key("different-key"));
    }

    #[test]
    fn session_cache_round_trip() {
        let store = make_test_store(&[]);
        let user = User {
            email: "alice@co.com".into(),
            display_name: "Alice".into(),
            provider: "mock".into(),
            session_id: Some("sess-1".into()),
            role: UserRole::Regular,
        };
        store.session_cache.insert("sess-1".into(), user.clone());
        let found = store.get_user_by_session("sess-1").unwrap();
        assert_eq!(found.email, "alice@co.com");
        store.evict_session("sess-1");
        assert!(store.get_user_by_session("sess-1").is_none());
    }

    #[test]
    fn generate_api_key_unique() {
        let store = make_test_store(&[]);
        let (id1, key1) = store.generate_api_key("alice@co.com", "key1");
        let (id2, key2) = store.generate_api_key("alice@co.com", "key2");
        assert_ne!(id1, id2);
        assert_ne!(key1, key2);
    }

    #[test]
    fn replay_jsonl_populates_state() {
        let dir = tempfile::tempdir().unwrap();
        let jsonl_path = dir.path().join("auth.jsonl");
        std::fs::write(
            &jsonl_path,
            r#"{"kind":"user","email":"alice@co.com","display_name":"Alice","provider":"google","created_at":"2026-01-01T00:00:00Z"}
{"kind":"top-admin","email":"alice@co.com"}
{"kind":"admin","email":"bob@co.com"}
{"kind":"api-key","key_id":"k1","key_hash":"h1","email":"alice@co.com","label":"my-key","created_at":"2026-01-01T00:00:00Z"}
{"kind":"share","share_id":"s1","owner_email":"alice@co.com","path":"alice@co.com/proj","grantee_email":"bob@co.com","permission":"read","created_at":"2026-01-01T00:00:00Z"}
{"kind":"domain","domain":"co.com"}
"#,
        )
        .unwrap();

        let store = AuthStore {
            exom_disk: dir.path().to_path_buf(),
            jsonl_path,
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(Vec::new()),
            share_grants: Mutex::new(Vec::new()),
            users: Mutex::new(HashMap::new()),
            api_keys: Mutex::new(HashMap::new()),
            api_key_by_hash: Mutex::new(HashMap::new()),
            top_admin: Mutex::new(None),
            admins: Mutex::new(HashSet::new()),
        };
        store.replay_jsonl().unwrap();

        assert_eq!(store.users.lock().unwrap().len(), 1);
        assert_eq!(
            store.top_admin.lock().unwrap().as_deref(),
            Some("alice@co.com")
        );
        assert!(store.admins.lock().unwrap().contains("bob@co.com"));
        assert_eq!(store.api_keys.lock().unwrap().len(), 1);
        assert_eq!(
            store.api_key_by_hash.lock().unwrap().get("h1"),
            Some(&"k1".to_string())
        );
        assert_eq!(store.share_grants.lock().unwrap().len(), 1);
        assert_eq!(
            store.allowed_domains.lock().unwrap().clone(),
            vec!["co.com".to_string()]
        );
        assert_eq!(store.resolve_role("alice@co.com"), UserRole::TopAdmin);
        assert_eq!(store.resolve_role("bob@co.com"), UserRole::Admin);
        assert_eq!(store.resolve_role("eve@co.com"), UserRole::Regular);
    }

    #[test]
    fn replay_handles_revocations() {
        let dir = tempfile::tempdir().unwrap();
        let jsonl_path = dir.path().join("auth.jsonl");
        std::fs::write(
            &jsonl_path,
            r#"{"kind":"admin","email":"bob@co.com"}
{"kind":"admin-revoke","email":"bob@co.com"}
{"kind":"api-key","key_id":"k1","key_hash":"h1","email":"alice@co.com","label":"key","created_at":"2026-01-01T00:00:00Z"}
{"kind":"api-key-revoke","key_id":"k1"}
{"kind":"share","share_id":"s1","owner_email":"alice@co.com","path":"p","grantee_email":"bob@co.com","permission":"read","created_at":"2026-01-01T00:00:00Z"}
{"kind":"share-revoke","share_id":"s1"}
{"kind":"domain","domain":"co.com"}
{"kind":"domain-revoke","domain":"co.com"}
{"kind":"user","email":"bob@co.com","display_name":"Bob","provider":"google","created_at":"2026-01-01T00:00:00Z"}
{"kind":"user-deactivate","email":"bob@co.com"}
"#,
        )
        .unwrap();

        let store = AuthStore {
            exom_disk: dir.path().to_path_buf(),
            jsonl_path,
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(Vec::new()),
            share_grants: Mutex::new(Vec::new()),
            users: Mutex::new(HashMap::new()),
            api_keys: Mutex::new(HashMap::new()),
            api_key_by_hash: Mutex::new(HashMap::new()),
            top_admin: Mutex::new(None),
            admins: Mutex::new(HashSet::new()),
        };
        store.replay_jsonl().unwrap();

        assert!(!store.admins.lock().unwrap().contains("bob@co.com"));
        assert!(store.api_keys.lock().unwrap().is_empty());
        assert!(store.api_key_by_hash.lock().unwrap().is_empty());
        assert!(store.share_grants.lock().unwrap().is_empty());
        assert!(store.allowed_domains.lock().unwrap().is_empty());
        assert!(!store.users.lock().unwrap().get("bob@co.com").unwrap().active);
    }

    #[test]
    fn append_and_replay_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let jsonl_path = dir.path().join("auth.jsonl");

        let store = AuthStore {
            exom_disk: dir.path().to_path_buf(),
            jsonl_path: jsonl_path.clone(),
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(Vec::new()),
            share_grants: Mutex::new(Vec::new()),
            users: Mutex::new(HashMap::new()),
            api_keys: Mutex::new(HashMap::new()),
            api_key_by_hash: Mutex::new(HashMap::new()),
            top_admin: Mutex::new(None),
            admins: Mutex::new(HashSet::new()),
        };

        // Use public methods to persist data.
        store.record_user("alice@co.com", "Alice", "google");
        store.set_top_admin("alice@co.com");
        store.record_api_key("k1", "h1", "alice@co.com", "my-key");
        store.add_domain("co.com");

        // Now create a fresh store and replay.
        let store2 = AuthStore {
            exom_disk: dir.path().to_path_buf(),
            jsonl_path,
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(Vec::new()),
            share_grants: Mutex::new(Vec::new()),
            users: Mutex::new(HashMap::new()),
            api_keys: Mutex::new(HashMap::new()),
            api_key_by_hash: Mutex::new(HashMap::new()),
            top_admin: Mutex::new(None),
            admins: Mutex::new(HashSet::new()),
        };
        store2.replay_jsonl().unwrap();

        assert_eq!(store2.users.lock().unwrap().len(), 1);
        assert_eq!(store2.resolve_role("alice@co.com"), UserRole::TopAdmin);
        assert_eq!(store2.api_keys.lock().unwrap().len(), 1);
        assert_eq!(
            store2.allowed_domains.lock().unwrap().clone(),
            vec!["co.com".to_string()]
        );
    }

    #[test]
    fn update_share_paths_on_rename() {
        let store = make_test_store(&[]);
        store.add_share_grant(ShareGrant {
            share_id: "s1".into(),
            owner_email: "alice@co.com".into(),
            path: "alice@co.com/proj".into(),
            grantee_email: "bob@co.com".into(),
            permission: "read".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        store.add_share_grant(ShareGrant {
            share_id: "s2".into(),
            owner_email: "alice@co.com".into(),
            path: "alice@co.com/proj/sub".into(),
            grantee_email: "carol@co.com".into(),
            permission: "read-write".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        });

        store.update_share_paths("alice@co.com/proj", "alice@co.com/work");

        let grants = store.share_grants.lock().unwrap();
        assert_eq!(grants[0].path, "alice@co.com/work");
        assert_eq!(grants[1].path, "alice@co.com/work/sub");
    }

    fn make_test_store(domains: &[String]) -> AuthStore {
        AuthStore {
            exom_disk: PathBuf::from("/tmp/fake"),
            jsonl_path: PathBuf::from("/tmp/fake/auth.jsonl"),
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(domains.to_vec()),
            share_grants: Mutex::new(Vec::new()),
            users: Mutex::new(HashMap::new()),
            api_keys: Mutex::new(HashMap::new()),
            api_key_by_hash: Mutex::new(HashMap::new()),
            top_admin: Mutex::new(None),
            admins: Mutex::new(HashSet::new()),
        }
    }
}
