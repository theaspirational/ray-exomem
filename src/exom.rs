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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionType { Multi, Single }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMeta {
    #[serde(rename = "type")] pub session_type: SessionType,
    pub label: String,
    pub initiated_by: String,
    pub agents: Vec<String>,
    pub closed_at: Option<String>,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExomMeta {
    pub format_version: u32,
    pub current_branch: String,
    pub kind: ExomKind,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionMeta>,
}

impl ExomMeta {
    pub fn new_project_main() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            current_branch: "main".into(),
            kind: ExomKind::ProjectMain,
            created_at: now_iso8601_basic(),
            session: None,
        }
    }

    pub fn new_bare() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            current_branch: "main".into(),
            kind: ExomKind::Bare,
            created_at: now_iso8601_basic(),
            session: None,
        }
    }

    pub fn new_session(sess: SessionMeta) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            current_branch: "main".into(),
            kind: ExomKind::Session,
            created_at: now_iso8601_basic(),
            session: Some(sess),
        }
    }
}

pub fn meta_path(exom_disk: &Path) -> PathBuf { exom_disk.join(META_FILENAME) }

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
    let tag = match kind { SessionType::Multi => "multi", SessionType::Single => "single" };
    format!("{ts_utc}_{tag}_agent_{label}")
}

pub fn now_iso8601_basic() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap();
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
        if days < dy { break; }
        days -= dy;
        y += 1;
    }
    let mdays = [31u32, if is_leap(y) { 29 } else { 28 }, 31,30,31,30,31,31,30,31,30,31];
    let mut mo = 1u32;
    for &md in &mdays {
        if days < md as i64 { break; }
        days -= md as i64;
        mo += 1;
    }
    (y as u32, mo, days as u32 + 1)
}

fn is_leap(y: i32) -> bool { (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 }

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_and_read_roundtrip() {
        let d = tempdir().unwrap();
        let exom = d.path().join("e");
        let meta = ExomMeta::new_project_main();
        write_meta(&exom, &meta).unwrap();
        let got = read_meta(&exom).unwrap();
        assert_eq!(got.format_version, FORMAT_VERSION);
        assert_eq!(got.kind, ExomKind::ProjectMain);
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
}

// ---------------------------------------------------------------------------
// FIXME(nested-exoms): replaced in later tasks
// ExomDir shim — kept to avoid breaking web.rs until Phase 4 refactors call sites.
// ---------------------------------------------------------------------------

use anyhow::{bail, Context, Result};
use crate::storage;

fn is_rayfall_symbol_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '_' | '.' | '-' | '!' | '?' | '+' | '*' | '/' | '%' | '<' | '>' | '=' | '&' | '|'
            )
    })
}

/// Manages the `~/.ray-exomem/` directory structure.
/// FIXME(nested-exoms): replaced in later tasks
pub struct ExomDir {
    root: PathBuf,
    recovery_mode: bool,
}

impl ExomDir {
    pub fn open(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(root.join("exoms"))
            .with_context(|| format!("failed to create {}/exoms", root.display()))?;

        let mut dir = Self {
            root,
            recovery_mode: false,
        };

        if !storage::sym_load(&dir.sym_path())? {
            eprintln!(
                "[ray-exomem] WARNING: symbol table incompatible (binary upgrade?). \
                 Recovering from JSONL sidecars: {}",
                dir.root.display()
            );
            dir.wipe_sym()?;
            dir.recovery_mode = true;
        }

        Ok(dir)
    }

    pub fn is_recovery_mode(&self) -> bool {
        self.recovery_mode
    }

    fn wipe_sym(&self) -> Result<()> {
        let sym = self.sym_path();
        if sym.exists() {
            std::fs::remove_file(&sym)
                .with_context(|| format!("failed to remove {}", sym.display()))?;
        }
        let sym_lk = self.root.join("sym.lk");
        if sym_lk.exists() {
            let _ = std::fs::remove_file(&sym_lk);
        }
        Ok(())
    }

    pub fn sym_path(&self) -> PathBuf {
        self.root.join("sym")
    }

    pub fn exom_path(&self, name: &str) -> PathBuf {
        self.root.join("exoms").join(name)
    }

    pub fn table_path(&self, exom: &str, table: &str) -> PathBuf {
        self.exom_path(exom).join(table)
    }

    pub fn list_exoms(&self) -> Result<Vec<String>> {
        let exoms_dir = self.root.join("exoms");
        if !exoms_dir.exists() {
            return Ok(Vec::new());
        }

        let mut names = Vec::new();
        for entry in std::fs::read_dir(&exoms_dir)
            .with_context(|| format!("failed to read {}", exoms_dir.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn create_exom(&self, name: &str) -> Result<()> {
        if !is_rayfall_symbol_name(name) {
            bail!("exom '{}' is not a valid bare Rayfall symbol", name);
        }
        let path = self.exom_path(name);
        if path.exists() {
            bail!("exom '{}' already exists", name);
        }
        std::fs::create_dir_all(&path)
            .with_context(|| format!("failed to create exom dir {}", path.display()))?;
        Ok(())
    }

    pub fn delete_exom(&self, name: &str) -> Result<()> {
        let path = self.exom_path(name);
        if !path.exists() {
            bail!("exom '{}' does not exist", name);
        }
        std::fs::remove_dir_all(&path)
            .with_context(|| format!("failed to delete exom dir {}", path.display()))?;
        Ok(())
    }

    pub fn rename_exom(&self, old: &str, new: &str) -> Result<()> {
        if !is_rayfall_symbol_name(new) {
            bail!("exom '{}' is not a valid bare Rayfall symbol", new);
        }
        let old_path = self.exom_path(old);
        let new_path = self.exom_path(new);
        if !old_path.exists() {
            bail!("exom '{}' does not exist", old);
        }
        if new_path.exists() {
            bail!("exom '{}' already exists", new);
        }
        std::fs::rename(&old_path, &new_path)
            .with_context(|| format!("failed to rename {} -> {}", old, new))?;
        Ok(())
    }
}
