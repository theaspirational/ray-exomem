//! JSONL-backed auth persistence with in-memory indexes (replay log).

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;

use crate::auth::UserRole;
use crate::db::{
    AllowedEmail, ApiKeyWithUser, AuthDb, SessionRow, ShareGrant, StoredApiKey, StoredUser,
};

pub struct JsonlAuthDb {
    jsonl_path: PathBuf,
    users: Mutex<HashMap<String, StoredUser>>,
    api_keys: Mutex<HashMap<String, StoredApiKey>>,
    api_key_by_hash: Mutex<HashMap<String, String>>,
    share_grants: Mutex<Vec<ShareGrant>>,
    allowed_domains: Mutex<Vec<String>>,
    allowed_emails: Mutex<HashMap<String, String>>,
    top_admin: Mutex<Option<String>>,
    admins: Mutex<HashSet<String>>,
    sessions: DashMap<String, SessionRow>,
}

impl JsonlAuthDb {
    pub fn new(jsonl_path: PathBuf) -> anyhow::Result<Self> {
        let db = Self {
            jsonl_path,
            users: Mutex::new(HashMap::new()),
            api_keys: Mutex::new(HashMap::new()),
            api_key_by_hash: Mutex::new(HashMap::new()),
            share_grants: Mutex::new(Vec::new()),
            allowed_domains: Mutex::new(Vec::new()),
            allowed_emails: Mutex::new(HashMap::new()),
            top_admin: Mutex::new(None),
            admins: Mutex::new(HashSet::new()),
            sessions: DashMap::new(),
        };
        db.replay_jsonl()?;
        Ok(db)
    }

    fn resolve_role(&self, email: &str) -> UserRole {
        if self.top_admin.lock().unwrap().as_deref() == Some(email) {
            return UserRole::TopAdmin;
        }
        if self.admins.lock().unwrap().contains(email) {
            return UserRole::Admin;
        }
        UserRole::Regular
    }

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

    fn append_entry(&self, entry: &serde_json::Value) -> anyhow::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.jsonl_path)?;
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
        Ok(())
    }

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
                    let email = email.to_string();
                    let role = self.resolve_role(&email);
                    let active = entry
                        .get("active")
                        .and_then(|v| v.as_bool())
                        .or_else(|| self.users.lock().unwrap().get(&email).map(|u| u.active))
                        .unwrap_or(true);
                    let last_login = entry
                        .get("last_login")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or_else(|| {
                            self.users
                                .lock()
                                .unwrap()
                                .get(&email)
                                .and_then(|u| u.last_login.clone())
                        });
                    self.users.lock().unwrap().insert(
                        email.clone(),
                        StoredUser {
                            email,
                            display_name: display_name.to_string(),
                            provider: provider.to_string(),
                            role,
                            active,
                            created_at: created_at.to_string(),
                            last_login,
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
                        self.api_key_by_hash
                            .lock()
                            .unwrap()
                            .remove(&removed.key_hash);
                    }
                }
            }
            "api-key-relabel" => {
                if let (Some(key_id), Some(label)) = (
                    entry.get("key_id").and_then(|v| v.as_str()),
                    entry.get("label").and_then(|v| v.as_str()),
                ) {
                    if let Some(stored) = self.api_keys.lock().unwrap().get_mut(key_id) {
                        stored.label = label.to_string();
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
                    self.allowed_domains.lock().unwrap().retain(|d| d != domain);
                }
            }
            "allowed-email" => {
                if let Some(email) = entry.get("email").and_then(|v| v.as_str()) {
                    let alias = entry
                        .get("alias")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    self.allowed_emails
                        .lock()
                        .unwrap()
                        .insert(email.to_string(), alias);
                }
            }
            "allowed-email-revoke" => {
                if let Some(email) = entry.get("email").and_then(|v| v.as_str()) {
                    self.allowed_emails.lock().unwrap().remove(email);
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
            "user-delete" => {
                if let Some(email) = entry.get("email").and_then(|v| v.as_str()) {
                    self.users.lock().unwrap().remove(email);
                    self.admins.lock().unwrap().remove(email);
                    if self.top_admin.lock().unwrap().as_deref() == Some(email) {
                        *self.top_admin.lock().unwrap() = None;
                    }
                    let key_ids: Vec<String> = self
                        .api_keys
                        .lock()
                        .unwrap()
                        .values()
                        .filter(|k| k.email == email)
                        .map(|k| k.key_id.clone())
                        .collect();
                    for key_id in key_ids {
                        if let Some(removed) = self.api_keys.lock().unwrap().remove(&key_id) {
                            self.api_key_by_hash
                                .lock()
                                .unwrap()
                                .remove(&removed.key_hash);
                        }
                    }
                    self.share_grants.lock().unwrap().retain(|grant| {
                        grant.owner_email != email
                            && grant.grantee_email != email
                            && grant.path != email
                            && !grant.path.starts_with(&format!("{email}/"))
                    });
                    self.sessions.retain(|_, sess| sess.email != email);
                }
            }
            _ => {
                eprintln!("auth: unknown JSONL kind: {kind}");
            }
        }
    }

    fn now_rfc3339() -> String {
        Utc::now().to_rfc3339()
    }
}

#[async_trait]
impl AuthDb for JsonlAuthDb {
    async fn upsert_user(&self, user: &StoredUser) -> anyhow::Result<()> {
        let entry = serde_json::json!({
            "kind": "user",
            "email": user.email,
            "display_name": user.display_name,
            "provider": user.provider,
            "created_at": user.created_at,
            "active": user.active,
            "last_login": user.last_login,
        });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn get_user(&self, email: &str) -> anyhow::Result<Option<StoredUser>> {
        let u = self.users.lock().unwrap().get(email).cloned();
        Ok(u.map(|mut u| {
            u.role = self.resolve_role(email);
            u
        }))
    }

    async fn list_users(&self) -> anyhow::Result<Vec<StoredUser>> {
        let users = self.users.lock().unwrap();
        let mut out: Vec<StoredUser> = users
            .values()
            .map(|u| {
                let mut u = u.clone();
                u.role = self.resolve_role(&u.email);
                u
            })
            .collect();
        drop(users);
        out.sort_by(|a, b| a.email.cmp(&b.email));
        Ok(out)
    }

    async fn set_role(&self, email: &str, role: UserRole) -> anyhow::Result<()> {
        let entry = match role {
            UserRole::TopAdmin => serde_json::json!({ "kind": "top-admin", "email": email }),
            UserRole::Admin => serde_json::json!({ "kind": "admin", "email": email }),
            UserRole::Regular => serde_json::json!({ "kind": "admin-revoke", "email": email }),
        };
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn deactivate_user(&self, email: &str) -> anyhow::Result<()> {
        let entry = serde_json::json!({ "kind": "user-deactivate", "email": email });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn activate_user(&self, email: &str) -> anyhow::Result<()> {
        let entry = serde_json::json!({ "kind": "user-activate", "email": email });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn delete_user(&self, email: &str) -> anyhow::Result<bool> {
        if !self.users.lock().unwrap().contains_key(email) {
            return Ok(false);
        }
        let entry = serde_json::json!({ "kind": "user-delete", "email": email });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(true)
    }

    async fn update_last_login(&self, _email: &str, _at: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn create_session(&self, session: &SessionRow) -> anyhow::Result<()> {
        self.sessions
            .insert(session.session_id.clone(), session.clone());
        Ok(())
    }

    async fn get_session(&self, session_id: &str) -> anyhow::Result<Option<SessionRow>> {
        let now = Self::now_rfc3339();
        Ok(self.sessions.get(session_id).and_then(|s| {
            if s.expires_at > now {
                Some(s.clone())
            } else {
                None
            }
        }))
    }

    async fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        self.sessions.remove(session_id);
        Ok(())
    }

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionRow>> {
        let now = Self::now_rfc3339();
        let mut rows: Vec<SessionRow> = self
            .sessions
            .iter()
            .filter_map(|r| {
                if r.expires_at > now {
                    Some(r.clone())
                } else {
                    None
                }
            })
            .collect();
        rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        Ok(rows)
    }

    async fn cleanup_expired_sessions(&self) -> anyhow::Result<usize> {
        let now = Self::now_rfc3339();
        let mut removed = 0usize;
        self.sessions.retain(|_, s| {
            if s.expires_at <= now {
                removed += 1;
                false
            } else {
                true
            }
        });
        Ok(removed)
    }

    async fn store_api_key(&self, key: &StoredApiKey) -> anyhow::Result<()> {
        let entry = serde_json::json!({
            "kind": "api-key",
            "key_id": key.key_id,
            "key_hash": key.key_hash,
            "email": key.email,
            "label": key.label,
            "created_at": key.created_at,
        });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn revoke_api_key(&self, key_id: &str) -> anyhow::Result<bool> {
        let keys = self.api_keys.lock().unwrap();
        if !keys.contains_key(key_id) {
            return Ok(false);
        }
        drop(keys);

        let entry = serde_json::json!({ "kind": "api-key-revoke", "key_id": key_id });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(true)
    }

    async fn rename_api_key(&self, key_id: &str, new_label: &str) -> anyhow::Result<bool> {
        let keys = self.api_keys.lock().unwrap();
        if !keys.contains_key(key_id) {
            return Ok(false);
        }
        drop(keys);

        let entry = serde_json::json!({
            "kind": "api-key-relabel",
            "key_id": key_id,
            "label": new_label,
        });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(true)
    }

    async fn list_api_keys(&self) -> anyhow::Result<Vec<ApiKeyWithUser>> {
        let keys = self.api_keys.lock().unwrap();
        let mut out = Vec::with_capacity(keys.len());
        for k in keys.values() {
            let user = self
                .users
                .lock()
                .unwrap()
                .get(&k.email)
                .map(|u| {
                    let mut u = u.clone();
                    u.role = self.resolve_role(&u.email);
                    u
                })
                .unwrap_or_else(|| StoredUser {
                    email: k.email.clone(),
                    display_name: k.email.clone(),
                    provider: "unknown".into(),
                    role: self.resolve_role(&k.email),
                    active: true,
                    created_at: String::new(),
                    last_login: None,
                });
            out.push(ApiKeyWithUser {
                key_id: k.key_id.clone(),
                key_hash: k.key_hash.clone(),
                email: k.email.clone(),
                label: k.label.clone(),
                created_at: k.created_at.clone(),
                user,
            });
        }
        drop(keys);
        out.sort_by(|a, b| a.key_id.cmp(&b.key_id));
        Ok(out)
    }

    async fn list_api_keys_for_user(&self, email: &str) -> anyhow::Result<Vec<StoredApiKey>> {
        let keys = self.api_keys.lock().unwrap();
        let mut out: Vec<StoredApiKey> = keys
            .values()
            .filter(|k| k.email == email)
            .cloned()
            .collect();
        drop(keys);
        out.sort_by(|a, b| a.key_id.cmp(&b.key_id));
        Ok(out)
    }

    async fn get_api_key_by_hash(&self, key_hash: &str) -> anyhow::Result<Option<ApiKeyWithUser>> {
        let key_id = {
            let by_hash = self.api_key_by_hash.lock().unwrap();
            by_hash.get(key_hash).cloned()
        };
        let Some(key_id) = key_id else {
            return Ok(None);
        };
        let stored = {
            let keys = self.api_keys.lock().unwrap();
            keys.get(&key_id).cloned()
        };
        let Some(k) = stored else {
            return Ok(None);
        };
        let user = self
            .users
            .lock()
            .unwrap()
            .get(&k.email)
            .map(|u| {
                let mut u = u.clone();
                u.role = self.resolve_role(&u.email);
                u
            })
            .unwrap_or_else(|| StoredUser {
                email: k.email.clone(),
                display_name: k.email.clone(),
                provider: "unknown".into(),
                role: self.resolve_role(&k.email),
                active: true,
                created_at: String::new(),
                last_login: None,
            });
        Ok(Some(ApiKeyWithUser {
            key_id: k.key_id,
            key_hash: k.key_hash,
            email: k.email.clone(),
            label: k.label,
            created_at: k.created_at,
            user,
        }))
    }

    async fn add_share(&self, grant: &ShareGrant) -> anyhow::Result<()> {
        let entry = serde_json::json!({
            "kind": "share",
            "share_id": grant.share_id,
            "owner_email": grant.owner_email,
            "path": grant.path,
            "grantee_email": grant.grantee_email,
            "permission": grant.permission,
            "created_at": grant.created_at,
        });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn revoke_share(&self, share_id: &str) -> anyhow::Result<bool> {
        let mut grants = self.share_grants.lock().unwrap();
        let before = grants.len();
        grants.retain(|g| g.share_id != share_id);
        let removed = grants.len() < before;
        drop(grants);

        if removed {
            let entry = serde_json::json!({ "kind": "share-revoke", "share_id": share_id });
            self.append_entry(&entry)?;
        }
        Ok(removed)
    }

    async fn shares_for_grantee(&self, grantee_email: &str) -> anyhow::Result<Vec<ShareGrant>> {
        Ok(self
            .share_grants
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.grantee_email == grantee_email)
            .cloned()
            .collect())
    }

    async fn shares_for_owner(&self, owner_email: &str) -> anyhow::Result<Vec<ShareGrant>> {
        Ok(self
            .share_grants
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.owner_email == owner_email)
            .cloned()
            .collect())
    }

    async fn list_all_shares(&self) -> anyhow::Result<Vec<ShareGrant>> {
        Ok(self.share_grants.lock().unwrap().clone())
    }

    async fn update_share_paths(&self, old_prefix: &str, new_prefix: &str) -> anyhow::Result<u64> {
        if old_prefix == new_prefix {
            return Ok(0);
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
        for (share_id, new_path) in &to_log {
            let entry = serde_json::json!({
                "kind": "share-path-update",
                "share_id": share_id,
                "old_path": old_prefix,
                "new_path": new_path,
            });
            self.append_entry(&entry)?;
        }
        Ok(to_log.len() as u64)
    }

    async fn add_domain(&self, domain: &str) -> anyhow::Result<()> {
        let domain = domain.trim().to_lowercase();
        if domain.is_empty() {
            return Ok(());
        }
        let entry = serde_json::json!({ "kind": "domain", "domain": domain });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn remove_domain(&self, domain: &str) -> anyhow::Result<()> {
        let entry = serde_json::json!({ "kind": "domain-revoke", "domain": domain });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn list_domains(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.allowed_domains.lock().unwrap().clone())
    }

    async fn add_allowed_email(&self, email: &str, alias: &str) -> anyhow::Result<()> {
        let email = email.trim().to_lowercase();
        if email.is_empty() {
            return Ok(());
        }
        let alias = alias.trim();
        let entry = serde_json::json!({
            "kind": "allowed-email",
            "email": email,
            "alias": alias,
        });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn remove_allowed_email(&self, email: &str) -> anyhow::Result<()> {
        let entry = serde_json::json!({
            "kind": "allowed-email-revoke",
            "email": email,
        });
        self.append_entry(&entry)?;
        self.apply_entry(&entry);
        Ok(())
    }

    async fn list_allowed_emails(&self) -> anyhow::Result<Vec<AllowedEmail>> {
        let map = self.allowed_emails.lock().unwrap();
        let mut out: Vec<AllowedEmail> = map
            .iter()
            .map(|(email, alias)| AllowedEmail {
                email: email.clone(),
                alias: alias.clone(),
            })
            .collect();
        drop(map);
        out.sort_by(|a, b| a.email.cmp(&b.email));
        Ok(out)
    }

    async fn factory_reset(&self) -> anyhow::Result<()> {
        // Preserve login policy (allowed_domains + allowed_emails) across the
        // reset; everything else goes.
        let domains: Vec<String> = self.allowed_domains.lock().unwrap().clone();
        let emails: Vec<(String, String)> = self
            .allowed_emails
            .lock()
            .unwrap()
            .iter()
            .map(|(e, a)| (e.clone(), a.clone()))
            .collect();

        self.users.lock().unwrap().clear();
        self.api_keys.lock().unwrap().clear();
        self.api_key_by_hash.lock().unwrap().clear();
        self.share_grants.lock().unwrap().clear();
        *self.top_admin.lock().unwrap() = None;
        self.admins.lock().unwrap().clear();
        self.sessions.clear();

        if let Some(parent) = self.jsonl_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.jsonl_path)?;
        for d in domains {
            let entry = serde_json::json!({ "kind": "domain", "domain": d });
            writeln!(file, "{}", serde_json::to_string(&entry)?)?;
        }
        for (email, alias) in emails {
            let entry = serde_json::json!({
                "kind": "allowed-email",
                "email": email,
                "alias": alias,
            });
            writeln!(file, "{}", serde_json::to_string(&entry)?)?;
        }
        Ok(())
    }
}
