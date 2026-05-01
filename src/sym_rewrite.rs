//! Sym-rewrite-on-startup migration.
//!
//! Decouples the on-disk representation of string identities from the
//! runtime's slot layout. On startup we parse the persisted sym file as
//! raw strings, let `ray_lang_init` register builtins at their canonical
//! slots for the current binary, then re-intern each old string. This
//! builds an `old_id → new_id` remap that we apply to every on-disk
//! splay's RAY_SYM columns and datom-tagged I64 columns.
//!
//! Design doc: `archive/2026-04-24_sym-rewrite-migration/design.md`.
//!
//! Called exactly once at `AppState::from_data_dir` startup, BEFORE
//! `load_tree_exoms_into`. The engine must already exist with builtins
//! registered (i.e. not created via `ray_runtime_create_with_sym` — we
//! specifically do NOT load the old sym up front, because the whole
//! point is to rewrite it under the current binary's canonical slot
//! layout).

use crate::{datom, ffi, storage};
use anyhow::{anyhow, bail, Context, Result};
use std::{
    ffi::CString,
    fs,
    path::{Path, PathBuf},
};

const SYM_MAGIC: &[u8] = b"STRL";

#[derive(Debug)]
pub enum RewriteOutcome {
    /// No sym file existed. First boot — nothing to do.
    FreshBoot,
    /// Old sym file contents round-trip to identical slot layout under
    /// the current binary. Fast path — no on-disk rewrite needed.
    FastPath { persisted: usize },
    /// Sym layout shifted; every splay's RAY_SYM / datom-tagged columns
    /// were rewritten through the remap table.
    Remapped {
        persisted: usize,
        splays_rewritten: usize,
    },
}

const MARKER_NAME: &str = ".sym_rewrite_in_progress";

/// Top-level entry. Safe to call on every boot.
///
/// Preconditions: the engine exists and has already run `ray_lang_init`
/// (implicit in `RayforceEngine::new()`), so builtins occupy their
/// canonical slots for this binary. We do not touch the engine's env
/// — only the sym table and on-disk splays.
///
/// Postconditions: the persisted sym file's contents are round-trip-
/// consistent with the current binary's sym interning; every on-disk
/// splay under `tree_root` references strings through remapped sym
/// IDs that resolve correctly with the new sym table.
///
/// Crash safety: a marker file (`.sym_rewrite_in_progress`) is written
/// before any destructive step and removed after commit. If the marker
/// is present on entry, we refuse to boot and demand operator
/// intervention — because a partially-rewritten state (some splays
/// remapped, others not, sym in either state) cannot be automatically
/// recovered without potentially double-remapping already-migrated
/// splays.
pub fn run_sym_rewrite(sym_path: &Path, tree_root: &Path) -> Result<RewriteOutcome> {
    let marker_path = marker_path(sym_path);
    if marker_path.exists() {
        eprintln!();
        eprintln!("[ray-exomem] ========================================================");
        eprintln!("[ray-exomem] REFUSING TO BOOT: sym-rewrite marker present");
        eprintln!("[ray-exomem]");
        eprintln!("[ray-exomem] Marker: {}", marker_path.display());
        eprintln!("[ray-exomem]");
        eprintln!("[ray-exomem] A previous sym-rewrite migration started but did not");
        eprintln!("[ray-exomem] commit. The on-disk state may be partially migrated —");
        eprintln!("[ray-exomem] some splays rewritten through the new remap, others not.");
        eprintln!("[ray-exomem] Continuing automatically would risk double-remapping");
        eprintln!("[ray-exomem] the already-migrated splays and corrupting data.");
        eprintln!("[ray-exomem]");
        eprintln!("[ray-exomem] Recovery:");
        eprintln!("[ray-exomem]   1. RESTORE FROM BACKUP if available — safest.");
        eprintln!("[ray-exomem]   2. If you accept potential data loss, remove the");
        eprintln!("[ray-exomem]      marker manually and wipe the tree to boot fresh.");
        eprintln!("[ray-exomem] ========================================================");
        eprintln!();
        bail!(
            "sym-rewrite marker present at {} — previous rewrite was interrupted \
             and the on-disk state may be partially migrated. Restore from backup \
             or (if you accept the risk of data loss) delete the marker manually \
             and wipe the tree.",
            marker_path.display()
        );
    }

    if !sym_path.exists() {
        return Ok(RewriteOutcome::FreshBoot);
    }

    let old_strings = parse_sym_file(sym_path)
        .with_context(|| format!("parse sym file {}", sym_path.display()))?;
    let persisted = old_strings.len();

    // Re-intern every old string. Builtins hit existing slots (registered
    // by ray_lang_init before we got here); user strings append.
    let mut remap: Vec<i64> = Vec::with_capacity(persisted);
    for (old_id, s) in old_strings.iter().enumerate() {
        let new_id = unsafe {
            let c = CString::new(s.as_str())
                .with_context(|| format!("sym[{old_id}] contains interior NUL"))?;
            ffi::ray_sym_intern(c.as_ptr(), s.len())
        };
        if new_id < 0 {
            bail!(
                "ray_sym_intern rejected persisted string at slot {old_id} ({:?}); \
                 refusing to continue — would corrupt RAY_SYM columns",
                s
            );
        }
        remap.push(new_id);
    }

    // Fast path: the old file already reflects the current binary's
    // canonical slot layout. Nothing to rewrite.
    let identity = remap.iter().enumerate().all(|(i, &v)| v as usize == i);
    if identity {
        return Ok(RewriteOutcome::FastPath { persisted });
    }

    // Non-identity remap: transition on-disk state. Write the marker
    // BEFORE any destructive step so a crash mid-rewrite is detectable
    // on next boot.
    fs::write(&marker_path, b"")
        .with_context(|| format!("write rewrite marker {}", marker_path.display()))?;

    let splays_rewritten =
        rewrite_all_splays(tree_root, &old_strings, &remap).with_context(|| {
            format!(
                "rewrite splays under {} (marker {} left in place)",
                tree_root.display(),
                marker_path.display()
            )
        })?;

    // Commit the new sym file. We delete the old file first because
    // `ray_sym_save` would otherwise merge-check against its stale
    // contents and reject the save — that check exists to protect
    // against concurrent writers with divergent sym tables, but we
    // are deliberately diverging here.
    if sym_path.exists() {
        fs::remove_file(sym_path)
            .with_context(|| format!("remove stale sym file {}", sym_path.display()))?;
    }
    let lk = sym_path.with_extension("lk");
    if lk.exists() {
        let _ = fs::remove_file(&lk);
    }
    storage::sym_save(sym_path)
        .with_context(|| format!("persist new sym to {}", sym_path.display()))?;

    // Final commit: remove marker. From here on, the on-disk state is
    // consistent and future boots will hit the fast path.
    fs::remove_file(&marker_path)
        .with_context(|| format!("remove rewrite marker {}", marker_path.display()))?;

    Ok(RewriteOutcome::Remapped {
        persisted,
        splays_rewritten,
    })
}

fn marker_path(sym_path: &Path) -> PathBuf {
    let parent = sym_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(MARKER_NAME)
}

/// Parse a rayforce2 sym file (`STRL` magic + u64 count + per-entry
/// `u32 len + len bytes`). Returns strings in disk-position order, so
/// `strings[i]` was sym ID `i` when the file was written.
fn parse_sym_file(path: &Path) -> Result<Vec<String>> {
    let bytes = fs::read(path).with_context(|| format!("read sym file {}", path.display()))?;
    if bytes.len() < 12 {
        bail!("sym file too short ({} bytes)", bytes.len());
    }
    if &bytes[..4] != SYM_MAGIC {
        bail!(
            "sym file has bad magic (expected {:?}, got {:?})",
            SYM_MAGIC,
            &bytes[..4]
        );
    }
    let count = u64::from_le_bytes(bytes[4..12].try_into().unwrap()) as usize;
    let mut strings = Vec::with_capacity(count);
    let mut pos = 12;
    for i in 0..count {
        if pos + 4 > bytes.len() {
            bail!("sym file truncated at entry {i} (missing len field at offset {pos})");
        }
        let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + len > bytes.len() {
            bail!(
                "sym file truncated at entry {i} (body len={len}, available={})",
                bytes.len() - pos
            );
        }
        let body = &bytes[pos..pos + len];
        let s = std::str::from_utf8(body)
            .with_context(|| format!("sym entry {i} is not utf-8"))?
            .to_string();
        strings.push(s);
        pos += len;
    }
    Ok(strings)
}

/// Enumerate every splay subdir under `tree_root` and rewrite each
/// through the remap table. Returns the count of splays actually
/// rewritten.
///
/// Each splay save deliberately passes NULL for the sym path so
/// `ray_sym_save` is not called during this phase — the on-disk sym
/// file is still stale relative to the in-memory layout and the
/// merge-check in `ray_sym_save` would otherwise reject the save.
fn rewrite_all_splays(tree_root: &Path, old_strings: &[String], remap: &[i64]) -> Result<usize> {
    let mut splay_dirs: Vec<PathBuf> = Vec::new();
    collect_splay_dirs(tree_root, &mut splay_dirs);

    let mut rewritten = 0usize;
    for dir in splay_dirs {
        rewrite_splay(&dir, old_strings, remap)
            .with_context(|| format!("remap splay {}", dir.display()))?;
        rewritten += 1;
    }
    Ok(rewritten)
}

fn collect_splay_dirs(current: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(current) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let path = entry.path();
        // Skip staging / backup dirs from a previous crashed swap.
        let ext = path.extension().and_then(|e| e.to_str());
        if matches!(ext, Some("new") | Some("old")) {
            continue;
        }
        // Splay dirs are recognized by their `.d` marker file, written
        // by ray_splay_save.
        if storage::table_exists(&path) {
            out.push(path);
            continue;
        }
        collect_splay_dirs(&path, out);
    }
}

/// Load a splay by parsing `.d` schema directly and reading each named
/// column file via `ray_col_load`. This bypasses `ray_splay_load`'s
/// reliance on in-memory sym resolution for column names, which would
/// fail here because the schema's sym IDs reference the OLD layout
/// (pre-re-intern) while in-memory is the shifted-through-remap layout.
///
/// For each column:
/// - RAY_SYM columns have their cell values passed through `remap`.
/// - RAY_I64 columns have datom-tagged cells remapped (payload only).
/// - Other types are loaded and re-added as-is.
///
/// Column names are recovered by looking up each old schema sym ID in
/// `old_strings` (not in in-memory sym) — so no live sym resolution is
/// needed for names. The NEW table is built with fresh sym IDs via
/// `sym_intern` on those name strings, which produces canonical-layout
/// IDs that match the post-rewrite sym table.
fn rewrite_splay(dir: &Path, old_strings: &[String], remap: &[i64]) -> Result<()> {
    // Parse .d schema file directly. It's a ray_col-saved RAY_I64
    // vector whose values are OLD-layout sym IDs for each column name.
    let old_col_name_ids = parse_d_schema(&dir.join(".d"))
        .with_context(|| format!("parse schema {}/.d", dir.display()))?;

    // Resolve col names via old_strings (NOT in-memory sym — those
    // IDs refer to the OLD layout we parsed from disk).
    let schema_resolved: Vec<String> = old_col_name_ids
        .iter()
        .map(|&id| {
            usize::try_from(id)
                .ok()
                .and_then(|i| old_strings.get(i).cloned())
                .unwrap_or_default()
        })
        .collect();

    // Cross-check: every schema-derived name must exist as a file
    // in the dir. If it doesn't, the sym table has diverged from
    // what the splay was written against (usually from a prior sym
    // wipe). Fall back to the directory listing in that case — the
    // filesystem is the source of truth for which columns exist.
    let schema_ok = schema_resolved
        .iter()
        .all(|n| !n.is_empty() && !n.starts_with('.') && !n.contains('/') && dir.join(n).is_file());
    let col_names: Vec<String> = if schema_ok {
        schema_resolved
    } else {
        // Recovery path: list files directly, filter out control
        // entries (.d, .new, .old). Sort for deterministic order.
        let mut names = Vec::new();
        if let Ok(rd) = fs::read_dir(dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with('.') {
                        continue;
                    }
                    names.push(name.to_string());
                }
            }
        }
        names.sort();
        eprintln!(
            "[sym-rewrite] WARNING: {} has schema IDs that don't resolve to on-disk \
             col files (likely from a prior sym wipe); recovering via directory \
             listing. Columns: {:?}",
            dir.display(),
            names
        );
        names
    };
    let ncols = col_names.len() as i64;

    // Build a new table by loading each column file directly.
    let new_tbl_ptr = unsafe { ffi::ray_table_new(ncols) };
    if new_tbl_ptr.is_null() {
        bail!("ray_table_new failed for {}", dir.display());
    }
    let mut new_tbl_ptr = new_tbl_ptr;
    let raii = PartialTable {
        ptr: &mut new_tbl_ptr,
    };

    for (col_idx, name) in col_names.iter().enumerate() {
        let col_path = dir.join(name);
        let c_col_path = std::ffi::CString::new(col_path.to_str().unwrap_or(""))
            .with_context(|| format!("col path {} has NUL byte", col_path.display()))?;
        let col_raw = unsafe { ffi::ray_col_load(c_col_path.as_ptr()) };
        if col_raw.is_null() {
            bail!("ray_col_load returned null for {}", col_path.display());
        }
        let col_type = unsafe { ffi::ray_obj_type(col_raw) };
        // For vectors, `len` lives at bytes 24-31 of the ray_t header
        // (same union slot as a table's ncols/nrows pair uses). Read
        // it directly — `ray_table_nrows` would return 0 for a plain
        // vector object since it's not a RAY_TABLE.
        let nrows = unsafe { vector_len(col_raw) };

        let new_col = match col_type {
            ffi::RAY_SYM => {
                let out = rebuild_sym_column(col_raw, nrows, remap, dir, col_idx as i64);
                unsafe { ffi::ray_release(col_raw) };
                out?
            }
            ffi::RAY_I64 => {
                let out = rebuild_i64_column(col_raw, nrows, remap);
                unsafe { ffi::ray_release(col_raw) };
                out?
            }
            _ => {
                // Plain column — keep as-is. The new table's add_col
                // takes its own reference, so we release our col_raw
                // after handing it over.
                col_raw
            }
        };

        let new_name_id = storage::sym_intern(name);
        let with_col = unsafe { ffi::ray_table_add_col(*raii.ptr, new_name_id, new_col) };
        unsafe { ffi::ray_release(new_col) };
        if with_col.is_null() {
            bail!(
                "{}: ray_table_add_col failed on col {col_idx} ({})",
                dir.display(),
                name
            );
        }
        *raii.ptr = with_col;
    }

    let final_ptr = *raii.ptr;
    std::mem::forget(raii);
    let new_tbl = storage::RayObj::from_raw(final_ptr)?;
    storage::save_table_skip_sym(&new_tbl, dir)?;
    Ok(())
}

/// Read a splay's `.d` schema file directly and extract the OLD-layout
/// sym IDs for each column name. Format: standard `ray_col` RAY_I64
/// vector (32-byte ray_t header, then i64 values). We only need the
/// values; no sym resolution happens here.
fn parse_d_schema(path: &Path) -> Result<Vec<i64>> {
    let bytes = fs::read(path)?;
    if bytes.len() < 32 {
        bail!(".d too short ({} bytes)", bytes.len());
    }
    // Header layout (from rayforce.h): bytes 18 = type, 19 = attrs,
    // 24-31 = len (u64 / i64 / same field depending on interpretation).
    let type_byte = bytes[18] as i8;
    if type_byte != ffi::RAY_I64 {
        bail!(
            ".d has unexpected type {} (expected RAY_I64={})",
            type_byte,
            ffi::RAY_I64
        );
    }
    let len = i64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
    let data_off = 32;
    if bytes.len() < data_off + 8 * len {
        bail!(
            ".d truncated: header says len={}, file has {} data bytes",
            len,
            bytes.len() - data_off
        );
    }
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let off = data_off + 8 * i;
        out.push(i64::from_le_bytes(bytes[off..off + 8].try_into().unwrap()));
    }
    Ok(out)
}

/// Read a vector's `len` field from its ray_t header. Vectors store
/// length at bytes 24-31 of the header (the `len` member of the union).
unsafe fn vector_len(v: *mut ffi::ray_t) -> i64 {
    if v.is_null() {
        return 0;
    }
    let ptr = v.cast::<u8>().add(24).cast::<i64>();
    *ptr
}

/// Build a fresh RAY_SYM column with every slot's sym ID remapped.
/// Width (W8/W16/W32/i64) is re-chosen by rayforce2 internally as
/// `ray_vec_append` may widen when a new id exceeds the current cap.
fn rebuild_sym_column(
    old: *mut ffi::ray_t,
    nrows: i64,
    remap: &[i64],
    dir: &Path,
    col_idx: i64,
) -> Result<*mut ffi::ray_t> {
    unsafe {
        let new_col_raw = ffi::ray_vec_new(ffi::RAY_SYM, nrows.max(1));
        if new_col_raw.is_null() {
            bail!("ray_vec_new(RAY_SYM, {nrows}) failed");
        }
        let mut col = new_col_raw;
        for row in 0..nrows {
            let old_id = ffi::ray_vec_get_sym_id(old, row);
            let new_id = remap_one(old_id, remap).with_context(|| {
                format!(
                    "{}: col {col_idx} row {row} sym id {old_id} out of remap range ({})",
                    dir.display(),
                    remap.len()
                )
            })?;
            let val_ptr: *const i64 = &new_id;
            col = ffi::ray_vec_append(col, val_ptr.cast());
            if col.is_null() {
                bail!("ray_vec_append (SYM) returned null at row {row}");
            }
            if ffi::ray_vec_is_null(old, row) {
                ffi::ray_vec_set_null(col, row, true);
            }
        }
        Ok(col)
    }
}

/// Build a fresh RAY_I64 column. Each cell is passed through the
/// datom-aware remap: plain i64 values (kind I64) are copied as-is;
/// SYM/STR-tagged values have their payload sym ID remapped and the tag
/// preserved.
fn rebuild_i64_column(old: *mut ffi::ray_t, nrows: i64, remap: &[i64]) -> Result<*mut ffi::ray_t> {
    unsafe {
        let new_col_raw = ffi::ray_vec_new(ffi::RAY_I64, nrows.max(1));
        if new_col_raw.is_null() {
            bail!("ray_vec_new(RAY_I64, {nrows}) failed");
        }
        let mut col = new_col_raw;
        for row in 0..nrows {
            let old_v = ffi::ray_vec_get_i64(old, row);
            let new_v =
                remap_datom_i64(old_v, remap).with_context(|| format!("row {row} datom remap"))?;
            let val_ptr: *const i64 = &new_v;
            col = ffi::ray_vec_append(col, val_ptr.cast());
            if col.is_null() {
                bail!("ray_vec_append (I64) returned null at row {row}");
            }
            if ffi::ray_vec_is_null(old, row) {
                ffi::ray_vec_set_null(col, row, true);
            }
        }
        Ok(col)
    }
}

/// Apply the datom-tag-aware remap to a single i64 cell.
/// Plain / untagged i64 values pass through. Tagged values have their
/// payload sym ID remapped; the tag bits are preserved.
fn remap_datom_i64(encoded: i64, remap: &[i64]) -> Result<i64> {
    let kind = datom::kind(encoded);
    if kind == datom::KIND_I64 {
        return Ok(encoded);
    }
    let old_sym = datom::payload(encoded);
    let new_sym = remap_one(old_sym, remap).with_context(|| {
        format!(
            "datom-tagged i64 ({:#x}) references out-of-range sym id",
            encoded
        )
    })?;
    Ok(match kind {
        datom::KIND_SYM => datom::encode_sym(new_sym),
        datom::KIND_STR => datom::encode_str(new_sym),
        _ => unreachable!("datom::kind() returns only I64/SYM/STR"),
    })
}

fn remap_one(old_id: i64, remap: &[i64]) -> Result<i64> {
    let idx = usize::try_from(old_id).map_err(|_| anyhow!("negative sym id {old_id}"))?;
    remap
        .get(idx)
        .copied()
        .ok_or_else(|| anyhow!("sym id {old_id} beyond persisted range {}", remap.len()))
}

/// RAII drop-guard for a partially-built table that hasn't been wrapped
/// in a RayObj yet. Used so `?` early-returns don't leak the table.
struct PartialTable<'a> {
    ptr: &'a mut *mut ffi::ray_t,
}

impl<'a> Drop for PartialTable<'a> {
    fn drop(&mut self) {
        unsafe {
            if !(*self.ptr).is_null() {
                ffi::ray_release(*self.ptr);
            }
        }
    }
}
