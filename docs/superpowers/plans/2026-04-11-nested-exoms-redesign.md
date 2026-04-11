# Nested Exoms Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the flat exom namespace with a tree of folders and exoms, introduce path-based addressing, TOFU branch ownership, rework the CLI and HTTP API, and rebuild the Svelte UI around a drawer + focus view model — all per the approved spec at `docs/superpowers/specs/2026-04-11-nested-exoms-redesign-design.md`.

**Architecture:** The daemon owns a tree rooted at `~/.ray-exomem/tree/`. Each directory is either a folder (no `exom.json`) or an exom (has `exom.json` + splay tables). CLI paths use `::` separators; disk + API paths use `/`. Branch ownership is enforced server-side via TOFU on a `branch/claimed_by` attribute. Session metadata (label/closed_at/archived_at) rides on the normal assert path and is mirrored to `exom.json`. The Svelte UI is built on shadcn primitives with the Impeccable skills scheduled at specific plan steps.

**Tech Stack:** Rust (clap for CLI, serde for `exom.json`, existing rayforce2 FFI), Axum-style handlers in `src/web.rs`, Svelte 5 + SvelteKit + shadcn-svelte, vitest / cargo test. Integration tests run against a real daemon.

**Spec reference:** `docs/superpowers/specs/2026-04-11-nested-exoms-redesign-design.md`

**Greenfield posture:** No migration. Users running the old daemon must `rm -rf ~/.ray-exomem` before switching. No compatibility shims. No flat-namespace fallback.

---

## Phase 0 — Reset and scaffold test infrastructure

### Task 0.1: Create `tests/` integration test crate entrypoints

**Files:**
- Create: `tests/common/mod.rs`
- Create: `tests/common/daemon.rs`
- Create: `tests/smoke.rs`

- [ ] **Step 1: Create `tests/common/mod.rs`**

```rust
// Shared test helpers for integration tests.
pub mod daemon;
```

- [ ] **Step 2: Create `tests/common/daemon.rs` with a minimal test harness**

```rust
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct TestDaemon {
    pub data_dir: tempfile::TempDir,
    pub port: u16,
    pub base_url: String,
    child: Child,
}

impl TestDaemon {
    pub fn start() -> Self {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let port = free_port();
        let bin = env!("CARGO_BIN_EXE_ray-exomem");
        let child = Command::new(bin)
            .args(["serve", "--bind", &format!("127.0.0.1:{port}")])
            .env("RAY_EXOMEM_HOME", data_dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon");

        let base_url = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(r) = ureq::get(&format!("{base_url}/api/status")).call() {
                if r.status() == 200 { break; }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        TestDaemon { data_dir, port, base_url, child }
    }

    pub fn tree_root(&self) -> PathBuf {
        self.data_dir.path().join("tree")
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().unwrap().port()
}
```

- [ ] **Step 3: Create `tests/smoke.rs` with a single "daemon comes up" test**

```rust
mod common;
use common::daemon::TestDaemon;

#[test]
fn daemon_starts_and_reports_status() {
    let d = TestDaemon::start();
    let resp = ureq::get(&format!("{}/api/status", d.base_url))
        .call()
        .expect("status");
    assert_eq!(resp.status(), 200);
}
```

- [ ] **Step 4: Add `tempfile` and `ureq` as dev-dependencies in `Cargo.toml`**

```toml
[dev-dependencies]
tempfile = "3"
ureq = { version = "2", features = ["json"] }
```

- [ ] **Step 5: Run smoke test and note the expected failure**

Run: `cargo test --test smoke`
Expected: the test FAILS because the daemon does not yet honour `RAY_EXOMEM_HOME` or the new tree layout. This is the baseline; leave the test in place and move on — it will pass naturally once Phase 2 lands.

- [ ] **Step 6: Commit**

```bash
git add tests/ Cargo.toml
git commit -m "test: add integration harness and baseline smoke test"
```

---

## Phase 1 — Path type and segment validation (pure library code)

### Task 1.1: `TreePath` type with `::`/`/` parsing

**Files:**
- Create: `src/path.rs`
- Modify: `src/lib.rs` (add `pub mod path;`)

- [ ] **Step 1: Create `src/path.rs` with the failing test module first**

```rust
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TreePath {
    segments: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PathError {
    #[error("empty path")]
    Empty,
    #[error("invalid segment {0:?}: {1}")]
    InvalidSegment(String, &'static str),
    #[error("reserved segment {0:?} cannot be used as an exom")]
    ReservedSegment(String),
}

impl TreePath {
    pub fn segments(&self) -> &[String] { &self.segments }
    pub fn is_empty(&self) -> bool { self.segments.is_empty() }
    pub fn len(&self) -> usize { self.segments.len() }

    pub fn last(&self) -> Option<&str> {
        self.segments.last().map(String::as_str)
    }

    pub fn parent(&self) -> Option<TreePath> {
        if self.segments.len() <= 1 { return None; }
        Some(TreePath { segments: self.segments[..self.segments.len() - 1].to_vec() })
    }

    pub fn join(&self, segment: &str) -> Result<TreePath, PathError> {
        validate_segment(segment)?;
        let mut s = self.segments.clone();
        s.push(segment.to_string());
        Ok(TreePath { segments: s })
    }

    pub fn to_disk_path(&self, tree_root: &Path) -> PathBuf {
        let mut p = tree_root.to_path_buf();
        for seg in &self.segments { p.push(seg); }
        p
    }

    pub fn to_cli_string(&self) -> String { self.segments.join("::") }
    pub fn to_slash_string(&self) -> String { self.segments.join("/") }
}

impl std::str::FromStr for TreePath {
    type Err = PathError;
    fn from_str(s: &str) -> Result<Self, PathError> {
        if s.is_empty() { return Err(PathError::Empty); }
        // Accept either separator on input.
        let normalized = s.replace("::", "/");
        let segments: Vec<String> = normalized
            .split('/')
            .filter(|seg| !seg.is_empty())
            .map(String::from)
            .collect();
        if segments.is_empty() { return Err(PathError::Empty); }
        for seg in &segments { validate_segment(seg)?; }
        Ok(TreePath { segments })
    }
}

impl fmt::Display for TreePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_cli_string())
    }
}

fn validate_segment(seg: &str) -> Result<(), PathError> {
    if seg.is_empty() {
        return Err(PathError::InvalidSegment(seg.to_string(), "empty"));
    }
    let mut chars = seg.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphanumeric() || first == '_' || first == '-') {
        return Err(PathError::InvalidSegment(seg.to_string(), "first char must be [_A-Za-z0-9-]"));
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
            return Err(PathError::InvalidSegment(seg.to_string(), "chars must be [_A-Za-z0-9.-]"));
        }
    }
    Ok(())
}

/// Reserved only when used as an EXOM name. Allowed as a folder segment created by `init`.
pub const RESERVED_EXOM_NAMES: &[&str] = &["sessions"];

pub fn ensure_not_reserved_as_exom(name: &str) -> Result<(), PathError> {
    if RESERVED_EXOM_NAMES.contains(&name) {
        return Err(PathError::ReservedSegment(name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn parses_double_colon() {
        let p: TreePath = "work::ath::lynx::orsl::main".parse().unwrap();
        assert_eq!(p.segments(), &["work","ath","lynx","orsl","main"]);
        assert_eq!(p.to_cli_string(), "work::ath::lynx::orsl::main");
        assert_eq!(p.to_slash_string(), "work/ath/lynx/orsl/main");
    }

    #[test]
    fn parses_slash() {
        let p: TreePath = "work/ath/lynx".parse().unwrap();
        assert_eq!(p.segments(), &["work","ath","lynx"]);
    }

    #[test]
    fn empty_is_error() {
        assert_eq!("".parse::<TreePath>().unwrap_err(), PathError::Empty);
    }

    #[test]
    fn whitespace_in_segment_rejected() {
        let err = "work::a b".parse::<TreePath>().unwrap_err();
        matches!(err, PathError::InvalidSegment(_, _));
    }

    #[test]
    fn sessions_is_reserved_for_exoms() {
        assert!(ensure_not_reserved_as_exom("sessions").is_err());
        assert!(ensure_not_reserved_as_exom("main").is_ok());
    }

    #[test]
    fn join_validates_segment() {
        let p = TreePath::from_str("work").unwrap();
        assert!(p.join("ath").is_ok());
        assert!(p.join("bad segment").is_err());
    }

    #[test]
    fn to_disk_path_joins_segments() {
        let p: TreePath = "work::ath".parse().unwrap();
        let root = PathBuf::from("/root/tree");
        assert_eq!(p.to_disk_path(&root), PathBuf::from("/root/tree/work/ath"));
    }
}
```

- [ ] **Step 2: Add `thiserror = "1"` to `[dependencies]` in `Cargo.toml`** (if not already present)

- [ ] **Step 3: Register the module in `src/lib.rs`**

Add near the top of `src/lib.rs`:
```rust
pub mod path;
```

- [ ] **Step 4: Run the unit tests — they must all pass**

Run: `cargo test -p ray-exomem path::tests`
Expected: 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/path.rs src/lib.rs Cargo.toml
git commit -m "feat(path): TreePath type with :: and / parsing + reserved-name check"
```

---

## Phase 2 — Persistence layer (tree walker, scaffolding primitives)

### Task 2.1: Tree root resolution and `RAY_EXOMEM_HOME` support

**Files:**
- Modify: `src/storage.rs`

- [ ] **Step 1: Locate the existing data-dir resolver in `src/storage.rs` and replace it with a version honouring `RAY_EXOMEM_HOME`. Add the new helper `tree_root()` returning `data_dir().join("tree")`.**

Insert near the top of `src/storage.rs`:
```rust
pub fn data_dir() -> std::path::PathBuf {
    if let Ok(custom) = std::env::var("RAY_EXOMEM_HOME") {
        return std::path::PathBuf::from(custom);
    }
    dirs::home_dir()
        .expect("home dir")
        .join(".ray-exomem")
}

pub fn tree_root() -> std::path::PathBuf {
    data_dir().join("tree")
}
```

If the existing module already has a `data_dir()` helper, replace its body with the snippet above rather than adding a duplicate.

- [ ] **Step 2: Grep for every call site using the old flat `~/.ray-exomem/exoms/...` path and change it to go through `tree_root()`**

Run: `rg "exoms/" src --glob '!src/*.md'`
For each hit, replace the literal `exoms/` segment with the value of `tree_root()` joined with a path derived from a `TreePath`. Where a call site still hard-codes a flat exom name (i.e. a `String` field that used to hold a bare `"main"`), mark it with `// FIXME(nested-exoms-task-4.4): path` — Task 4.4 rewrites every such site to accept a path-based parameter. Do NOT attempt to rewrite those call sites in this task; the goal here is only to introduce `tree_root()` and leave path-aware fixes for the HTTP-layer task that already touches them.

- [ ] **Step 3: Add a unit test that `RAY_EXOMEM_HOME` is honoured**

Append to `src/storage.rs` under `#[cfg(test)] mod tests`:
```rust
#[test]
fn tree_root_follows_env() {
    std::env::set_var("RAY_EXOMEM_HOME", "/tmp/ray-exomem-test");
    assert_eq!(tree_root(), std::path::PathBuf::from("/tmp/ray-exomem-test/tree"));
    std::env::remove_var("RAY_EXOMEM_HOME");
}
```

- [ ] **Step 4: Run test**

Run: `cargo test -p ray-exomem tree_root_follows_env`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/storage.rs
git commit -m "feat(storage): tree_root() + RAY_EXOMEM_HOME override"
```

### Task 2.2: Node kind detection (folder vs exom)

**Files:**
- Create: `src/tree.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/tree.rs` with a `NodeKind` enum and detection function**

```rust
use crate::path::TreePath;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Missing,
    Folder,
    Exom,
}

pub fn classify(disk_path: &Path) -> NodeKind {
    if !disk_path.exists() {
        return NodeKind::Missing;
    }
    if disk_path.join("exom.json").exists() {
        NodeKind::Exom
    } else {
        NodeKind::Folder
    }
}

pub fn ensure_folder_path(tree_root: &Path, path: &TreePath) -> std::io::Result<PathBuf> {
    let disk = path.to_disk_path(tree_root);
    std::fs::create_dir_all(&disk)?;
    Ok(disk)
}

/// Walk from tree_root down the path; fail if any intermediate segment is already an EXOM
/// (cannot nest inside exoms).
pub fn check_no_exom_ancestor(tree_root: &Path, path: &TreePath) -> Result<(), String> {
    let mut disk = tree_root.to_path_buf();
    for seg in path.segments() {
        disk.push(seg);
        if classify(&disk) == NodeKind::Exom && Some(seg.as_str()) != path.last() {
            return Err(format!("cannot nest inside exom {}", disk.display()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn missing_is_missing() {
        let d = tempdir().unwrap();
        assert_eq!(classify(&d.path().join("nope")), NodeKind::Missing);
    }

    #[test]
    fn empty_dir_is_folder() {
        let d = tempdir().unwrap();
        fs::create_dir(d.path().join("f")).unwrap();
        assert_eq!(classify(&d.path().join("f")), NodeKind::Folder);
    }

    #[test]
    fn dir_with_exom_json_is_exom() {
        let d = tempdir().unwrap();
        fs::create_dir(d.path().join("e")).unwrap();
        fs::write(d.path().join("e/exom.json"), "{}").unwrap();
        assert_eq!(classify(&d.path().join("e")), NodeKind::Exom);
    }

    #[test]
    fn ancestor_exom_blocks_nesting() {
        let d = tempdir().unwrap();
        fs::create_dir_all(d.path().join("work/ath")).unwrap();
        fs::write(d.path().join("work/ath/exom.json"), "{}").unwrap();
        let p: TreePath = "work::ath::lynx".parse().unwrap();
        assert!(check_no_exom_ancestor(d.path(), &p).is_err());
    }

    #[test]
    fn leaf_exom_is_fine() {
        let d = tempdir().unwrap();
        fs::create_dir_all(d.path().join("work/main")).unwrap();
        fs::write(d.path().join("work/main/exom.json"), "{}").unwrap();
        let p: TreePath = "work::main".parse().unwrap();
        assert!(check_no_exom_ancestor(d.path(), &p).is_ok());
    }
}
```

- [ ] **Step 2: Add to `src/lib.rs`**

```rust
pub mod tree;
```

- [ ] **Step 3: Add `tempfile` as a regular dev-dep if not already (it is, from Phase 0)**

- [ ] **Step 4: Run tests**

Run: `cargo test -p ray-exomem tree::tests`
Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/tree.rs src/lib.rs
git commit -m "feat(tree): NodeKind classifier and ancestor-exom check"
```

### Task 2.3: `ExomMeta` (format_version 2) and `exom.json` read/write

**Files:**
- Modify: `src/exom.rs` (rewrite the metadata type; the file is small — full rewrite is appropriate)

- [ ] **Step 1: Rewrite `src/exom.rs` with the new `ExomMeta` struct**

Replace the file contents with:

```rust
use crate::path::TreePath;
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
    fn iso8601_basic_is_20_chars() {
        let ts = now_iso8601_basic();
        assert_eq!(ts.len(), 16); // YYYYMMDDTHHMMSSZ
        assert!(ts.ends_with('Z'));
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p ray-exomem exom::tests`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/exom.rs
git commit -m "feat(exom): ExomMeta format_version 2 with session metadata"
```

### Task 2.4: Scaffolding primitives (`init`, `exom new`)

**Files:**
- Create: `src/scaffold.rs`
- Modify: `src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/scaffold.rs`

- [ ] **Step 1: Write the failing tests first in `src/scaffold.rs`**

```rust
use crate::exom::{self, ExomKind, ExomMeta};
use crate::path::{ensure_not_reserved_as_exom, TreePath};
use crate::tree::{classify, NodeKind};
use std::io;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("path: {0}")] Path(#[from] crate::path::PathError),
    #[error("io: {0}")] Io(#[from] io::Error),
    #[error("cannot nest inside exom at {0}")] NestInsideExom(String),
    #[error("already exists as {0:?} at {1}")] AlreadyExistsDifferent(NodeKind, String),
}

pub fn init_project(tree_root: &Path, path: &TreePath) -> Result<(), ScaffoldError> {
    crate::tree::check_no_exom_ancestor(tree_root, path)
        .map_err(ScaffoldError::NestInsideExom)?;
    let leaf = path.to_disk_path(tree_root);
    std::fs::create_dir_all(&leaf)?;

    // main exom
    let main_path = leaf.join("main");
    match classify(&main_path) {
        NodeKind::Missing => {
            exom::write_meta(&main_path, &ExomMeta::new_project_main())?;
        }
        NodeKind::Exom => {
            let meta = exom::read_meta(&main_path)?;
            if meta.kind != ExomKind::ProjectMain {
                return Err(ScaffoldError::AlreadyExistsDifferent(NodeKind::Exom, main_path.display().to_string()));
            }
        }
        NodeKind::Folder => {
            return Err(ScaffoldError::AlreadyExistsDifferent(NodeKind::Folder, main_path.display().to_string()));
        }
    }

    // sessions/ folder (empty dir, no metadata)
    let sessions_path = leaf.join("sessions");
    std::fs::create_dir_all(&sessions_path)?;
    Ok(())
}

pub fn new_bare_exom(tree_root: &Path, path: &TreePath) -> Result<(), ScaffoldError> {
    if let Some(last) = path.last() {
        ensure_not_reserved_as_exom(last)?;
    }
    crate::tree::check_no_exom_ancestor(tree_root, path)
        .map_err(ScaffoldError::NestInsideExom)?;
    let disk = path.to_disk_path(tree_root);
    match classify(&disk) {
        NodeKind::Missing => {
            std::fs::create_dir_all(&disk)?;
            exom::write_meta(&disk, &ExomMeta::new_bare())?;
            Ok(())
        }
        NodeKind::Exom => Ok(()), // idempotent
        NodeKind::Folder => Err(ScaffoldError::AlreadyExistsDifferent(NodeKind::Folder, disk.display().to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn tp(s: &str) -> TreePath { s.parse().unwrap() }

    #[test]
    fn init_creates_main_and_sessions() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work::ath::lynx::orsl")).unwrap();
        assert_eq!(classify(&d.path().join("work/ath/lynx/orsl/main")), NodeKind::Exom);
        assert_eq!(classify(&d.path().join("work/ath/lynx/orsl/sessions")), NodeKind::Folder);
    }

    #[test]
    fn init_is_idempotent() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work::ath")).unwrap();
        init_project(d.path(), &tp("work::ath")).unwrap();
    }

    #[test]
    fn projects_nest_freely() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work::ath::lynx::orsl")).unwrap();
        init_project(d.path(), &tp("work::ath")).unwrap();
        assert_eq!(classify(&d.path().join("work/ath/main")), NodeKind::Exom);
        assert_eq!(classify(&d.path().join("work/ath/lynx/orsl/main")), NodeKind::Exom);
    }

    #[test]
    fn cannot_nest_inside_exom() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work")).unwrap(); // work/main is exom
        let err = init_project(d.path(), &tp("work::main::deeper"));
        assert!(matches!(err, Err(ScaffoldError::NestInsideExom(_))));
    }

    #[test]
    fn new_bare_exom_rejects_reserved() {
        let d = tempdir().unwrap();
        assert!(matches!(new_bare_exom(d.path(), &tp("work::sessions")),
                         Err(ScaffoldError::Path(_))));
    }

    #[test]
    fn new_bare_exom_is_idempotent() {
        let d = tempdir().unwrap();
        new_bare_exom(d.path(), &tp("work::ath::notes")).unwrap();
        new_bare_exom(d.path(), &tp("work::ath::notes")).unwrap();
    }
}
```

- [ ] **Step 2: Register in `src/lib.rs`**

```rust
pub mod scaffold;
```

- [ ] **Step 3: Run tests, make all 6 pass**

Run: `cargo test -p ray-exomem scaffold::tests`
Expected: 6 pass.

- [ ] **Step 4: Commit**

```bash
git add src/scaffold.rs src/lib.rs
git commit -m "feat(scaffold): init_project and new_bare_exom with nesting rules"
```

### Task 2.5: Tree walker for `inspect` + `/api/tree`

**Files:**
- Modify: `src/tree.rs`

- [ ] **Step 1: Append a tree-walking struct and function**

```rust
use crate::exom::{self, ExomKind};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TreeNode {
    Folder {
        name: String,
        path: String, // slash form
        children: Vec<TreeNode>,
    },
    Exom {
        name: String,
        path: String,
        exom_kind: ExomKind,
        fact_count: u64,
        current_branch: String,
        last_tx: Option<String>,
        branches: Option<Vec<String>>, // only when requested
        archived: bool,
        closed: bool,
        session: Option<exom::SessionMeta>,
    },
}

pub struct WalkOptions {
    pub depth: Option<usize>,
    pub include_archived: bool,
    pub include_branches: bool,
    pub include_activity: bool,
}

pub fn walk(tree_root: &std::path::Path, start: &crate::path::TreePath, opts: &WalkOptions)
    -> std::io::Result<TreeNode>
{
    let start_disk = start.to_disk_path(tree_root);
    walk_inner(&start_disk, start, 0, opts)
}

fn walk_inner(
    disk: &std::path::Path,
    path: &crate::path::TreePath,
    depth: usize,
    opts: &WalkOptions,
) -> std::io::Result<TreeNode> {
    let name = path.last().unwrap_or("").to_string();
    let slash = path.to_slash_string();
    match classify(disk) {
        NodeKind::Missing => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing")),
        NodeKind::Exom => {
            let meta = exom::read_meta(disk)?;
            let archived = meta.session.as_ref().and_then(|s| s.archived_at.as_ref()).is_some();
            let closed = meta.session.as_ref().and_then(|s| s.closed_at.as_ref()).is_some();
            if archived && !opts.include_archived {
                return Ok(TreeNode::Folder { name, path: slash, children: vec![] });
            }
            // fact_count / last_tx / branches are pulled from splay tables via callback in later tasks;
            // stubbed to zero/None here.
            Ok(TreeNode::Exom {
                name, path: slash,
                exom_kind: meta.kind,
                fact_count: 0,
                current_branch: meta.current_branch,
                last_tx: None,
                branches: if opts.include_branches { Some(vec![]) } else { None },
                archived, closed,
                session: meta.session,
            })
        }
        NodeKind::Folder => {
            let stop = matches!(opts.depth, Some(max) if depth >= max);
            let mut children = vec![];
            if !stop {
                let mut entries: Vec<_> = std::fs::read_dir(disk)?
                    .filter_map(|e| e.ok())
                    .collect();
                entries.sort_by_key(|e| e.file_name());
                for entry in entries {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let sub_path = path.join(&name).map_err(|e|
                        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
                    let sub_disk = entry.path();
                    if sub_disk.is_dir() {
                        children.push(walk_inner(&sub_disk, &sub_path, depth + 1, opts)?);
                    }
                }
            }
            Ok(TreeNode::Folder { name, path: slash, children })
        }
    }
}
```

**Note:** `fact_count`, `last_tx`, `recent_activity`, and per-exom branch lists are stubbed here because they depend on the splay tables. Task 3.4 will route these through a `BrainView` callback.

- [ ] **Step 2: Add a test**

Append to the `tests` module:

```rust
#[test]
fn walks_a_scaffolded_project() {
    let d = tempdir().unwrap();
    crate::scaffold::init_project(d.path(), &"work::ath::lynx::orsl".parse().unwrap()).unwrap();
    let root: crate::path::TreePath = "work".parse().unwrap();
    let node = walk(d.path(), &root, &WalkOptions {
        depth: Some(5),
        include_archived: false,
        include_branches: false,
        include_activity: false,
    }).unwrap();
    // Serialize to confirm the shape round-trips through serde.
    let json = serde_json::to_string(&node).unwrap();
    assert!(json.contains("\"kind\":\"folder\""));
    assert!(json.contains("\"name\":\"main\""));
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ray-exomem tree::tests`
Expected: 6 pass.

- [ ] **Step 4: Commit**

```bash
git add src/tree.rs
git commit -m "feat(tree): walk() returns TreeNode JSON for inspect/api-tree"
```

---

## Phase 3 — Brain layer (scaffolding ops, TOFU, session metadata)

### Task 3.1: Session creation in brain

**Files:**
- Modify: `src/brain.rs`

- [ ] **Step 1: Add a `session_new()` method on `Brain` (or on whatever top-level struct coordinates mutations in `src/brain.rs` — grep for `impl Brain` to find it)**

First, add imports at the top of `brain.rs`:

```rust
use crate::exom::{self, ExomKind, ExomMeta, SessionMeta, SessionType, session_id, now_iso8601_basic};
use crate::path::TreePath;
use crate::tree::{classify, NodeKind};
```

Then, inside `impl Brain` (or whatever the primary mutation struct is called — if there isn't one, create a new free function `pub fn session_new(tree_root, project_path, type, label, actor, agents) -> Result<TreePath, ScaffoldError>` in `src/scaffold.rs` instead):

```rust
pub fn session_new(
    tree_root: &std::path::Path,
    project_path: &TreePath,
    session_type: SessionType,
    label: &str,
    actor: &str,
    agents: &[String],
) -> Result<TreePath, crate::scaffold::ScaffoldError> {
    // Project must exist and have sessions/ folder.
    let project_disk = project_path.to_disk_path(tree_root);
    if classify(&project_disk.join("main")) != NodeKind::Exom {
        return Err(crate::scaffold::ScaffoldError::NestInsideExom(
            format!("project not initialised at {}", project_path)));
    }
    let ts = now_iso8601_basic();
    let dir = session_id(&ts, session_type.clone(), label);
    let session_path = project_path
        .join("sessions")
        .and_then(|p| p.join(&dir))
        .map_err(crate::scaffold::ScaffoldError::Path)?;
    let disk = session_path.to_disk_path(tree_root);
    std::fs::create_dir_all(&disk)?;

    // Build agents list: orchestrator must always be first; dedupe.
    let mut agents_final: Vec<String> = vec![actor.to_string()];
    for a in agents {
        if !agents_final.contains(a) { agents_final.push(a.clone()); }
    }

    let meta = ExomMeta::new_session(SessionMeta {
        session_type,
        label: label.to_string(),
        initiated_by: actor.to_string(),
        agents: agents_final.clone(),
        closed_at: None,
        archived_at: None,
    });
    exom::write_meta(&disk, &meta)?;

    // Pre-create branches (logical records on the branch splay table will be handled
    // by the existing brain branch-creation path in Task 3.2). For now, record the
    // branch names into the session meta only.
    Ok(session_path)
}
```

If the existing code has a different place where branches are persisted to the splay table, call that path for each agent here and TOFU-claim the orchestrator's branch as `main`.

- [ ] **Step 2: Add an inline test using a `tempdir`**

```rust
#[cfg(test)]
mod session_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_session_with_correct_name() {
        let d = tempdir().unwrap();
        crate::scaffold::init_project(d.path(), &"work::ath".parse().unwrap()).unwrap();
        let project: TreePath = "work::ath".parse().unwrap();
        let session = session_new(
            d.path(), &project, SessionType::Multi, "landing",
            "orchestrator", &["agent_a".into(), "agent_b".into()],
        ).unwrap();
        let segs = session.segments();
        assert_eq!(segs[0], "work");
        assert_eq!(segs[1], "ath");
        assert_eq!(segs[2], "sessions");
        assert!(segs[3].ends_with("_multi_agent_landing"));
        let meta = exom::read_meta(&session.to_disk_path(d.path())).unwrap();
        assert_eq!(meta.session.unwrap().agents, vec!["orchestrator", "agent_a", "agent_b"]);
    }
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p ray-exomem session_tests`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/brain.rs
git commit -m "feat(brain): session_new creates exom with agent list + metadata"
```

### Task 3.2: TOFU branch claim on assert-fact

**Files:**
- Modify: `src/brain.rs`
- Modify: `src/system_schema.rs`

- [ ] **Step 1: Add reserved system attributes in `src/system_schema.rs`**

Find the existing `system_attributes` definition. Add:

```rust
// Reserved attributes for branch ownership and session lifecycle.
sys_attrs.push(("branch/claimed_by", "mutable string; TOFU-owner of this branch"));
sys_attrs.push(("session/label",     "mutable string; display label for the session"));
sys_attrs.push(("session/closed_at", "timestamp; non-null => writes rejected"));
sys_attrs.push(("session/archived_at","timestamp; non-null => hidden from default inspect"));
```

- [ ] **Step 2: In `src/brain.rs`, grep for the assert-fact path. Add a precheck function before the write reaches storage.**

```rust
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("no such exom {0}")] NoSuchExom(String),
    #[error("session closed")] SessionClosed,
    #[error("branch {0} not in exom")] BranchMissing(String),
    #[error("branch owned by {0}")] BranchOwned(String),
    #[error("actor required")] ActorRequired,
    #[error("io: {0}")] Io(#[from] std::io::Error),
}

pub fn precheck_write(
    tree_root: &std::path::Path,
    exom_path: &TreePath,
    branch: &str,
    actor: &str,
) -> Result<(), WriteError> {
    if actor.is_empty() { return Err(WriteError::ActorRequired); }
    let disk = exom_path.to_disk_path(tree_root);
    if classify(&disk) != NodeKind::Exom {
        return Err(WriteError::NoSuchExom(exom_path.to_cli_string()));
    }
    let meta = exom::read_meta(&disk)?;
    if let Some(s) = &meta.session {
        if s.closed_at.is_some() { return Err(WriteError::SessionClosed); }
    }
    // Branch existence + ownership: implemented against the existing branch-splay
    // table helpers in brain.rs. Pseudocode, adapt to the real helpers:
    //
    //     let branches = existing_list_branches(&disk)?;
    //     if !branches.iter().any(|b| b.name == branch) {
    //         return Err(WriteError::BranchMissing(branch.into()));
    //     }
    //     match existing_read_claimed_by(&disk, branch)? {
    //         Some(owner) if owner != actor => return Err(WriteError::BranchOwned(owner)),
    //         Some(_) => {}
    //         None => existing_write_claimed_by(&disk, branch, actor)?, // TOFU
    //     }
    Ok(())
}
```

Then wire `precheck_write` into the existing assert-fact entry point so every mutation hits it first. Grep for the current `assert_fact` / `apply_rayfall` public function in `brain.rs` and insert the call at the top.

- [ ] **Step 3: Write tests for each rejection path**

```rust
#[cfg(test)]
mod tofu_tests {
    use super::*;
    use tempfile::tempdir;

    fn tp(s: &str) -> TreePath { s.parse().unwrap() }

    #[test]
    fn rejects_unknown_exom() {
        let d = tempdir().unwrap();
        let err = precheck_write(d.path(), &tp("work::nope"), "main", "me").unwrap_err();
        assert!(matches!(err, WriteError::NoSuchExom(_)));
    }

    #[test]
    fn rejects_missing_actor() {
        let d = tempdir().unwrap();
        crate::scaffold::init_project(d.path(), &tp("work")).unwrap();
        let err = precheck_write(d.path(), &tp("work::main"), "main", "").unwrap_err();
        assert!(matches!(err, WriteError::ActorRequired));
    }

    #[test]
    fn rejects_closed_session() {
        let d = tempdir().unwrap();
        crate::scaffold::init_project(d.path(), &tp("work")).unwrap();
        let session = session_new(d.path(), &tp("work"), SessionType::Single, "x", "me", &[]).unwrap();
        // Mark closed by editing exom.json directly.
        let disk = session.to_disk_path(d.path());
        let mut meta = exom::read_meta(&disk).unwrap();
        meta.session.as_mut().unwrap().closed_at = Some("2026-04-11T00:00:00Z".into());
        exom::write_meta(&disk, &meta).unwrap();
        let err = precheck_write(d.path(), &session, "main", "me").unwrap_err();
        assert!(matches!(err, WriteError::SessionClosed));
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p ray-exomem tofu_tests`
Expected: 3 pass.

- [ ] **Step 5: Commit**

```bash
git add src/brain.rs src/system_schema.rs
git commit -m "feat(brain): precheck_write enforces exom/session/actor invariants"
```

### Task 3.3: Session metadata mirroring to `exom.json`

**Files:**
- Modify: `src/brain.rs`

- [ ] **Step 1: After every successful assert-fact targeting one of the reserved attributes (`session/label`, `session/closed_at`, `session/archived_at`), update `exom.json` on disk.**

Find the assert-fact success path in `brain.rs`. Immediately after commit, add:

```rust
fn mirror_session_meta_to_disk(
    tree_root: &std::path::Path,
    exom_path: &TreePath,
    predicate: &str,
    value: &str,
) -> std::io::Result<()> {
    let disk = exom_path.to_disk_path(tree_root);
    let mut meta = exom::read_meta(&disk)?;
    if let Some(sess) = meta.session.as_mut() {
        match predicate {
            "session/label"       => sess.label = value.to_string(),
            "session/closed_at"   => sess.closed_at = Some(value.to_string()),
            "session/archived_at" => sess.archived_at = Some(value.to_string()),
            _ => return Ok(()),
        }
        exom::write_meta(&disk, &meta)?;
    }
    Ok(())
}
```

Call this after each matching commit.

- [ ] **Step 2: Test — assert the reserved attribute and confirm exom.json is updated**

```rust
#[test]
fn mirroring_updates_exom_json() {
    use tempfile::tempdir;
    let d = tempdir().unwrap();
    crate::scaffold::init_project(d.path(), &"work".parse().unwrap()).unwrap();
    let session = session_new(d.path(), &"work".parse().unwrap(), SessionType::Single, "old", "me", &[]).unwrap();
    mirror_session_meta_to_disk(d.path(), &session, "session/label", "new").unwrap();
    let meta = exom::read_meta(&session.to_disk_path(d.path())).unwrap();
    assert_eq!(meta.session.unwrap().label, "new");
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p ray-exomem mirroring_updates_exom_json`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/brain.rs
git commit -m "feat(brain): mirror session metadata writes into exom.json"
```

### Task 3.4: Wire fact counts and last-tx into `tree::walk`

**Files:**
- Modify: `src/tree.rs`
- Modify: `src/brain.rs` (export a read-only `BrainView` helper)

- [ ] **Step 1: In `brain.rs`, add a free function `read_exom_stats(exom_disk: &Path) -> io::Result<ExomStats>` that reads the existing splay tables and returns `{ fact_count, last_tx, branches }`.**

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ExomStats {
    pub fact_count: u64,
    pub last_tx: Option<String>,
    pub branches: Vec<String>,
}

pub fn read_exom_stats(exom_disk: &std::path::Path) -> std::io::Result<ExomStats> {
    // Use the existing splay-table loaders in brain.rs to compute counts.
    // Grep for the current function that counts facts in an exom and call it here.
    // Return a filled ExomStats.
    unimplemented!("call into existing splay loaders")
}
```

Grep `src/brain.rs` for the current fact-count / branch-list helpers and fill the body. If the splay loaders require a mutable brain, open a read-only view; if not, the function can stay free.

- [ ] **Step 2: In `tree.rs`, replace the stubs in `walk_inner`'s Exom branch with a call to `brain::read_exom_stats`**

```rust
let stats = crate::brain::read_exom_stats(disk).unwrap_or(crate::brain::ExomStats {
    fact_count: 0, last_tx: None, branches: vec![],
});
// ...
Ok(TreeNode::Exom {
    name, path: slash,
    exom_kind: meta.kind,
    fact_count: stats.fact_count,
    current_branch: meta.current_branch,
    last_tx: stats.last_tx,
    branches: if opts.include_branches { Some(stats.branches) } else { None },
    archived, closed,
    session: meta.session,
})
```

- [ ] **Step 3: Integration test — scaffold a project, insert one fact via existing brain API, walk, assert fact_count == 1**

Add to `src/tree.rs` tests:

This test requires a running daemon because it touches the splay tables via `brain::read_exom_stats`. Put it in `tests/walk_stats.rs` instead of an inline unit test:

```rust
mod common;
use common::daemon::TestDaemon;
use serde_json::json;

#[test]
fn walk_reports_fact_count() {
    let d = TestDaemon::start();
    ureq::post(&format!("{}/api/actions/init", d.base_url))
        .send_json(json!({"path":"work"})).unwrap();
    ureq::post(&format!("{}/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom":"work::main","branch":"main","actor":"me",
            "predicate":"note/body","value":"hello",
        })).unwrap();
    let tree: serde_json::Value = ureq::get(&format!("{}/api/tree?path=work/main", d.base_url))
        .call().unwrap().into_json().unwrap();
    assert_eq!(tree["kind"], "exom");
    assert_eq!(tree["fact_count"], 1);
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p ray-exomem walk_reports_fact_count`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/brain.rs src/tree.rs
git commit -m "feat(tree): walk() reports real fact_count + branches"
```

---

## Phase 4 — HTTP API

### Task 4.0: Structured error response helper

**Files:**
- Create: `src/http_error.rs`
- Modify: `src/lib.rs`
- Modify: `src/web.rs`

- [ ] **Step 1: Create `src/http_error.rs`**

```rust
use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub suggestion: Option<String>,
}

impl ApiError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), path: None, actor: None, branch: None, suggestion: None }
    }
    pub fn with_path(mut self, p: impl Into<String>) -> Self { self.path = Some(p.into()); self }
    pub fn with_actor(mut self, a: impl Into<String>) -> Self { self.actor = Some(a.into()); self }
    pub fn with_branch(mut self, b: impl Into<String>) -> Self { self.branch = Some(b.into()); self }
    pub fn with_suggestion(mut self, s: impl Into<String>) -> Self { self.suggestion = Some(s.into()); self }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

/// Convert the internal WriteError into a structured ApiError.
impl From<crate::brain::WriteError> for ApiError {
    fn from(e: crate::brain::WriteError) -> Self {
        use crate::brain::WriteError::*;
        match e {
            NoSuchExom(p) => ApiError::new("no_such_exom", format!("no such exom {p}"))
                .with_path(p.clone())
                .with_suggestion(format!("ray-exomem init {p}")),
            SessionClosed => ApiError::new("session_closed", "session closed")
                .with_suggestion("retract session/closed_at to reopen"),
            BranchMissing(b) => ApiError::new("branch_not_in_exom", format!("branch {b} not in exom"))
                .with_branch(b.clone())
                .with_suggestion(format!("ask orchestrator to run session add-agent --agent {b}")),
            BranchOwned(other) => ApiError::new("branch_owned", format!("branch owned by {other}"))
                .with_suggestion("write to a branch you own, or ask orchestrator to allocate one"),
            ActorRequired => ApiError::new("actor_required", "actor required")
                .with_suggestion("pass --actor <name>"),
            Io(e) => ApiError::new("io", e.to_string()),
        }
    }
}

impl From<crate::scaffold::ScaffoldError> for ApiError {
    fn from(e: crate::scaffold::ScaffoldError) -> Self {
        use crate::scaffold::ScaffoldError::*;
        match e {
            Path(p) => ApiError::new("bad_path", p.to_string()),
            Io(e) => ApiError::new("io", e.to_string()),
            NestInsideExom(msg) => ApiError::new("cannot_nest_inside_exom", msg),
            AlreadyExistsDifferent(_, msg) => ApiError::new("already_exists_different", msg),
        }
    }
}
```

- [ ] **Step 2: Register and adopt**

Add `pub mod http_error;` to `src/lib.rs`. In `src/web.rs`, replace every ad-hoc `error_response(String)` helper with `Err(ApiError::new(...))?` or `(ApiError::from(e)).into_response()`.

- [ ] **Step 3: Unit test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::WriteError;

    #[test]
    fn write_error_maps_to_actor_required() {
        let api: ApiError = WriteError::ActorRequired.into();
        assert_eq!(api.code, "actor_required");
        assert!(api.suggestion.is_some());
    }

    #[test]
    fn write_error_maps_to_no_such_exom() {
        let api: ApiError = WriteError::NoSuchExom("work::ath".into()).into();
        assert_eq!(api.code, "no_such_exom");
        assert_eq!(api.path.as_deref(), Some("work::ath"));
        assert!(api.suggestion.unwrap().contains("init"));
    }
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p ray-exomem http_error::tests`
Expected: 2 pass.

- [ ] **Step 5: Commit**

```bash
git add src/http_error.rs src/lib.rs src/web.rs
git commit -m "feat(api): structured ApiError {code, message, path, actor, branch, suggestion}"
```

### Task 4.1: `GET /api/tree`

**Files:**
- Modify: `src/web.rs`

- [ ] **Step 1: Add a new handler for `GET /api/tree`**

Grep `src/web.rs` for the existing route registration (likely a `router.route("/api/status", ...)` pattern). Add:

```rust
async fn api_tree(Query(q): Query<TreeQuery>) -> impl IntoResponse {
    let tree_root = crate::storage::tree_root();
    let path: crate::path::TreePath = match q.path.as_deref().unwrap_or("").parse() {
        Ok(p) => p,
        Err(_) => {
            // empty path = walk from tree_root itself
            return match crate::tree::walk_root(&tree_root, &default_opts(&q)) {
                Ok(node) => Json(node).into_response(),
                Err(e) => error_response(e.to_string()),
            };
        }
    };
    match crate::tree::walk(&tree_root, &path, &opts_from(&q)) {
        Ok(node) => Json(node).into_response(),
        Err(e) => error_response(e.to_string()),
    }
}

#[derive(Deserialize)]
struct TreeQuery {
    path: Option<String>,
    depth: Option<usize>,
    archived: Option<bool>,
    branches: Option<bool>,
    activity: Option<bool>,
}
```

Also add a `walk_root` helper in `src/tree.rs` that lists the top-level folders when given no path.

- [ ] **Step 2: Register the route**

Find the router builder in `web.rs` (grep for `.route("/api/status"`). Add:

```rust
.route("/api/tree", get(api_tree))
```

- [ ] **Step 3: Integration test**

Create `tests/api_tree.rs`:

```rust
mod common;
use common::daemon::TestDaemon;

#[test]
fn api_tree_empty_root_returns_folder() {
    let d = TestDaemon::start();
    let body: serde_json::Value = ureq::get(&format!("{}/api/tree", d.base_url))
        .call().unwrap().into_json().unwrap();
    assert_eq!(body["kind"], "folder");
}
```

- [ ] **Step 4: Run test**

Run: `cargo test --test api_tree`
Expected: PASS once the daemon boots cleanly on the empty tree.

- [ ] **Step 5: Commit**

```bash
git add src/web.rs src/tree.rs tests/api_tree.rs
git commit -m "feat(api): GET /api/tree returns folder/exom tree"
```

### Task 4.2: `POST /api/actions/init`, `/exom-new`, `/session-new`, `/session-join`, `/branch-create`

**Files:**
- Modify: `src/web.rs`

- [ ] **Step 1: Add handlers for each action, all thin wrappers around the brain + scaffold functions from Phase 2/3**

```rust
#[derive(Deserialize)] struct InitBody { path: String }
#[derive(Deserialize)] struct ExomNewBody { path: String }
#[derive(Deserialize)] struct SessionNewBody {
    project_path: String,
    #[serde(rename = "type")] session_type: String,
    label: String,
    actor: String,
    agents: Option<Vec<String>>,
}
#[derive(Deserialize)] struct SessionJoinBody { session_path: String, actor: String }
#[derive(Deserialize)] struct BranchCreateBody { exom: String, branch: String }

async fn api_init(Json(b): Json<InitBody>) -> impl IntoResponse {
    let path: crate::path::TreePath = match b.path.parse() { Ok(p) => p, Err(e) => return error_response(e.to_string()) };
    match crate::scaffold::init_project(&crate::storage::tree_root(), &path) {
        Ok(()) => Json(json!({"ok": true, "path": path.to_slash_string()})).into_response(),
        Err(e) => error_response(e.to_string()),
    }
}

async fn api_exom_new(Json(b): Json<ExomNewBody>) -> impl IntoResponse { /* similar */ }
async fn api_session_new(Json(b): Json<SessionNewBody>) -> impl IntoResponse { /* similar, calls brain::session_new */ }
async fn api_session_join(Json(b): Json<SessionJoinBody>) -> impl IntoResponse { /* TOFU-claim helper */ }
async fn api_branch_create(Json(b): Json<BranchCreateBody>) -> impl IntoResponse { /* orchestrator check + branch insert */ }
```

Fill in the bodies using the helpers added in Phase 2/3.

Also emit an SSE `tree-changed` event after every successful action; Task 4.5 covers the SSE broadcast channel.

- [ ] **Step 2: Register the routes**

```rust
.route("/api/actions/init", post(api_init))
.route("/api/actions/exom-new", post(api_exom_new))
.route("/api/actions/session-new", post(api_session_new))
.route("/api/actions/session-join", post(api_session_join))
.route("/api/actions/branch-create", post(api_branch_create))
```

- [ ] **Step 3: Integration tests**

Create `tests/api_actions.rs`:

```rust
mod common;
use common::daemon::TestDaemon;
use serde_json::json;

#[test]
fn init_then_session_new_then_tree_contains_session() {
    let d = TestDaemon::start();

    ureq::post(&format!("{}/api/actions/init", d.base_url))
        .send_json(json!({"path": "work::ath"})).unwrap();

    let resp: serde_json::Value = ureq::post(&format!("{}/api/actions/session-new", d.base_url))
        .send_json(json!({
            "project_path": "work::ath",
            "type": "multi",
            "label": "landing",
            "actor": "orchestrator",
            "agents": ["agent_a", "agent_b"],
        })).unwrap().into_json().unwrap();
    let session_path = resp["session_path"].as_str().unwrap().to_string();

    let tree: serde_json::Value = ureq::get(&format!("{}/api/tree?path=work/ath&depth=3", d.base_url))
        .call().unwrap().into_json().unwrap();
    let s = serde_json::to_string(&tree).unwrap();
    assert!(s.contains("_multi_agent_landing"), "tree should contain new session");
    assert!(s.contains(&session_path) || s.contains("landing"));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test api_actions`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/web.rs tests/api_actions.rs
git commit -m "feat(api): init + exom-new + session lifecycle endpoints"
```

### Task 4.3: `POST /api/actions/rename` with SSE broadcast

**Files:**
- Modify: `src/web.rs`
- Modify: `src/tree.rs` (add `rename_last_segment` helper)

- [ ] **Step 1: Add `rename_last_segment` in `src/tree.rs`**

```rust
pub fn rename_last_segment(
    tree_root: &std::path::Path,
    path: &crate::path::TreePath,
    new_segment: &str,
) -> Result<crate::path::TreePath, String> {
    crate::path::validate_segment_pub(new_segment).map_err(|e| e.to_string())?;
    let parent = match path.parent() {
        Some(p) => p,
        None => crate::path::TreePath::root(), // if root-level, parent is the tree_root itself
    };
    let src = path.to_disk_path(tree_root);
    let dst = parent.to_disk_path(tree_root).join(new_segment);
    if dst.exists() { return Err(format!("target already exists: {}", dst.display())); }
    std::fs::rename(&src, &dst).map_err(|e| e.to_string())?;
    let new_path = parent.join(new_segment).map_err(|e| e.to_string())?;
    Ok(new_path)
}
```

Expose `validate_segment` publicly (rename internal `validate_segment` → `pub fn validate_segment_pub`) and add a `TreePath::root()` helper returning an empty-segments path.

Reject rename if `classify(src) == NodeKind::Exom && meta.kind == Session` — session ids are immutable.

- [ ] **Step 2: Handler**

```rust
#[derive(Deserialize)] struct RenameBody { path: String, new_segment: String }

async fn api_rename(
    State(state): State<Arc<AppState>>,
    Json(b): Json<RenameBody>,
) -> impl IntoResponse {
    let path: crate::path::TreePath = match b.path.parse() { Ok(p) => p, Err(e) => return error_response(e.to_string()) };
    // Reject renaming a session exom id.
    let disk = path.to_disk_path(&crate::storage::tree_root());
    if classify(&disk) == NodeKind::Exom {
        if let Ok(meta) = crate::exom::read_meta(&disk) {
            if meta.kind == crate::exom::ExomKind::Session {
                return error_response("cannot rename session id; use session/label".into());
            }
        }
    }
    match crate::tree::rename_last_segment(&crate::storage::tree_root(), &path, &b.new_segment) {
        Ok(new_path) => {
            state.sse.send(SseEvent::TreeChanged).await.ok();
            Json(json!({"ok": true, "new_path": new_path.to_slash_string()})).into_response()
        }
        Err(e) => error_response(e),
    }
}
```

- [ ] **Step 3: Register route and test**

Register: `.route("/api/actions/rename", post(api_rename))`

Create `tests/api_rename.rs`:

```rust
mod common;
use common::daemon::TestDaemon;
use serde_json::json;

#[test]
fn rename_folder_cascades() {
    let d = TestDaemon::start();
    ureq::post(&format!("{}/api/actions/init", d.base_url))
        .send_json(json!({"path":"work::ath"})).unwrap();
    let resp: serde_json::Value = ureq::post(&format!("{}/api/actions/rename", d.base_url))
        .send_json(json!({"path":"work::ath","new_segment":"foo"})).unwrap().into_json().unwrap();
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["new_path"], "work/foo");
    let tree: serde_json::Value = ureq::get(&format!("{}/api/tree?path=work", d.base_url))
        .call().unwrap().into_json().unwrap();
    assert!(serde_json::to_string(&tree).unwrap().contains("\"foo\""));
}

#[test]
fn rename_rejects_session_id() {
    let d = TestDaemon::start();
    ureq::post(&format!("{}/api/actions/init", d.base_url))
        .send_json(json!({"path":"work"})).unwrap();
    let s: serde_json::Value = ureq::post(&format!("{}/api/actions/session-new", d.base_url))
        .send_json(json!({"project_path":"work","type":"single","label":"x","actor":"me"}))
        .unwrap().into_json().unwrap();
    let session_path = s["session_path"].as_str().unwrap().replace('/', "::");
    let err: ureq::Error = ureq::post(&format!("{}/api/actions/rename", d.base_url))
        .send_json(json!({"path":session_path,"new_segment":"y"})).unwrap_err();
    assert!(matches!(err, ureq::Error::Status(400, _)));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test api_rename`
Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git add src/tree.rs src/web.rs src/path.rs tests/api_rename.rs
git commit -m "feat(api): POST /api/actions/rename with session-id protection"
```

### Task 4.4: Update existing endpoints to accept path-based `exom` params

**Files:**
- Modify: `src/web.rs`

- [ ] **Step 1: Grep `src/web.rs` for every occurrence of `exom=` / `"exom"` / `exom: String` in query structs and request bodies. For each, replace the plain-string handling with `TreePath::from_str`.**

Specifically touch (grep to confirm exact line numbers):

- `/api/status` — `exom` query param
- `/api/query` — `exom` in body
- `/api/actions/assert-fact` — `exom`, `actor` (now required), `branch` (optional)
- `/api/actions/eval` — `exom`, `actor` on writes
- `/api/facts/<id>` — `exom` query param
- `/api/explain` — `exom` query param
- `/api/branches` — `exom` query param

For each, replace the body/query deserialization with:
```rust
let exom_path: TreePath = match body.exom.parse() {
    Ok(p) => p,
    Err(e) => return error_response(format!("bad exom path: {}", e)),
};
```

Pass `exom_path` into `brain::precheck_write` before any mutation.

- [ ] **Step 2: Make `GET /api/exoms` return 410 Gone**

```rust
async fn api_exoms_gone() -> impl IntoResponse {
    (StatusCode::GONE, Json(json!({
        "error": "gone",
        "message": "/api/exoms is removed; use /api/tree instead",
    })))
}

// in router builder:
.route("/api/exoms", get(api_exoms_gone))
```

- [ ] **Step 3: Update `/api/status` response**

Add `server.tree_root` and change `storage.exom_path` to report the full path.

- [ ] **Step 4: Integration test**

Append to `tests/api_actions.rs`:

```rust
#[test]
fn assert_fact_requires_actor() {
    let d = TestDaemon::start();
    ureq::post(&format!("{}/api/actions/init", d.base_url))
        .send_json(json!({"path":"work"})).unwrap();
    let err = ureq::post(&format!("{}/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom": "work::main",
            "predicate": "note/body",
            "value": "hello",
        })).unwrap_err();
    // Expect a 400 with actor_required.
    if let ureq::Error::Status(status, r) = err {
        assert_eq!(status, 400);
        let body: serde_json::Value = r.into_json().unwrap();
        assert_eq!(body["code"], "actor_required");
    } else { panic!("expected status error"); }
}

#[test]
fn api_exoms_is_gone() {
    let d = TestDaemon::start();
    let err = ureq::get(&format!("{}/api/exoms", d.base_url)).call().unwrap_err();
    if let ureq::Error::Status(status, _) = err { assert_eq!(status, 410); }
    else { panic!("expected status error"); }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test api_actions`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/web.rs tests/api_actions.rs
git commit -m "feat(api): path-based exom params; 410 on /api/exoms"
```

### Task 4.5: SSE `tree-changed` event

**Files:**
- Modify: `src/web.rs`

- [ ] **Step 1: Locate the existing SSE broadcast channel in `web.rs` (grep for `sse` / `Sse::new` / `broadcast::channel`). Add a `TreeChanged` variant to the event enum.**

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum SseEvent {
    #[serde(rename = "tree-changed")]
    TreeChanged,
    // ...existing variants
}
```

- [ ] **Step 2: Emit the event from every mutating handler**

Inside `api_init`, `api_exom_new`, `api_session_new`, `api_session_join`, `api_branch_create`, `api_rename`, and wherever `assert-fact` commits a change to reserved attributes — call `state.sse.send(SseEvent::TreeChanged).await.ok();`.

- [ ] **Step 3: Integration test**

```rust
#[test]
fn init_emits_tree_changed_sse() {
    // Open SSE, run init, confirm the event arrives within 500ms.
    // Use a channel and a background thread to read the event stream.
}
```

Fill in the test body once the SSE client side is wired up in the UI (a pragmatic shortcut is to confirm via log that the broadcast was sent — if the existing test harness doesn't support SSE, use `hyper`'s streaming client or mark the test `#[ignore]` and hand-test once in Phase 7).

- [ ] **Step 4: Commit**

```bash
git add src/web.rs tests/
git commit -m "feat(api): emit tree-changed SSE on every mutating action"
```

### Task 4.6: `GET /api/guide` serves the doctrine markdown

**Files:**
- Modify: `src/web.rs`
- Modify: `src/agent_guide.rs` (the file already exists — read it first)

- [ ] **Step 1: Read `src/agent_guide.rs` to see its current shape**

Run: `sed -n '1,40p' src/agent_guide.rs`

- [ ] **Step 2: Replace its content with the new doctrine**

Rewrite `src/agent_guide.rs` to contain a single `pub fn doctrine() -> &'static str` returning a `const GUIDE: &str = include_str!("../docs/agent_guide.md");` — then create `docs/agent_guide.md` containing the full guide from spec §10.2.

The guide markdown file should be checked in. Build it from the spec sections verbatim, substituting the literal example paths.

- [ ] **Step 3: Handler**

```rust
async fn api_guide() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        crate::agent_guide::doctrine(),
    )
}

// router:
.route("/api/guide", get(api_guide))
```

- [ ] **Step 4: Test**

```rust
#[test]
fn guide_endpoint_returns_markdown() {
    let d = TestDaemon::start();
    let body = ureq::get(&format!("{}/api/guide", d.base_url))
        .call().unwrap().into_string().unwrap();
    assert!(body.contains("# Ray-exomem agent guide"));
    assert!(body.contains("Branching rules"));
}
```

- [ ] **Step 5: Commit**

```bash
git add src/agent_guide.rs docs/agent_guide.md src/web.rs tests/
git commit -m "feat(api): /api/guide serves the doctrine markdown"
```

---

## Phase 5 — CLI surface

### Task 5.1: `init`, `exom new` subcommands

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Grep `src/main.rs` for the existing clap `Subcommand` enum (or `enum Cli`).**

Add variants:

```rust
#[derive(Subcommand)]
enum Command {
    // ...existing...
    /// Scaffold a project (main exom + sessions/) at the given path.
    Init { path: String },
    /// Create a bare exom at the given path (no scaffolding).
    ExomNew { path: String },
    // ...
}
```

- [ ] **Step 2: Dispatch**

```rust
Command::Init { path } => {
    let tp: TreePath = path.parse()?;
    crate::scaffold::init_project(&crate::storage::tree_root(), &tp)?;
    println!("scaffolded {}", tp);
}
Command::ExomNew { path } => {
    let tp: TreePath = path.parse()?;
    crate::scaffold::new_bare_exom(&crate::storage::tree_root(), &tp)?;
    println!("created bare exom {}", tp);
}
```

- [ ] **Step 3: Integration test — CLI E2E**

Create `tests/cli.rs`:

```rust
mod common;
use common::daemon::TestDaemon;
use std::process::Command;

fn run(d: &TestDaemon, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ray-exomem"))
        .args(args)
        .env("RAY_EXOMEM_HOME", d.data_dir.path())
        .output().unwrap()
}

#[test]
fn init_creates_project_on_disk() {
    let d = TestDaemon::start();
    let out = run(&d, &["init", "work::ath::lynx::orsl"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(d.tree_root().join("work/ath/lynx/orsl/main/exom.json").exists());
    assert!(d.tree_root().join("work/ath/lynx/orsl/sessions").is_dir());
}
```

- [ ] **Step 4: Run**

Run: `cargo test --test cli init_creates_project_on_disk`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs tests/cli.rs
git commit -m "feat(cli): init and exom new subcommands"
```

### Task 5.2: `session` subcommand group

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add nested `session` subcommand with `new | add-agent | join | rename | close | archive`**

```rust
#[derive(Subcommand)]
enum SessionCmd {
    New {
        project_path: String,
        #[arg(long)] name: String,
        #[arg(long, group = "type")] multi: bool,
        #[arg(long, group = "type")] single: bool,
        #[arg(long)] actor: String,
        #[arg(long, value_delimiter = ',')] agents: Vec<String>,
    },
    AddAgent { session_path: String, #[arg(long)] agent: String },
    Join { session_path: String, #[arg(long)] actor: String },
    Rename { session_path: String, #[arg(long)] label: String },
    Close { session_path: String },
    Archive { session_path: String },
}
```

`rename`, `close`, `archive` dispatch to the normal assert-fact path against reserved attributes.
`new`, `add-agent`, `join` hit the HTTP API directly so the running daemon updates its in-memory state.

- [ ] **Step 2: E2E test for `session new`**

```rust
#[test]
fn session_new_creates_session_exom() {
    let d = TestDaemon::start();
    run(&d, &["init", "work::ath"]);
    let out = run(&d, &[
        "session", "new", "work::ath",
        "--multi", "--name", "landing", "--actor", "orchestrator",
        "--agents", "agent_a,agent_b",
    ]);
    assert!(out.status.success());
    let sessions_dir = d.tree_root().join("work/ath/sessions");
    let entries: Vec<_> = std::fs::read_dir(&sessions_dir).unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert!(entries.iter().any(|e| e.ends_with("_multi_agent_landing")));
}
```

- [ ] **Step 3: Commit**

```bash
git add src/main.rs tests/cli.rs
git commit -m "feat(cli): session new/add-agent/join/rename/close/archive"
```

### Task 5.3: `inspect` subcommand

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add the subcommand**

```rust
#[derive(Parser)]
struct InspectCmd {
    path: Option<String>,
    #[arg(long, default_value_t = 2)] depth: usize,
    #[arg(long)] branches: bool,
    #[arg(long)] archived: bool,
    #[arg(long)] json: bool,
}
```

Dispatch: parse path (or empty), call `tree::walk`, render as either pretty tree text or JSON.

- [ ] **Step 2: Pretty renderer**

```rust
fn render_tree(node: &TreeNode, indent: usize, include_branches: bool) -> String {
    // Classic ASCII tree with `├──`, `└──`, per-node stats
}
```

Render example:

```
work/
└── ath/
    └── lynx/
        └── orsl/
            ├── main              (exom, 142 facts, branch: main)
            └── sessions/
                └── 20260411T1432Z_multi_agent_landing-page  (session, 47 facts)
                    branches: main, agent_a, agent_b
```

- [ ] **Step 3: Test**

```rust
#[test]
fn inspect_prints_tree() {
    let d = TestDaemon::start();
    run(&d, &["init", "work::ath::lynx::orsl"]);
    let out = run(&d, &["inspect", "work", "--depth", "4"]);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("orsl/"));
    assert!(s.contains("main"));
}
```

- [ ] **Step 4: Commit**

```bash
git add src/main.rs tests/cli.rs
git commit -m "feat(cli): inspect subcommand with --depth/--branches/--archived/--json"
```

### Task 5.4: `guide` subcommand + `--help` blurb

**Files:**
- Modify: `src/main.rs`
- Modify: `src/agent_guide.rs`

- [ ] **Step 1: Add the subcommand**

```rust
Command::Guide => {
    print!("{}", crate::agent_guide::doctrine());
}
```

- [ ] **Step 2: Add the blurb to clap's top-level help via the `about` / `long_about` attribute on the top `struct Cli`**

```rust
#[derive(Parser)]
#[command(
    name = "ray-exomem",
    long_about = "Ray-exomem persists memory as a tree of folders and exoms.\n\
  Tree:        work/ath/lynx/orsl/main              (the project's main exom)\n\
               work/ath/lynx/orsl/sessions/<id>     (per-session exoms)\n\
  CLI paths:   work::ath::lynx::orsl::main          (`::` == `/`)\n\
  Branches:    per-exom; write only to your own (TOFU + orchestrator-allocated)\n\
  Writes:      always require --actor <name>\n\
  Full agent workflow:   ray-exomem guide\n"
)]
struct Cli { /* ... */ }
```

- [ ] **Step 3: Test**

```rust
#[test]
fn top_help_contains_schema_blurb() {
    let out = Command::new(env!("CARGO_BIN_EXE_ray-exomem"))
        .args(["--help"]).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Tree:"));
    assert!(s.contains("`::` == `/`"));
}

#[test]
fn guide_subcommand_prints_doctrine() {
    let out = Command::new(env!("CARGO_BIN_EXE_ray-exomem"))
        .arg("guide").output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("agent guide"));
    assert!(s.contains("Multi-line commands"));
}
```

- [ ] **Step 4: Commit**

```bash
git add src/main.rs src/agent_guide.rs tests/cli.rs
git commit -m "feat(cli): guide subcommand + --help blurb"
```

### Task 5.5: Rayfall `@file` / `-` stdin support

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add a helper**

```rust
fn read_rayfall_arg(arg: &str) -> anyhow::Result<String> {
    if arg == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        return Ok(s);
    }
    if let Some(path) = arg.strip_prefix('@') {
        return Ok(std::fs::read_to_string(path)?);
    }
    Ok(arg.to_string())
}
```

Apply to every subcommand that takes a rayfall body: `query`, `expand-query`, `eval`. Wire before passing the argument into the API call.

- [ ] **Step 2: Test**

```rust
#[test]
fn query_accepts_at_file() {
    let d = TestDaemon::start();
    run(&d, &["init", "work"]);
    let ray_path = d.data_dir.path().join("q.ray");
    std::fs::write(&ray_path, "(query (facts))").unwrap();
    let arg = format!("@{}", ray_path.display());
    let out = run(&d, &["query", "--exom", "work::main", &arg]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
}
```

- [ ] **Step 3: Commit**

```bash
git add src/main.rs tests/cli.rs
git commit -m "feat(cli): rayfall bodies accept @file and - stdin"
```

---

## Phase 6 — UI foundation (pause for Impeccable:shape)

### Task 6.1: Run `impeccable:shape` to produce the UI design brief

**Files:**
- Create: `docs/superpowers/design-briefs/ui-refactor-shape.md`

- [ ] **Step 1: Invoke the `impeccable:shape` skill**

```
Skill: impeccable:shape
Context: rebuild ray-exomem UI per spec §9 — tree drawer + focus view + session modes + rename modal + guide route.
Grounded in: docs/superpowers/specs/2026-04-11-nested-exoms-redesign-design.md
```

The skill runs a structured discovery and outputs a design brief. Save its result to `docs/superpowers/design-briefs/ui-refactor-shape.md`.

- [ ] **Step 2: Commit the brief**

```bash
git add docs/superpowers/design-briefs/ui-refactor-shape.md
git commit -m "docs(ui): design brief from impeccable:shape for the refactor"
```

### Task 6.2: Install the missing shadcn-svelte primitives

**Files:**
- Modify: `ui/src/lib/components/ui/**` (created by shadcn CLI)
- Modify: `ui/package.json`

- [ ] **Step 1: Invoke the `shadcn` skill**

Ask the `shadcn` skill to add the following components to `ui/`:
- `sheet` (for the drawer)
- `tabs`
- `dialog` (rename modal)
- `command` (command palette)
- `context-menu` (drawer right-click)
- `tree` (drawer tree) — if not present, the shadcn registry has a tree component; otherwise use `collapsible` + hand-rolled tree
- `tooltip`
- `toast` (for rename confirmation + errors)

- [ ] **Step 2: Verify installed**

Run: `ls ui/src/lib/components/ui/`
Expected to see the new directories.

- [ ] **Step 3: Run `npm run check` and fix any type errors**

Run: `cd ui && npm run check`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add ui/
git commit -m "feat(ui): add shadcn primitives for drawer/tabs/dialog/command/context-menu"
```

---

## Phase 7 — UI shell (drawer, top bar, status bar)

### Task 7.1: `+layout.svelte` skeleton with drawer region

**Files:**
- Modify: `ui/src/routes/+layout.svelte`
- Create: `ui/src/lib/Drawer.svelte`
- Create: `ui/src/lib/TopBar.svelte`
- Create: `ui/src/lib/StatusBar.svelte`

- [ ] **Step 1: Invoke `svelte:svelte-file-editor` with the target file — every edit to `.svelte` files in this phase must go through the Svelte skill + MCP autofixer before committing**

Request from the skill: "produce a Svelte 5 `+layout.svelte` containing a `<TopBar />`, a collapsible `<Drawer />` using shadcn `sheet`, a main `{@render children()}` slot, and a `<StatusBar />`. Use runes (`$state`, `$derived`, `$effect`) idiomatically. Fetch docs via the MCP server to confirm APIs."

- [ ] **Step 2: Scaffold `Drawer.svelte`**

Minimal API: expose a `drawer` runes-state (`{ open: boolean, expanded: boolean }`) and render the 32px icon rail when collapsed, a shadcn `sheet` overlay when opened.

- [ ] **Step 3: Scaffold `TopBar.svelte`**

Show breadcrumb segments from the current path, the active branch as a pill, and the actor indicator on the right.

- [ ] **Step 4: Scaffold `StatusBar.svelte`**

Fetch `/api/status` and render daemon health + tree root. Use the existing `exomem.svelte.ts` client.

- [ ] **Step 5: Manual smoke test**

Run in another terminal: `ray-exomem serve --bind 127.0.0.1:9780`
Run: `cd ui && npm run dev`
Open the printed URL. Confirm drawer rail is visible, top bar shows "—" for path, status bar shows "daemon ok".

- [ ] **Step 6: Run `svelte:svelte-file-editor` autofixer on every created file**

- [ ] **Step 7: Commit**

```bash
git add ui/src/routes/+layout.svelte ui/src/lib/Drawer.svelte ui/src/lib/TopBar.svelte ui/src/lib/StatusBar.svelte
git commit -m "feat(ui): layout shell with drawer rail, top bar, status bar"
```

### Task 7.2: Invoke `impeccable:layout` on the shell

- [ ] **Step 1: Run `impeccable:layout` with the shell files as targets**

Feedback from the skill is applied inline to `+layout.svelte`, `Drawer.svelte`, `TopBar.svelte`, `StatusBar.svelte`. The skill's output focuses on spacing, hierarchy, and rhythm.

- [ ] **Step 2: Rerun `npm run check` and manual smoke test**

- [ ] **Step 3: Commit**

```bash
git add ui/src/
git commit -m "polish(ui): apply impeccable:layout pass to shell"
```

---

## Phase 8 — UI tree drawer

### Task 8.1: Client-side tree fetching + rendering

**Files:**
- Modify: `ui/src/lib/exomem.svelte.ts`
- Create: `ui/src/lib/TreeDrawer.svelte`
- Create: `ui/src/lib/path.svelte.ts`

- [ ] **Step 1: `path.svelte.ts` — client-side path util**

```ts
export function toSlash(p: string): string { return p.replaceAll('::', '/'); }
export function toCli(p: string): string   { return p.replaceAll('/', '::'); }
export function segments(p: string): string[] {
  return toSlash(p).split('/').filter(Boolean);
}
export function parent(p: string): string | null {
  const s = segments(p);
  if (s.length <= 1) return null;
  return s.slice(0, -1).join('/');
}
```

- [ ] **Step 2: `exomem.svelte.ts` — add `fetchTree(path, opts)`**

```ts
export type TreeNode =
  | { kind: 'folder'; name: string; path: string; children: TreeNode[] }
  | {
      kind: 'exom'; name: string; path: string;
      exom_kind: 'project_main' | 'session' | 'bare';
      fact_count: number; current_branch: string;
      last_tx: string | null; branches: string[] | null;
      archived: boolean; closed: boolean;
      session: SessionMeta | null;
    };

export async function fetchTree(path: string, opts: { depth?: number; branches?: boolean; archived?: boolean; activity?: boolean } = {}): Promise<TreeNode> {
  const qs = new URLSearchParams();
  if (path) qs.set('path', path);
  if (opts.depth != null) qs.set('depth', String(opts.depth));
  if (opts.branches) qs.set('branches', '1');
  if (opts.archived) qs.set('archived', '1');
  if (opts.activity) qs.set('activity', '1');
  const r = await fetch(`/api/tree?${qs}`);
  if (!r.ok) throw new Error(`tree fetch failed: ${r.status}`);
  return r.json();
}
```

- [ ] **Step 3: `TreeDrawer.svelte`**

Use Svelte 5 runes. Props: `{ currentPath, onNavigate }`. Fetches tree, renders recursively. Each node is a `<button>` (exoms) or a collapsible (`folders`). Color-coded per spec §9.2.

- [ ] **Step 4: Validate via `svelte:svelte-file-editor` autofixer**

- [ ] **Step 5: Manual smoke test**

Scaffold some projects via CLI in the running daemon, refresh the UI, confirm the tree renders.

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/path.svelte.ts ui/src/lib/exomem.svelte.ts ui/src/lib/TreeDrawer.svelte
git commit -m "feat(ui): tree drawer with folder/exom rendering"
```

### Task 8.2: Context menu on drawer nodes

**Files:**
- Modify: `ui/src/lib/TreeDrawer.svelte`

- [ ] **Step 1: Wrap each node in `shadcn-svelte` `context-menu` trigger**

Menu items: `init here`, `exom new`, `session new`, `rename`, `close`, `archive`. Each dispatches a handler prop.

- [ ] **Step 2: Implement `init here` + `exom new` prompts**

Simplest: open a shadcn `dialog` with a text input for the new segment, then call `fetch('/api/actions/init', ...)`.

- [ ] **Step 3: Run the Svelte autofixer**

- [ ] **Step 4: Commit**

```bash
git add ui/src/lib/
git commit -m "feat(ui): drawer context menu for init/exom-new/session-new/rename/close/archive"
```

### Task 8.3: Invoke `impeccable:typeset` on the drawer

- [ ] **Step 1: Apply the typeset pass to TreeDrawer, TopBar breadcrumbs, session-id rendering. Fix fonts, weight, rhythm.**

- [ ] **Step 2: Commit**

```bash
git add ui/src/
git commit -m "polish(ui): impeccable:typeset pass on drawer + breadcrumbs"
```

---

## Phase 9 — UI focus views

### Task 9.1: Route `tree/[...path]/+page.svelte` dispatches to per-kind view

**Files:**
- Create: `ui/src/routes/tree/[...path]/+page.svelte`
- Create: `ui/src/routes/tree/[...path]/+page.ts`

- [ ] **Step 1: `+page.ts` loads the node**

```ts
import type { PageLoad } from './$types';
import { fetchTree } from '$lib/exomem.svelte';

export const load: PageLoad = async ({ params, url }) => {
  const path = params.path ?? '';
  const node = await fetchTree(path, { depth: 1, branches: true, activity: true });
  return { path, node, branch: url.searchParams.get('branch') ?? 'main', mode: url.searchParams.get('mode') ?? 'switcher' };
};
```

- [ ] **Step 2: `+page.svelte` dispatches to `<FolderView>`, `<ExomView>`, `<SessionView>`, or `<ArchivedView>` based on kind**

- [ ] **Step 3: Run Svelte autofixer + manual smoke test**

- [ ] **Step 4: Commit**

```bash
git add ui/src/routes/tree/
git commit -m "feat(ui): tree/[...path] route dispatch by node kind"
```

### Task 9.2: `FolderView.svelte`

**Files:**
- Create: `ui/src/routes/tree/[...path]/FolderView.svelte`

- [ ] **Step 1: Grid of children**

Display children as shadcn `card`s — exoms first, folders after. Each card shows name, kind, fact count (exoms only). Card click navigates via SvelteKit's `goto`.

- [ ] **Step 2: Inline quick-action bar at the top**

Buttons: `init here`, `exom new`, `session new`, each opening a dialog.

- [ ] **Step 3: Run Svelte autofixer + commit**

```bash
git add ui/src/routes/tree/[...path]/FolderView.svelte
git commit -m "feat(ui): FolderView grid with quick actions"
```

### Task 9.3: `ExomView.svelte` (project-main + bare exom)

**Files:**
- Create: `ui/src/routes/tree/[...path]/ExomView.svelte`

- [ ] **Step 1: Header (path, fact count, current branch, kind)**

- [ ] **Step 2: Tabs using shadcn `tabs`: Facts | Branches | History | Graph | Rules**

Initially each tab is a placeholder reusing the logic from today's `/facts`, `/graph`, `/rules` pages. These become tab panels instead of standalone routes.

- [ ] **Step 3: Facts tab — wire to existing fact-fetch code from `exomem.svelte.ts`**

- [ ] **Step 4: Run Svelte autofixer, smoke test, commit**

```bash
git add ui/src/routes/tree/[...path]/ExomView.svelte ui/src/lib/exomem.svelte.ts
git commit -m "feat(ui): ExomView with tabs for project-main and bare exoms"
```

### Task 9.4: `SessionView.svelte` with mode toggle

**Files:**
- Create: `ui/src/routes/tree/[...path]/SessionView.svelte`
- Create: `ui/src/routes/tree/[...path]/session-modes/Switcher.svelte`
- Create: `ui/src/routes/tree/[...path]/session-modes/Kanban.svelte`
- Create: `ui/src/routes/tree/[...path]/session-modes/Timeline.svelte`

- [ ] **Step 1: Extend `ExomView` or create a dedicated `SessionView` that embeds the same tabs plus a mode toggle in the Facts tab header**

Toggle group: `Switcher | Kanban | Timeline`, default `Switcher`. Selected mode stored in the URL query param `mode=` and in a runes-state for persistence across tab switches.

- [ ] **Step 2: Switcher.svelte**

Pills list of branches (using `exom.branches` from the tree node). Clicking a pill sets the active branch. Shows facts for the active branch only.

- [ ] **Step 3: Kanban.svelte**

Flex row with one column per branch. Each column: header (branch name, owner, fact count) + scrollable fact list.

- [ ] **Step 4: Timeline.svelte**

Fetch facts for every branch (separate requests, `Promise.all`), interleave by `tx_at` timestamp, render as color-coded rows. Filter pills at the top toggle branch visibility.

- [ ] **Step 5: Run Svelte autofixer on each file, smoke test against a running daemon with a multi-agent session containing a few facts**

- [ ] **Step 6: Commit**

```bash
git add ui/src/routes/tree/[...path]/SessionView.svelte ui/src/routes/tree/[...path]/session-modes/
git commit -m "feat(ui): SessionView with switcher/kanban/timeline modes"
```

### Task 9.5: `ArchivedView.svelte`

**Files:**
- Create: `ui/src/routes/tree/[...path]/ArchivedView.svelte`

- [ ] **Step 1: Minimal: same as `ExomView` but read-only (all write affordances hidden) and wrapped in a dim styling + "Unarchive" button at the top**

- [ ] **Step 2: Commit**

```bash
git add ui/src/routes/tree/[...path]/ArchivedView.svelte
git commit -m "feat(ui): ArchivedView read-only wrapper"
```

---

## Phase 10 — UI rename modal + session label modal + command palette

### Task 10.1: `RenameModal.svelte`

**Files:**
- Create: `ui/src/lib/RenameModal.svelte`

- [ ] **Step 1: Props**

```ts
type Props = {
  open: boolean;
  path: string;
  onClose: () => void;
  onConfirm: (newSegment: string) => Promise<void>;
};
```

- [ ] **Step 2: On open, fetch `GET /api/tree?path=<path>&activity=true` to find sessions with recent activity**

- [ ] **Step 3: Render the modal per spec §9.4 — list affected descendants + active-session warning + two buttons**

- [ ] **Step 4: `impeccable:clarify` pass on the modal copy**

Invoke `impeccable:clarify` with the modal text: warning wording, button labels, empty-state text.

- [ ] **Step 5: Run Svelte autofixer, wire into drawer context menu**

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/RenameModal.svelte ui/src/lib/TreeDrawer.svelte
git commit -m "feat(ui): RenameModal with active-session warning"
```

### Task 10.2: `SessionLabelModal.svelte`

**Files:**
- Create: `ui/src/lib/SessionLabelModal.svelte`

- [ ] **Step 1: Single text input bound to session label**

On submit, call `POST /api/actions/assert-fact` with predicate `session/label` and the new value. No cascade warning.

- [ ] **Step 2: Commit**

```bash
git add ui/src/lib/SessionLabelModal.svelte
git commit -m "feat(ui): SessionLabelModal for session label-only rename"
```

### Task 10.3: Command palette (`⌘K`)

**Files:**
- Create: `ui/src/lib/CommandPalette.svelte`

- [ ] **Step 1: Wire shadcn `command` primitive to a global ⌘K listener in `+layout.svelte`**

- [ ] **Step 2: Commands**

- `go to <path>` — navigate
- `switch branch <name>` — updates URL `branch=`
- `open guide` — navigates to `/guide`
- `init here` — opens init dialog at current folder
- `rename` — opens rename modal

Fuzzy match through all tree paths.

- [ ] **Step 3: Commit**

```bash
git add ui/src/lib/CommandPalette.svelte ui/src/routes/+layout.svelte
git commit -m "feat(ui): ⌘K command palette for navigation + actions"
```

### Task 10.4: `/guide` route

**Files:**
- Create: `ui/src/routes/guide/+page.svelte`

- [ ] **Step 1: Fetch `/api/guide` and render with a markdown renderer**

Use an existing markdown lib or install `marked`. Must be GFM-compatible.

- [ ] **Step 2: Commit**

```bash
git add ui/src/routes/guide/+page.svelte ui/package.json ui/package-lock.json
git commit -m "feat(ui): /guide route renders agent doctrine"
```

---

## Phase 11 — Retirement of old UI routes

### Task 11.1: Delete deprecated routes

**Files:**
- Delete: `ui/src/routes/exoms/`
- Delete: `ui/src/routes/branches/`
- Delete: `ui/src/routes/dependencies/`
- Modify: `ui/src/routes/+page.svelte` (redirect to `/tree/`)

- [ ] **Step 1: Replace `+page.svelte` with a redirect-only page**

```svelte
<script>
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  onMount(() => goto('/tree/'));
</script>
```

- [ ] **Step 2: `git rm` the deprecated directories**

```bash
git rm -r ui/src/routes/exoms ui/src/routes/branches ui/src/routes/dependencies
```

- [ ] **Step 3: Run `npm run check` — fix any dangling imports**

Run: `cd ui && npm run check`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add ui/src/routes/+page.svelte
git commit -m "refactor(ui): retire flat-exom routes; / redirects to /tree/"
```

---

## Phase 12 — UI polish and hardening

### Task 12.1: `impeccable:harden` pass

- [ ] **Step 1: Invoke the skill with the full UI as context**

Focus areas per spec §9.8: empty states for every view, missing-daemon state, loading skeletons, error toasts on API failures, long-path overflow in the breadcrumb, closed-session banner on `ExomView` / `SessionView`.

- [ ] **Step 2: Apply fixes inline, commit**

```bash
git add ui/src/
git commit -m "polish(ui): impeccable:harden pass — edge cases and empty states"
```

### Task 12.2: `impeccable:polish` pass

- [ ] **Step 1: Invoke**

Final polish on alignment, spacing, consistency, micro-detail.

- [ ] **Step 2: Commit**

```bash
git add ui/src/
git commit -m "polish(ui): impeccable:polish final pass"
```

### Task 12.3: `impeccable:audit` / `impeccable:critique`

- [ ] **Step 1: Run one of these as the final quality check — accessibility, responsive behavior, anti-patterns**

- [ ] **Step 2: Apply any P0/P1 fixes, defer P2/P3 to an "Open items" section in the plan**

- [ ] **Step 3: Commit**

```bash
git add ui/src/
git commit -m "polish(ui): impeccable:audit P0/P1 fixes"
```

---

## Phase 13 — End-to-end integration

### Task 13.1: Full-stack E2E test

**Files:**
- Create: `tests/e2e_full_flow.rs`

- [ ] **Step 1: The E2E flow**

```rust
mod common;
use common::daemon::TestDaemon;
use serde_json::json;

#[test]
fn full_nested_exoms_flow() {
    let d = TestDaemon::start();

    // 1. Init a nested project.
    ureq::post(&format!("{}/api/actions/init", d.base_url))
        .send_json(json!({"path": "work::ath::lynx::orsl"})).unwrap();

    // 2. Start a multi-agent session.
    let s: serde_json::Value = ureq::post(&format!("{}/api/actions/session-new", d.base_url))
        .send_json(json!({
            "project_path": "work::ath::lynx::orsl",
            "type": "multi",
            "label": "landing",
            "actor": "orchestrator",
            "agents": ["agent_a", "agent_b"],
        })).unwrap().into_json().unwrap();
    let session_path = s["session_path"].as_str().unwrap().to_string();

    // 3. agent_a writes to its branch; first write claims it.
    ureq::post(&format!("{}/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom": session_path.replace('/', "::"),
            "branch": "agent_a",
            "actor": "agent_a",
            "predicate": "task/status",
            "value": "in_progress",
        })).unwrap();

    // 4. agent_b cannot write to agent_a's branch.
    let err = ureq::post(&format!("{}/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom": session_path.replace('/', "::"),
            "branch": "agent_a",
            "actor": "agent_b",
            "predicate": "task/status",
            "value": "stolen",
        })).unwrap_err();
    if let ureq::Error::Status(s, _) = err { assert_eq!(s, 400); } else { panic!(); }

    // 5. agent_b writes to its own branch successfully.
    ureq::post(&format!("{}/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom": session_path.replace('/', "::"),
            "branch": "agent_b",
            "actor": "agent_b",
            "predicate": "task/status",
            "value": "done",
        })).unwrap();

    // 6. Close the session; further writes fail.
    ureq::post(&format!("{}/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom": session_path.replace('/', "::"),
            "branch": "main",
            "actor": "orchestrator",
            "predicate": "session/closed_at",
            "value": "2026-04-11T15:00:00Z",
        })).unwrap();
    let err = ureq::post(&format!("{}/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom": session_path.replace('/', "::"),
            "branch": "agent_a",
            "actor": "agent_a",
            "predicate": "note/body",
            "value": "after close",
        })).unwrap_err();
    if let ureq::Error::Status(s, _) = err { assert_eq!(s, 400); } else { panic!(); }

    // 7. Rename the mid-tree folder; verify tree reflects new path.
    ureq::post(&format!("{}/api/actions/rename", d.base_url))
        .send_json(json!({"path":"work::ath::lynx","new_segment":"lynx2"})).unwrap();
    let tree: serde_json::Value = ureq::get(&format!("{}/api/tree?path=work/ath", d.base_url))
        .call().unwrap().into_json().unwrap();
    let s = serde_json::to_string(&tree).unwrap();
    assert!(s.contains("lynx2"));
    assert!(!s.contains("\"name\":\"lynx\""));
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test e2e_full_flow`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e_full_flow.rs
git commit -m "test: end-to-end nested-exoms flow (init → session → TOFU → close → rename)"
```

### Task 13.2: Run full verification suite

- [ ] **Step 1: Cargo tests**

Run: `cargo test`
Expected: green.

- [ ] **Step 2: UI checks**

Run: `cd ui && npm run check && npm run build`
Expected: clean + build succeeds.

- [ ] **Step 3: Manual smoke test of the running daemon**

```bash
cargo build --release
ln -f target/release/ray-exomem ~/.local/bin/ray-exomem
rm -rf ~/.ray-exomem
ray-exomem daemon
ray-exomem init work::ath::lynx::orsl
ray-exomem session new work::ath::lynx::orsl \
  --multi --name landing --actor orchestrator \
  --agents agent_a,agent_b
ray-exomem inspect work --depth 4 --branches
ray-exomem guide | head -40
ray-exomem stop
```

Confirm each command produces the expected output.

- [ ] **Step 4: Open the UI in a browser, exercise the drawer + focus views + session modes + rename modal + command palette + /guide**

- [ ] **Step 5: Commit any last fixes, then done**

```bash
git add -u
git commit -m "chore: final cleanup after E2E verification"
```

---

## Open questions / deferred

- CLI rename parity (spec defers; v1 is UI-only).
- Cross-exom queries.
- Branch merging (`session close` just freezes).
- Remote daemons / multi-host trees.
- A dedicated `ray-exomem doctor` path-walker for orphaned splay tables and stale metadata (probably v1.1).
- SSE event format for `tree-changed` — if the existing channel is JSON, add `path?` and `event?` fields to support finer-grained UI invalidation later.
