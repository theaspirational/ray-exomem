//! End-to-end test for per-type splay sub-tables (T2).
//!
//! Walks a full Brain -> typed fact tables -> rayforce2 env binding ->
//! rule-backed query -> result table. This is the acceptance gate for
//! declarative health-band derivations: the onboarding rules use
//! `(facts_i64 ?id 'predicate ?v) (< ?v 60)` style clauses, which only
//! resolve correctly when the typed sub-tables are live-bound at query
//! time.
//!
//! The tests intentionally use variable-head rules only. Rule heads
//! that carry bare constants (`(health/water-band 'medium)`) currently
//! trip an upstream rayforce2 issue in `dl_project` — see the FIXME
//! note in `src/auth/routes.rs::health_bootstrap_rules` for the exact
//! restriction.
//!
//! Workaround used here: encode the band value as a bound variable by
//! joining with a seeded EDB `water_band_codes(?band_id, ?w_lo, ?w_hi,
//! ?h_lo, ?h_hi)` that carries the numeric bounds. The derived head is
//! `(health/water-band ?band_id)` — ?band_id comes from the auxiliary
//! EDB row, so it is a variable (not a constant).

use ray_exomem::{
    brain::Brain,
    context::MutationContext,
    fact_value::FactValue,
    rules::parse_rule_line,
    storage, RayforceEngine,
};

fn test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// Build the water-band ruleset using variable-head rules joined against
/// a seeded `water_band_codes` EDB. This sidesteps rayforce2's constant-
/// head evaluation path.
fn water_band_rules(exom: &str) -> Vec<String> {
    vec![
        // Match any band whose bounds straddle the current profile numbers.
        // water_band_codes schema: (band_id, w_lo, w_hi, h_lo, h_hi).
        format!(
            r#"(rule {exom} (health/water-band ?band) (water_band_codes ?band ?wlo ?whi ?hlo ?hhi) (facts_i64 ?w_id 'profile/weight_kg ?w) (facts_i64 ?h_id 'profile/height_cm ?h) (>= ?w ?wlo) (< ?w ?whi) (>= ?h ?hlo) (< ?h ?hhi))"#
        ),
    ]
}

/// Build a 5-column `water_band_codes` table: (band_id, w_lo, w_hi, h_lo, h_hi).
/// band_id is RAY_SYM, the four bounds are RAY_I64. Rows cover the
/// original Rust decision tree:
///   small  = w < 60 and h < 170
///   large  = w >= 85 or h >= 185
///   medium = otherwise (encoded via explicit numeric windows)
fn build_water_band_codes_table() -> storage::RayObj {
    use ray_exomem::ffi;
    unsafe {
        let rows: Vec<(&'static str, i64, i64, i64, i64)> = vec![
            ("small", 0, 60, 0, 170),
            // Medium, two windows chosen so any profile that is neither small
            // nor large matches exactly one row.
            ("medium", 60, 85, 0, 185),
            ("medium", 0, 85, 170, 185),
            ("large", 85, i64::MAX, 0, i64::MAX),
            ("large", 0, i64::MAX, 185, i64::MAX),
        ];
        let tbl = ffi::ray_table_new(5);
        let cap = rows.len() as i64;
        let mut id_col = ffi::ray_vec_new(ffi::RAY_SYM, cap);
        let mut wlo = ffi::ray_vec_new(ffi::RAY_I64, cap);
        let mut whi = ffi::ray_vec_new(ffi::RAY_I64, cap);
        let mut hlo = ffi::ray_vec_new(ffi::RAY_I64, cap);
        let mut hhi = ffi::ray_vec_new(ffi::RAY_I64, cap);
        for (id, a, b, c, d) in &rows {
            let sym = storage::sym_intern(id);
            id_col = ffi::ray_vec_append(id_col, &sym as *const i64 as *const _);
            wlo = ffi::ray_vec_append(wlo, a as *const i64 as *const _);
            whi = ffi::ray_vec_append(whi, b as *const i64 as *const _);
            hlo = ffi::ray_vec_append(hlo, c as *const i64 as *const _);
            hhi = ffi::ray_vec_append(hhi, d as *const i64 as *const _);
        }
        let tbl = ffi::ray_table_add_col(tbl, storage::sym_intern("band_id"), id_col);
        ffi::ray_release(id_col);
        let tbl = ffi::ray_table_add_col(tbl, storage::sym_intern("w_lo"), wlo);
        ffi::ray_release(wlo);
        let tbl = ffi::ray_table_add_col(tbl, storage::sym_intern("w_hi"), whi);
        ffi::ray_release(whi);
        let tbl = ffi::ray_table_add_col(tbl, storage::sym_intern("h_lo"), hlo);
        ffi::ray_release(hlo);
        let tbl = ffi::ray_table_add_col(tbl, storage::sym_intern("h_hi"), hhi);
        ffi::ray_release(hhi);
        storage::RayObj::from_raw(tbl).unwrap()
    }
}

/// Seed typed numeric profile facts and run a query via the rayforce2
/// engine, with `facts_i64` and `water_band_codes` bound under the env
/// names the auto-EDB hook expects.
fn query_first_col(
    exom: &str,
    profile: &[(&str, &str, FactValue)],
    query: &str,
    rules: &[String],
) -> Vec<String> {
    let _guard = match test_lock().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    let engine = RayforceEngine::new().unwrap();
    let mut brain = Brain::new();
    let ctx = MutationContext::default();
    for (fact_id, predicate, value) in profile {
        brain
            .assert_fact(fact_id, predicate, value.clone(), 1.0, "test", None, None, &ctx)
            .unwrap();
    }

    let datoms = storage::build_datoms_table(&brain).unwrap();
    let typed = storage::build_typed_fact_tables(&brain).unwrap();
    let band_codes = build_water_band_codes_table();

    engine
        .bind_named_db(storage::sym_intern(exom), &datoms)
        .unwrap();
    engine
        .bind_named_db(storage::sym_intern(storage::FACTS_I64_ENV), &typed.facts_i64)
        .unwrap();
    engine
        .bind_named_db(storage::sym_intern(storage::FACTS_STR_ENV), &typed.facts_str)
        .unwrap();
    engine
        .bind_named_db(storage::sym_intern(storage::FACTS_SYM_ENV), &typed.facts_sym)
        .unwrap();
    engine
        .bind_named_db(storage::sym_intern("water_band_codes"), &band_codes)
        .unwrap();

    let inline_bodies: Vec<String> = rules
        .iter()
        .map(|r| {
            parse_rule_line(r, ctx.clone(), String::new())
                .expect("rule must parse")
                .inline_body
        })
        .collect();
    let expanded =
        ray_exomem::rayfall_parser::rewrite_query_with_rules(query, &inline_bodies).unwrap();
    let raw = engine.eval_raw(&expanded).unwrap_or_else(|e| {
        panic!(
            "query failed: {}\nexpanded:\n{}\ninline-bodies:\n{:#?}",
            e, expanded, inline_bodies
        );
    });
    // The datalog engine internally converts all cells to bare i64. A
    // column whose rule body bound it to a SYM EDB column (here, the
    // `band_id` column of water_band_codes) comes back as raw intern
    // IDs. Resolve them via the global sym table before returning.
    let mut out = Vec::new();
    unsafe {
        let tbl = raw.as_ptr();
        let ncols = ray_exomem::ffi::ray_table_ncols(tbl);
        let nrows = ray_exomem::ffi::ray_table_nrows(tbl);
        if ncols > 0 {
            let col = ray_exomem::ffi::ray_table_get_col_idx(tbl, 0);
            for r in 0..nrows {
                let v = ray_exomem::ffi::ray_vec_get_i64(col, r);
                if let Ok(name) = storage::sym_lookup(v) {
                    if !name.is_empty() {
                        out.push(name);
                        continue;
                    }
                }
                out.push(v.to_string());
            }
        }
    }
    out
}

#[test]
fn facts_i64_scanned_as_edb_by_rule_body() {
    // Sanity: a rule body atom `(facts_i64 ?e 'attr ?v)` resolves against
    // the auto-registered facts_i64 EDB and the stored i64 value flows
    // through to the bound variable ?v for downstream cmp. Variable-head
    // rule sidesteps the rayforce2 constant-head issue.
    let exom = "dbg/main";
    let profile = &[
        ("w1", "profile/weight_kg", FactValue::I64(55)),
    ];
    let rules = vec![format!(
        r#"(rule {exom} (low-weight-id ?id) (facts_i64 ?id 'profile/weight_kg ?w) (< ?w 60))"#
    )];
    let q = format!("(query {exom} (find ?id) (where (low-weight-id ?id)))");
    let got = query_first_col(exom, profile, &q, &rules);
    assert_eq!(
        got,
        vec!["w1".to_string()],
        "expected low-weight-id to bind ?id=w1 for w=55"
    );
}

#[test]
fn water_band_medium_for_default_profile() {
    let exom = "testexom/health/main";
    let profile = &[
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(75)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(175)),
    ];
    let q = format!("(query {exom} (find ?b) (where (health/water-band ?b)))");
    let got = query_first_col(exom, profile, &q, &water_band_rules(exom));
    assert!(
        got.iter().any(|v| v == "medium" || v == "'medium"),
        "expected water-band medium for default profile (w=75, h=175); got {got:?}"
    );
}

#[test]
fn water_band_small_for_slight_profile() {
    let exom = "testexom/health/main";
    let profile = &[
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(55)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(160)),
    ];
    let q = format!("(query {exom} (find ?b) (where (health/water-band ?b)))");
    let got = query_first_col(exom, profile, &q, &water_band_rules(exom));
    assert!(
        got.iter().any(|v| v == "small" || v == "'small"),
        "expected water-band small for (w=55, h=160); got {got:?}"
    );
}

#[test]
fn water_band_large_for_heavy_profile() {
    let exom = "testexom/health/main";
    let profile = &[
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(90)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(175)),
    ];
    let q = format!("(query {exom} (find ?b) (where (health/water-band ?b)))");
    let got = query_first_col(exom, profile, &q, &water_band_rules(exom));
    assert!(
        got.iter().any(|v| v == "large" || v == "'large"),
        "expected water-band large for (w=90, h=175); got {got:?}"
    );
}
