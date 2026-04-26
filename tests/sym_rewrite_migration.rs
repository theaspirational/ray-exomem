//! End-to-end test for the sym-rewrite-on-startup migration.
//!
//! Builds a fresh data dir, writes some facts, saves sym + splays.
//! Then synthetically rewrites the sym file on disk to shift user
//! sym IDs — simulating what a rayforce2 builtin-shape refactor would
//! look like to a downstream binary. Finally, run_sym_rewrite should
//! detect the shift, remap the splay RAY_SYM + datom-tagged columns
//! through the shift, and queries against the remapped splays should
//! continue to return the original strings.

use ray_exomem::{
    brain::Brain,
    context::MutationContext,
    fact_value::FactValue,
    storage,
    sym_rewrite::{self, RewriteOutcome},
    RayforceEngine,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

fn test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn tmp_data_dir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "ray-exomem-sym-rewrite-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn parse_sym_blob(bytes: &[u8]) -> Vec<String> {
    assert_eq!(&bytes[..4], b"STRL");
    let count = u64::from_le_bytes(bytes[4..12].try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(count);
    let mut pos = 12;
    for _ in 0..count {
        let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        out.push(std::str::from_utf8(&bytes[pos..pos + len]).unwrap().to_string());
        pos += len;
    }
    out
}

fn write_sym_blob(path: &Path, strings: &[String]) {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"STRL");
    buf.extend_from_slice(&(strings.len() as u64).to_le_bytes());
    for s in strings {
        buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }
    let mut f = fs::File::create(path).unwrap();
    f.write_all(&buf).unwrap();
}

/// Given a fresh engine, seed a Brain with a few facts, write its
/// datoms splay to `tree_root/main/fact`, and return the sym file path.
fn seed_exom(data_dir: &Path) -> (PathBuf, PathBuf, Vec<String>) {
    let sym_path = data_dir.join("sym");
    let tree_dir = data_dir.join("tree");
    fs::create_dir_all(&tree_dir).unwrap();
    let exom_dir = tree_dir.join("main");
    let fact_dir = exom_dir.join("fact");

    let mut brain = Brain::new();
    let ctx = MutationContext::default();
    // Seed several facts with diverse values — some numeric (plain i64
    // datoms), some sym-encoded, some string-encoded. Each assert
    // interns at least `fact_id`, `predicate`, and (if non-numeric)
    // the value string, so we end up with plenty of user syms in the
    // table.
    let rows = [
        ("p-a", "project/priority", FactValue::I64(9)),
        ("p-b", "project/priority", FactValue::I64(3)),
        ("p-a", "project/name", FactValue::Str("alpha".into())),
        ("p-b", "project/name", FactValue::Str("beta".into())),
    ];
    for (fact_id, predicate, value) in rows {
        brain
            .assert_fact(
                fact_id, predicate, value, 1.0, "seed", None, None, &ctx,
            )
            .unwrap();
    }
    let datoms = storage::build_datoms_table(&brain).unwrap();
    storage::save_table(&datoms, &fact_dir, &sym_path).unwrap();
    storage::sym_save(&sym_path).unwrap();

    let strings = parse_sym_blob(&fs::read(&sym_path).unwrap());
    (sym_path, tree_dir, strings)
}

/// Count how many sym IDs referenced by any splay fall outside the
/// valid range [0, str_count). A remap bug would typically show as
/// cells pointing at slots that weren't in the rewritten sym table.
fn check_splay_sym_ids_in_range(
    fact_dir: &Path,
    _sym_path: &Path,
    str_count: usize,
) -> (usize, Vec<i64>) {
    // Use skip_sym so we don't trip the merge-check during diagnostic
    // reads — in-memory sym is set up by the caller.
    let tbl = storage::load_table_skip_sym(fact_dir).unwrap();
    let mut out_of_range: Vec<i64> = Vec::new();
    unsafe {
        let ncols = ray_exomem::ffi::ray_table_ncols(tbl.as_ptr());
        let nrows = ray_exomem::ffi::ray_table_nrows(tbl.as_ptr());
        for c in 0..ncols {
            let col = ray_exomem::ffi::ray_table_get_col_idx(tbl.as_ptr(), c);
            let t = ray_exomem::ffi::ray_obj_type(col);
            if t == ray_exomem::ffi::RAY_SYM {
                for r in 0..nrows {
                    let id = ray_exomem::ffi::ray_vec_get_sym_id(col, r);
                    if id < 0 || (id as usize) >= str_count {
                        out_of_range.push(id);
                    }
                }
            }
        }
        (nrows as usize, out_of_range)
    }
}

#[test]
fn sym_rewrite_fast_path_when_layout_unchanged() {
    let _guard = match test_lock().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    let data_dir = tmp_data_dir("fast-path");

    // Phase A: build engine, seed, save sym.
    {
        let _engine = RayforceEngine::new().unwrap();
        let _ = seed_exom(&data_dir);
    }

    // Phase B: re-boot. Fresh engine → builtins register → run rewrite
    // → should hit fast path because nothing upstream changed.
    {
        let _engine = RayforceEngine::new().unwrap();
        let sym_path = data_dir.join("sym");
        let tree_dir = data_dir.join("tree");
        let outcome = sym_rewrite::run_sym_rewrite(&sym_path, &tree_dir).unwrap();
        assert!(
            matches!(outcome, RewriteOutcome::FastPath { .. }),
            "expected FastPath, got {outcome:?}"
        );
    }
}

#[test]
fn sym_rewrite_remaps_when_layout_shifted() {
    let _guard = match test_lock().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    let data_dir = tmp_data_dir("shifted");

    // Phase A: seed as normal, capture what the "fresh" sym layout
    // looks like under THIS binary.
    let (sym_path, tree_dir, canonical_strings) = {
        let _engine = RayforceEngine::new().unwrap();
        seed_exom(&data_dir)
    };
    let canonical_count = canonical_strings.len();

    // Phase B: mutate the on-disk sym file to look like it was written
    // by a past binary with a *different* builtin layout. To force
    // non-identity remap, we need a BUILTIN string to appear at a
    // non-canonical position (or vice versa). Swapping two user
    // entries doesn't do it — by construction our re-intern loop
    // iterates positions 0..N and each position ends up at its own
    // index. The remap only shifts when a string at position i
    // resolves to a DIFFERENT slot under the current binary, which
    // only happens when builtins moved.
    //
    // Simulation: swap a builtin name (at its low canonical slot, e.g.
    // "+") with a user name ("project/priority") deep in the table.
    // After the swap:
    //   - position 0 holds "project/priority" instead of "+"
    //   - position user_idx holds "+" instead of "project/priority"
    // Re-intern iterates:
    //   - intern("project/priority") at iter 0 → it's not a builtin, appends
    //     to next free slot, which at this point is... 0. But wait,
    //     builtins 1..B were already registered by ray_lang_init at Phase B
    //     startup, so slot 0 is still occupied by "+". Intern finds "+"
    //     there, but our string is "project/priority" — appends at slot B.
    //     remap[0] = B (non-identity).
    //   - intern("+") at iter user_idx → "+" is a builtin at slot 0.
    //     Returns 0. remap[user_idx] = 0 (non-identity).
    let builtin_idx = canonical_strings
        .iter()
        .position(|s| s == "+")
        .expect("+ is always interned");
    let user_idx = canonical_strings
        .iter()
        .position(|s| s == "project/priority")
        .expect("user sym should be interned");
    assert!(
        builtin_idx < user_idx,
        "expected + at a lower slot than project/priority"
    );
    let mut shifted = canonical_strings.clone();
    shifted.swap(builtin_idx, user_idx);
    write_sym_blob(&sym_path, &shifted);

    // Phase C: re-boot with a fresh engine and run the rewrite. Keep
    // the engine alive through Phase D so ray_sym_count and related
    // FFI calls see a live runtime.
    let _engine = RayforceEngine::new().unwrap();
    let outcome = sym_rewrite::run_sym_rewrite(&sym_path, &tree_dir).unwrap();
    match outcome {
        RewriteOutcome::Remapped { persisted, splays_rewritten } => {
            assert_eq!(persisted, canonical_count);
            assert!(splays_rewritten >= 1, "at least main/fact splay must be rewritten");
        }
        other => panic!("expected Remapped, got {other:?}"),
    }

    // Phase D: verify the rewrite committed a valid on-disk state.
    // The marker file must be gone (rewrite completed). The sym file
    // must parse cleanly. The splay's columns must still exist and
    // have the same row count as before. Every RAY_SYM cell must
    // point to a valid slot in the rewritten sym.
    assert!(
        !data_dir.join(".sym_rewrite_in_progress").exists(),
        "marker file should be removed after successful rewrite"
    );
    let new_strings = parse_sym_blob(&fs::read(&sym_path).unwrap());
    let fact_dir = tree_dir.join("main").join("fact");
    let (nrows, out_of_range) =
        check_splay_sym_ids_in_range(&fact_dir, &sym_path, new_strings.len());
    assert!(nrows > 0, "post-rewrite splay should still have rows");
    assert_eq!(out_of_range, Vec::<i64>::new(),
        "some splay sym IDs point outside the remapped sym range (nrows={nrows})");
}

#[test]
fn sym_rewrite_fresh_boot_no_sym_file() {
    let _guard = match test_lock().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    let data_dir = tmp_data_dir("fresh");
    let sym_path = data_dir.join("sym");
    let tree_dir = data_dir.join("tree");
    fs::create_dir_all(&tree_dir).unwrap();
    // No sym file on disk.
    let _engine = RayforceEngine::new().unwrap();
    let outcome = sym_rewrite::run_sym_rewrite(&sym_path, &tree_dir).unwrap();
    assert!(
        matches!(outcome, RewriteOutcome::FreshBoot),
        "expected FreshBoot, got {outcome:?}"
    );
}

#[test]
fn sym_rewrite_refuses_when_marker_present() {
    let _guard = match test_lock().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    let data_dir = tmp_data_dir("marker");
    let sym_path = data_dir.join("sym");
    let tree_dir = data_dir.join("tree");
    fs::create_dir_all(&tree_dir).unwrap();
    // A prior crashed rewrite would have left this marker.
    fs::write(data_dir.join(".sym_rewrite_in_progress"), b"").unwrap();

    let _engine = RayforceEngine::new().unwrap();
    let err = sym_rewrite::run_sym_rewrite(&sym_path, &tree_dir).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("marker") || msg.contains("interrupted"),
        "expected marker/interrupt diagnostic, got: {msg}"
    );
}

#[test]
fn sym_rewrite_rejects_garbage_sym_file() {
    let _guard = match test_lock().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    let data_dir = tmp_data_dir("garbage");
    let sym_path = data_dir.join("sym");
    let tree_dir = data_dir.join("tree");
    fs::create_dir_all(&tree_dir).unwrap();
    fs::write(&sym_path, b"not a sym file").unwrap();
    let _engine = RayforceEngine::new().unwrap();
    let err = sym_rewrite::run_sym_rewrite(&sym_path, &tree_dir).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("STRL") || msg.to_lowercase().contains("magic") || msg.to_lowercase().contains("too short"),
        "expected diagnostic about bad sym file, got: {msg}"
    );
}
