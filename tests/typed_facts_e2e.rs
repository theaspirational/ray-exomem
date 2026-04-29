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
    // The datalog engine returns columns with real rayforce types. For
    // RAY_SYM columns (e.g. rule-head constant `(health/water-band "medium")`
    // produces a RAY_SYM column of the interned "medium" id), use
    // `ray_vec_get_sym_id` and resolve via the global sym table. For
    // RAY_I64 columns (e.g. integers or datom-tagged ids), fall back to
    // `ray_vec_get_i64` and attempt `sym_lookup` for datom-tagged ids.
    let mut out = Vec::new();
    unsafe {
        let tbl = raw.as_ptr();
        let ncols = ray_exomem::ffi::ray_table_ncols(tbl);
        let nrows = ray_exomem::ffi::ray_table_nrows(tbl);
        if ncols > 0 {
            let col = ray_exomem::ffi::ray_table_get_col_idx(tbl, 0);
            let col_type = ray_exomem::ffi::ray_obj_type(col);
            for r in 0..nrows {
                if col_type == ray_exomem::ffi::RAY_SYM {
                    let sid = ray_exomem::ffi::ray_vec_get_sym_id(col, r);
                    if let Ok(name) = storage::sym_lookup(sid) {
                        if !name.is_empty() {
                            out.push(name);
                            continue;
                        }
                    }
                    out.push(sid.to_string());
                } else {
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

// ---------------------------------------------------------------------------
// Plan-shape rule set (B2 + B3) — exercised end-to-end. rayforce2 ships
// constant-head rule support (commit 862846e on feature/datalog-aggregates)
// but a stratifiable version of B2 replaces the plan's `(not ...)` clauses
// with their positive-body equivalents, because a `medium` rule that negates
// the same predicate it writes to (`health/water-band`) is not
// stratification-safe under rayforce2's current semi-naive evaluator. The
// positive-body encoding is semantically identical: the complement of
// (w<60 AND h<170) is (w>=60 OR h>=170), and the complement of
// (w>=85 OR h>=185) is (w<85 AND h<185). See `health_bootstrap_rules`
// for the mirror copy shipped in the live bootstrap.
// ---------------------------------------------------------------------------

fn plan_verbatim_health_rules(exom: &str) -> Vec<String> {
    // CRITICAL: rule ORDER matters. rayforce2's IDB column-type alignment
    // pins types to the first rule that declares them. A rec rule whose
    // body references `(health/step-band "medium")` must NOT be declared
    // before the step-band derivation rules — otherwise an IDB for
    // `health/step-band` is created with a legacy RAY_I64 column, the
    // later RAY_SYM-headed step-band rules' rows "leak" into neighboring
    // rules (seen as `["medium","high"]` where only `"medium"` should
    // fire). Derive bands FIRST, then compose rec rules on top.
    vec![
        // water-band = small  :-  w < 60 AND h < 170
        format!(
            r#"(rule {exom} (health/water-band "small") (facts_i64 ?w_id 'profile/weight_kg ?w) (facts_i64 ?h_id 'profile/height_cm ?h) (< ?w 60) (< ?h 170))"#
        ),
        // water-band = large  :-  w >= 85
        format!(
            r#"(rule {exom} (health/water-band "large") (facts_i64 ?w_id 'profile/weight_kg ?w) (>= ?w 85))"#
        ),
        // water-band = large  :-  h >= 185
        format!(
            r#"(rule {exom} (health/water-band "large") (facts_i64 ?h_id 'profile/height_cm ?h) (>= ?h 185))"#
        ),
        // water-band = medium  :-  (w >= 60) AND (w < 85) AND (h < 185)
        // (Positive-body encoding of "not small AND not large" that the
        // plan writes with `(not ...)` clauses. A literal negation over
        // the same predicate is not stratification-safe in rayforce2.)
        format!(
            r#"(rule {exom} (health/water-band "medium") (facts_i64 ?w_id 'profile/weight_kg ?w) (facts_i64 ?h_id 'profile/height_cm ?h) (>= ?w 60) (< ?w 85) (< ?h 185))"#
        ),
        // water-band = medium  :-  (h >= 170) AND (w < 85) AND (h < 185)
        // (Second disjunct of the complement.)
        format!(
            r#"(rule {exom} (health/water-band "medium") (facts_i64 ?w_id 'profile/weight_kg ?w) (facts_i64 ?h_id 'profile/height_cm ?h) (>= ?h 170) (< ?w 85) (< ?h 185))"#
        ),

        format!(
            r#"(rule {exom} (health/step-band "high") (facts_i64 ?id 'profile/age ?a) (< ?a 30))"#
        ),
        format!(
            r#"(rule {exom} (health/step-band "medium") (facts_i64 ?id 'profile/age ?a) (>= ?a 30) (< ?a 50))"#
        ),
        format!(
            r#"(rule {exom} (health/step-band "gentle") (facts_i64 ?id 'profile/age ?a) (>= ?a 50))"#
        ),

        // Composable recommended-* rules must come AFTER band derivation
        // rules (see note above). These are trivial joins onto the
        // derived band IDBs.
        format!(r#"(rule {exom} (health/recommended-water-ml "2000") (health/water-band "small"))"#),
        format!(r#"(rule {exom} (health/recommended-water-ml "2500") (health/water-band "medium"))"#),
        format!(r#"(rule {exom} (health/recommended-water-ml "3000") (health/water-band "large"))"#),
        format!(r#"(rule {exom} (health/recommended-steps-per-day "10000") (health/step-band "high"))"#),
        format!(r#"(rule {exom} (health/recommended-steps-per-day "9000") (health/step-band "medium"))"#),
        format!(r#"(rule {exom} (health/recommended-steps-per-day "7500") (health/step-band "gentle"))"#),
    ]
}

#[test]
fn plan_verbatim_water_band_medium_for_default_profile() {
    let exom = "plan/health/main";
    let profile = &[
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(75)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(175)),
        ("health/profile/age", "profile/age", FactValue::I64(30)),
    ];
    let q = format!("(query {exom} (find ?b) (where (health/water-band ?b)))");
    let got = query_first_col(exom, profile, &q, &plan_verbatim_health_rules(exom));
    assert_eq!(
        got,
        vec!["medium".to_string()],
        "expected ONLY 'medium' for (w=75,h=175); got {got:?}"
    );
}

#[test]
fn plan_verbatim_water_band_small_for_slight_profile() {
    let exom = "plan/health/main";
    let profile = &[
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(55)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(160)),
        ("health/profile/age", "profile/age", FactValue::I64(25)),
    ];
    let q = format!("(query {exom} (find ?b) (where (health/water-band ?b)))");
    let got = query_first_col(exom, profile, &q, &plan_verbatim_health_rules(exom));
    assert_eq!(
        got,
        vec!["small".to_string()],
        "expected ONLY 'small' for (w=55,h=160); got {got:?}"
    );
}

#[test]
fn plan_verbatim_water_band_large_for_heavy_profile() {
    let exom = "plan/health/main";
    let profile = &[
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(90)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(175)),
        ("health/profile/age", "profile/age", FactValue::I64(30)),
    ];
    let q = format!("(query {exom} (find ?b) (where (health/water-band ?b)))");
    let got = query_first_col(exom, profile, &q, &plan_verbatim_health_rules(exom));
    assert_eq!(
        got,
        vec!["large".to_string()],
        "expected ONLY 'large' for (w=90,h=175); got {got:?}"
    );
}

#[test]
fn plan_verbatim_step_band_high_young() {
    let exom = "plan/health/main";
    let profile = &[
        ("health/profile/age", "profile/age", FactValue::I64(25)),
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(75)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(175)),
    ];
    let q = format!("(query {exom} (find ?b) (where (health/step-band ?b)))");
    let got = query_first_col(exom, profile, &q, &plan_verbatim_health_rules(exom));
    assert_eq!(
        got,
        vec!["high".to_string()],
        "expected ONLY 'high' for age=25; got {got:?}"
    );
}

#[test]
fn plan_verbatim_step_band_medium_default() {
    let exom = "plan/health/main";
    let profile = &[
        ("health/profile/age", "profile/age", FactValue::I64(30)),
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(75)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(175)),
    ];
    let q = format!("(query {exom} (find ?b) (where (health/step-band ?b)))");
    let got = query_first_col(exom, profile, &q, &plan_verbatim_health_rules(exom));
    assert_eq!(
        got,
        vec!["medium".to_string()],
        "expected ONLY 'medium' for age=30; got {got:?}"
    );
}

#[test]
fn plan_verbatim_step_band_gentle_older() {
    let exom = "plan/health/main";
    let profile = &[
        ("health/profile/age", "profile/age", FactValue::I64(55)),
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(75)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(175)),
    ];
    let q = format!("(query {exom} (find ?b) (where (health/step-band ?b)))");
    let got = query_first_col(exom, profile, &q, &plan_verbatim_health_rules(exom));
    assert_eq!(
        got,
        vec!["gentle".to_string()],
        "expected ONLY 'gentle' for age=55; got {got:?}"
    );
}

#[test]
fn plan_verbatim_recommended_water_ml_default() {
    let exom = "plan/health/main";
    let profile = &[
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(75)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(175)),
        ("health/profile/age", "profile/age", FactValue::I64(30)),
    ];
    let q = format!("(query {exom} (find ?ml) (where (health/recommended-water-ml ?ml)))");
    let got = query_first_col(exom, profile, &q, &plan_verbatim_health_rules(exom));
    assert_eq!(
        got,
        vec!["2500".to_string()],
        "expected ONLY '2500' for default (w=75,h=175); got {got:?}"
    );
}

#[test]
fn plan_verbatim_recommended_steps_default() {
    let exom = "plan/health/main";
    let profile = &[
        ("health/profile/age", "profile/age", FactValue::I64(30)),
        ("health/profile/weight_kg", "profile/weight_kg", FactValue::I64(75)),
        ("health/profile/height_cm", "profile/height_cm", FactValue::I64(175)),
    ];
    let q = format!("(query {exom} (find ?sp) (where (health/recommended-steps-per-day ?sp)))");
    let got = query_first_col(exom, profile, &q, &plan_verbatim_health_rules(exom));
    assert_eq!(
        got,
        vec!["9000".to_string()],
        "expected ONLY '9000' for age=30; got {got:?}"
    );
}

// ---------------------------------------------------------------------------
// B7 regression: pinning a string literal in a body atom value position must
// match a DATOM-encoded i64 column. ray-exomem stores str values in the V
// column as `(0x4000... | sym_id)`; rayforce2 used to intern body string
// literals as plain sym ids, so the equality compare missed every row. The
// rayforce2-side fix tracks the body literal's source ray type and lets
// dl_col_eq_row do a tag-aware payload compare against DATOM-tagged I64
// columns. Plain RAY_SYM IDB columns (built from rule heads with string
// constants) keep matching too — the fix is additive: the direct compare
// is tried first, the tagged-payload compare only as a fallback.
// ---------------------------------------------------------------------------

#[test]
fn body_string_literal_pins_datom_encoded_value_column() {
    let exom = "b7/main";
    let profile = &[
        ("f1", "color", FactValue::Str("red".into())),
        ("f2", "color", FactValue::Str("blue".into())),
    ];
    let q = format!(
        r#"(query {exom} (find ?id) (where ({exom} ?id 'color "red")))"#
    );
    let got = query_first_col(exom, profile, &q, &[]);
    assert_eq!(
        got,
        vec!["f1".to_string()],
        "string literal in V position must pin the DATOM-encoded color=red row; got {got:?}"
    );
}

#[test]
fn body_string_literal_pins_datom_encoded_via_facts_str_edb() {
    // Same regression but routed through the typed `facts_str` EDB which
    // also uses encode_string_datom for its V column.
    let exom = "b7/main";
    let profile = &[
        ("f1", "color", FactValue::Str("red".into())),
        ("f2", "color", FactValue::Str("blue".into())),
    ];
    let q = format!(
        r#"(query {exom} (find ?id) (where (facts_str ?id 'color "red")))"#
    );
    let got = query_first_col(exom, profile, &q, &[]);
    assert_eq!(
        got,
        vec!["f1".to_string()],
        "string literal must match facts_str V column; got {got:?}"
    );
}

// ---------------------------------------------------------------------------
// Ñ2 — schema-aware sym pinning. Body atom `(?e 'fact/predicate "name")`
// must compare against the SYM-tagged predicate column. Without the
// lowering-layer rewrite the literal flows through as STR-tagged and the
// engine returns 0 rows. The rewrite walks the canonical AST and converts
// the literal to a quoted symbol when the schema's value_kind for the
// attribute is sym-encoded.
// ---------------------------------------------------------------------------

/// Apply the schema-aware sym-pinning rewrite to a Rayfall query string.
/// Mirrors `expand_canonical_query` in server.rs without the rule-expansion
/// step so the rewrite can be exercised in isolation by E2E tests.
fn rewrite_query_with_schema(query: &str) -> String {
    use ray_exomem::rayfall_ast::{lower_top_level, CanonicalForm, LoweringOptions};

    let expr = ray_exomem::rayfall_ast::parse_one(query).expect("query must parse");
    let lowered = lower_top_level(
        &expr,
        LoweringOptions {
            default_query_exom: None,
            default_rule_exom: None,
        },
    )
    .expect("query must lower");
    let CanonicalForm::Query(mut canonical) = lowered.into_iter().next().expect("one form") else {
        panic!("expected CanonicalForm::Query");
    };
    let brain = Brain::new();
    canonical.rewrite_body_literals_with_schema(|attr| brain.value_kind_for_attr(attr));
    canonical.emit()
}

#[test]
fn body_string_literal_pins_predicate_position_sym_column() {
    // Assert one fact and query its `fact/predicate` via a string literal.
    // The schema declares fact/predicate as value_kind=predicate (sym-encoded),
    // so the rewrite must convert "test/n" → 'test/n before the engine sees it.
    let exom = "n2/main";
    let profile = &[("f1", "test/n", FactValue::Str("payload".into()))];
    let raw_q = format!(
        r#"(query {exom} (find ?id) (where (?id 'fact/predicate "test/n")))"#
    );
    let q = rewrite_query_with_schema(&raw_q);
    assert!(
        q.contains("'test/n"),
        "rewrite must pin literal to sym; got {q}"
    );
    let got = query_first_col(exom, profile, &q, &[]);
    assert!(
        got.iter().any(|v| v == "f1"),
        "expected predicate-name match to return f1; got {got:?}"
    );
}
