use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FORMAT_VERSION: u32 = 2;
pub const META_FILENAME: &str = "exom.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExomKind {
    ProjectMain,
    Session,
    Bare,
}

/// Per-exom write policy.
///
/// - `SoloEdit` (default): only `created_by` writes the trunk; non-creators
///   with grants get whatever their grant says (the existing model).
/// - `CoEdit`: the `main` branch's TOFU claim is bypassed, so any user
///   admitted by the auth layer can write to `main`. Non-`main` branches
///   keep TOFU regardless of mode. Concurrent same-`fact_id` writes resolve
///   last-write-wins (existing `assert_fact` behaviour).
///
/// Only `Bare` and `ProjectMain` exoms may be `CoEdit`; `Session` exoms
/// are always `SoloEdit` (their structured-collab model uses
/// orchestrator-allocated branches instead).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AclMode {
    #[default]
    SoloEdit,
    CoEdit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    Multi,
    Single,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMeta {
    #[serde(rename = "type")]
    pub session_type: SessionType,
    pub label: String,
    pub initiated_by: String,
    pub agents: Vec<String>,
    pub closed_at: Option<String>,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExomMeta {
    pub format_version: u32,
    pub kind: ExomKind,
    pub created_at: String,
    /// Email of the user who created this exom. Empty string means
    /// system-owned or pre-Model-A legacy (a startup migration backfills
    /// from the `main` branch's TOFU claimer; an exom that ends up empty
    /// is effectively read-only for everyone except top-admin recovery).
    /// Drives the `public/*` access decision: only the creator gets
    /// FullAccess on a public exom; everyone else is ReadOnly + can fork.
    #[serde(default)]
    pub created_by: String,
    /// Write policy. `SoloEdit` is the default; `CoEdit` opens the `main`
    /// branch to all auth-admitted writers (Wikipedia-style commons). The
    /// creator can flip via `PATCH /api/actions/exom-mode`. Absent in
    /// pre-co-edit `exom.json` files; deserialises as `SoloEdit`.
    #[serde(default)]
    pub acl_mode: AclMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionMeta>,
    /// Lineage when this exom was created via `exom-fork`. Carried for
    /// future sync-request flows; absent on non-fork exoms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<ForkLineage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForkLineage {
    pub source_path: String,
    pub source_tx_id: u64,
    pub forked_at: String,
}

impl ExomMeta {
    pub fn new_project_main(created_by: &str) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            kind: ExomKind::ProjectMain,
            created_at: now_iso8601_basic(),
            created_by: created_by.to_string(),
            acl_mode: AclMode::SoloEdit,
            session: None,
            forked_from: None,
        }
    }

    pub fn new_bare(created_by: &str) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            kind: ExomKind::Bare,
            created_at: now_iso8601_basic(),
            created_by: created_by.to_string(),
            acl_mode: AclMode::SoloEdit,
            session: None,
            forked_from: None,
        }
    }

    pub fn new_session(sess: SessionMeta, created_by: &str) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            kind: ExomKind::Session,
            created_at: now_iso8601_basic(),
            created_by: created_by.to_string(),
            // Sessions cannot be co-edit (Q7): structured-collab is
            // orchestrator-allocated branches, not a shared trunk.
            acl_mode: AclMode::SoloEdit,
            session: Some(sess),
            forked_from: None,
        }
    }
}

pub fn meta_path(exom_disk: &Path) -> PathBuf {
    exom_disk.join(META_FILENAME)
}

pub fn write_meta(exom_disk: &Path, meta: &ExomMeta) -> io::Result<()> {
    let p = meta_path(exom_disk);
    fs::create_dir_all(exom_disk)?;
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(p, json)
}

pub fn read_meta(exom_disk: &Path) -> io::Result<ExomMeta> {
    let p = meta_path(exom_disk);
    let s = fs::read_to_string(p)?;
    serde_json::from_str(&s).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn session_id(ts_utc: &str, kind: SessionType, label: &str) -> String {
    let tag = match kind {
        SessionType::Multi => "multi",
        SessionType::Single => "single",
    };
    format!("{ts_utc}_{tag}_agent_{label}")
}

pub fn now_iso8601_basic() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let secs = now.as_secs();
    let (y, mo, d, h, mi, s) = epoch_to_ymdhms(secs);
    format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z")
}

fn epoch_to_ymdhms(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    // Simple UTC conversion without chrono.
    let s = (secs % 60) as u32;
    let m = ((secs / 60) % 60) as u32;
    let h = ((secs / 3600) % 24) as u32;
    let days = (secs / 86_400) as i64;
    let (y, mo, d) = days_to_ymd(days);
    (y, mo, d, h, m, s)
}

fn days_to_ymd(mut days: i64) -> (u32, u32, u32) {
    // Unix epoch is 1970-01-01 (Thursday).
    let mut y: i32 = 1970;
    loop {
        let ly = is_leap(y);
        let dy = if ly { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        y += 1;
    }
    let mdays = [
        31u32,
        if is_leap(y) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 1u32;
    for &md in &mdays {
        if days < md as i64 {
            break;
        }
        days -= md as i64;
        mo += 1;
    }
    (y as u32, mo, days as u32 + 1)
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// All-attempts-taken signal returned by `default_fork_target` when 100
/// auto-suffixed candidates were already in use. The caller should respond
/// with the canonical `fork_collision` error and ask the user to pass an
/// explicit `target`.
#[derive(Debug)]
pub struct ForkCollision;

impl std::fmt::Display for ForkCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("could not find a free target path; pass `target` explicitly")
    }
}

impl std::error::Error for ForkCollision {}

/// Compute the default fork target for `source_slash` when the caller
/// passed no explicit `target`. Returns the spec-correct
/// `{user_email}/forked/<source-subpath>` shape, with `-2`/`-3`/...
/// auto-suffix on the leaf segment if `tree_root.join(candidate)` already
/// exists.
///
/// `source_last` is the source path's last segment (`TreePath::last`),
/// accepted as a plain `Option<&str>` so callers don't have to import
/// `TreePath` here.
///
/// This is the **single source of truth** for the default-target shape.
/// Both `server::api_exom_fork` and `mcp::tool_exom_fork` call it, so
/// the two transports cannot drift again. Subpath stripping rules:
///
/// - `public/X/Y/Z`         → `{user}/forked/X/Y/Z`
/// - `{other_email}/X/Y`    → `{user}/forked/{other_email}/X/Y`
///   (preserves the source owner so lineage is readable from the path)
/// - `{user}/X/Y` (self-fork) → `{user}/forked/X/Y`
///
/// On collision (the disk path already exists) the leaf segment is
/// suffixed with `-2`, `-3`, ... up to 100 attempts.
pub fn default_fork_target(
    tree_root: &Path,
    user_email: &str,
    source_slash: &str,
    source_last: Option<&str>,
) -> Result<String, ForkCollision> {
    let self_prefix = format!("{}/", user_email);
    let subpath: String = if source_slash == "public" || source_slash.starts_with("public/") {
        source_slash
            .strip_prefix("public/")
            .unwrap_or("forked")
            .to_string()
    } else if source_slash == user_email || source_slash.starts_with(&self_prefix) {
        source_slash
            .strip_prefix(&self_prefix)
            .unwrap_or("forked")
            .to_string()
    } else {
        source_slash.to_string()
    };
    // Defensive: a folder root (no leaf) shouldn't reach here, but if it
    // does, fall back to the literal `forked` segment.
    let subpath = if subpath.is_empty() {
        source_last.unwrap_or("forked").to_string()
    } else {
        subpath
    };
    let base = format!("{}/forked/{}", user_email, subpath);
    let mut candidate = base.clone();
    let mut i = 2;
    while tree_root.join(&candidate).exists() {
        candidate = format!("{}-{}", base, i);
        i += 1;
        if i > 100 {
            return Err(ForkCollision);
        }
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_and_read_roundtrip() {
        let d = tempdir().unwrap();
        let exom = d.path().join("e");
        let meta = ExomMeta::new_project_main("alice@test");
        write_meta(&exom, &meta).unwrap();
        let got = read_meta(&exom).unwrap();
        assert_eq!(got.format_version, FORMAT_VERSION);
        assert_eq!(got.kind, ExomKind::ProjectMain);
        assert_eq!(got.created_by, "alice@test");
    }

    #[test]
    fn session_id_format() {
        let id = session_id("20260411T143215Z", SessionType::Multi, "landing-page");
        assert_eq!(id, "20260411T143215Z_multi_agent_landing-page");
    }

    #[test]
    fn iso8601_basic_is_16_chars() {
        let ts = now_iso8601_basic();
        assert_eq!(ts.len(), 16); // YYYYMMDDTHHMMSSZ
        assert!(ts.ends_with('Z'));
    }

    // ---------------------------------------------------------------------
    // default_fork_target — guards against the MCP/HTTP drift fixed
    // 2026-05-02 (regression report at archive/2026-05-02_stress-test-763e40fb/).
    // ---------------------------------------------------------------------

    #[test]
    fn default_fork_target_strips_public_prefix() {
        let d = tempdir().unwrap();
        let target = default_fork_target(
            d.path(),
            "alice@test",
            "public/work/team/proj/main",
            Some("main"),
        )
        .unwrap();
        assert_eq!(target, "alice@test/forked/work/team/proj/main");
    }

    #[test]
    fn default_fork_target_self_fork_strips_self_prefix() {
        let d = tempdir().unwrap();
        let target =
            default_fork_target(d.path(), "alice@test", "alice@test/notes/draft", Some("draft"))
                .unwrap();
        assert_eq!(target, "alice@test/forked/notes/draft");
    }

    #[test]
    fn default_fork_target_preserves_other_email_subpath() {
        let d = tempdir().unwrap();
        let target = default_fork_target(
            d.path(),
            "alice@test",
            "bob@test/shared/notebook",
            Some("notebook"),
        )
        .unwrap();
        assert_eq!(target, "alice@test/forked/bob@test/shared/notebook");
    }

    #[test]
    fn default_fork_target_auto_suffix_on_collision() {
        let d = tempdir().unwrap();
        // Pre-create the default target so the helper has to suffix.
        std::fs::create_dir_all(d.path().join("alice@test/forked/work/wiki")).unwrap();
        let t1 = default_fork_target(
            d.path(),
            "alice@test",
            "public/work/wiki",
            Some("wiki"),
        )
        .unwrap();
        assert_eq!(t1, "alice@test/forked/work/wiki-2");

        // Now both base and -2 are taken; expect -3.
        std::fs::create_dir_all(d.path().join(&t1)).unwrap();
        let t2 = default_fork_target(
            d.path(),
            "alice@test",
            "public/work/wiki",
            Some("wiki"),
        )
        .unwrap();
        assert_eq!(t2, "alice@test/forked/work/wiki-3");
    }
}
