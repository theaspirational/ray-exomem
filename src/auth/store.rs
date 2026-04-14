//! AuthStore — typed read/write over _system/auth exom.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dashmap::DashMap;

use crate::auth::{User, UserRole};
use crate::exom::{self, ExomMeta, META_FILENAME};

pub struct AuthStore {
    pub exom_disk: PathBuf,
    pub session_cache: DashMap<String, User>,
    pub api_key_cache: DashMap<String, User>,
    pub allowed_domains: Mutex<Vec<String>>,
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

impl AuthStore {
    /// Creates `_system/auth` directory + `exom.json` if not present,
    /// loads existing state, seeds domains on first boot only.
    pub fn bootstrap(tree_root: &Path, seed_domains: &[String]) -> anyhow::Result<Self> {
        let exom_disk = tree_root.join("_system").join("auth");
        let meta_file = exom_disk.join(META_FILENAME);

        let first_boot = !meta_file.exists();
        if first_boot {
            let meta = ExomMeta::new_bare();
            exom::write_meta(&exom_disk, &meta)?;
        }

        let domains = if first_boot {
            seed_domains.to_vec()
        } else {
            // On subsequent boots, start with empty (allow-all).
            // Full persistence will be wired later via the exom fact store.
            Vec::new()
        };

        Ok(Self {
            exom_disk,
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(domains),
        })
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

    /// Placeholder — returns Regular for now.
    pub fn resolve_role(&self, _email: &str) -> UserRole {
        UserRole::Regular
    }

    /// Placeholder — returns empty.
    pub fn shares_for_grantee(&self, _grantee_email: &str) -> Vec<ShareGrant> {
        Vec::new()
    }

    /// Placeholder — returns empty.
    pub fn shares_for_path(&self, _path: &str) -> Vec<ShareGrant> {
        Vec::new()
    }

    /// Placeholder for rename support.
    pub fn update_share_paths(&self, _old_prefix: &str, _new_prefix: &str) {}
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

    fn make_test_store(domains: &[String]) -> AuthStore {
        AuthStore {
            exom_disk: PathBuf::from("/tmp/fake"),
            session_cache: DashMap::new(),
            api_key_cache: DashMap::new(),
            allowed_domains: Mutex::new(domains.to_vec()),
        }
    }
}
