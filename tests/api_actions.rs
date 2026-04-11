mod common;
use common::daemon::TestDaemon;
use serde_json::json;

// FIXME(nested-exoms-task-12.x): write SSE integration test for tree-changed events

#[test]
fn init_then_session_new_then_tree_contains_session() {
    let d = TestDaemon::start();

    let r = ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path": "work::ath"})).unwrap();
    assert_eq!(r.status(), 200);

    let resp: serde_json::Value = ureq::post(&format!("{}/ray-exomem/api/actions/session-new", d.base_url))
        .send_json(json!({
            "project_path": "work::ath",
            "type": "multi",
            "label": "landing",
            "actor": "orchestrator",
            "agents": ["agent_a", "agent_b"],
        })).unwrap().into_json().unwrap();
    assert_eq!(resp["ok"], true);
    let session_path = resp["session_path"].as_str().unwrap().to_string();

    let tree: serde_json::Value = ureq::get(&format!("{}/ray-exomem/api/tree?path=work/ath&depth=3", d.base_url))
        .call().unwrap().into_json().unwrap();
    let s = serde_json::to_string(&tree).unwrap();
    assert!(s.contains("_multi_agent_landing"), "tree should contain new session: {}", s);
    assert!(s.contains(&session_path) || s.contains("landing"), "tree should contain session path: {}", s);
}

#[test]
fn exom_new_creates_bare_exom() {
    let d = TestDaemon::start();
    let r: serde_json::Value = ureq::post(&format!("{}/ray-exomem/api/actions/exom-new", d.base_url))
        .send_json(json!({"path": "scratch::notes"})).unwrap().into_json().unwrap();
    assert_eq!(r["ok"], true);

    let tree: serde_json::Value = ureq::get(&format!("{}/ray-exomem/api/tree?path=scratch", d.base_url))
        .call().unwrap().into_json().unwrap();
    let s = serde_json::to_string(&tree).unwrap();
    assert!(s.contains("notes"), "tree should contain bare exom: {}", s);
}

#[test]
fn session_join_returns_501() {
    let d = TestDaemon::start();
    let err = ureq::post(&format!("{}/ray-exomem/api/actions/session-join", d.base_url))
        .send_json(json!({"session_path": "x", "actor": "y"})).unwrap_err();
    if let ureq::Error::Status(s, _) = err { assert_eq!(s, 501); }
    else { panic!("expected status error"); }
}

#[test]
fn branch_create_returns_501() {
    let d = TestDaemon::start();
    let err = ureq::post(&format!("{}/ray-exomem/api/actions/branch-create", d.base_url))
        .send_json(json!({"exom": "x", "branch": "y"})).unwrap_err();
    if let ureq::Error::Status(s, _) = err { assert_eq!(s, 501); }
    else { panic!("expected status error"); }
}

// ---------------------------------------------------------------------------
// Task 4.4 tests
// ---------------------------------------------------------------------------

#[test]
fn assert_fact_requires_actor() {
    let d = TestDaemon::start();
    ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path": "work"})).unwrap();
    let err = ureq::post(&format!("{}/ray-exomem/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom": "work::main",
            "predicate": "note/body",
            "value": "hello",
        })).unwrap_err();
    if let ureq::Error::Status(status, r) = err {
        assert_eq!(status, 400);
        let body: serde_json::Value = r.into_json().unwrap();
        assert_eq!(body["code"], "actor_required");
    } else {
        panic!("expected status error, got: {:?}", err);
    }
}

#[test]
fn api_exoms_is_gone() {
    let d = TestDaemon::start();
    let err = ureq::get(&format!("{}/ray-exomem/api/exoms", d.base_url))
        .call().unwrap_err();
    if let ureq::Error::Status(status, _) = err {
        assert_eq!(status, 410);
    } else {
        panic!("expected status error");
    }
}

#[test]
fn assert_fact_with_actor_succeeds() {
    let d = TestDaemon::start();
    ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path": "work"})).unwrap();
    let resp = ureq::post(&format!("{}/ray-exomem/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom": "work::main",
            "branch": "main",
            "actor": "me",
            "predicate": "note/body",
            "value": "hello",
        })).unwrap();
    assert_eq!(resp.status(), 200);
}

#[test]
fn status_includes_tree_root() {
    let d = TestDaemon::start();
    let body: serde_json::Value = ureq::get(&format!("{}/ray-exomem/api/status", d.base_url))
        .call().unwrap().into_json().unwrap();
    assert!(body["server"]["tree_root"].is_string(), "server.tree_root missing: {}", body);
}

// ---------------------------------------------------------------------------
// Task 5 load-bearing tests
// ---------------------------------------------------------------------------

#[test]
fn nested_exoms_do_not_collide() {
    let d = TestDaemon::start();
    // Create two projects whose main exoms have the same last segment "main".
    ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path":"work"})).unwrap();
    ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path":"lab"})).unwrap();

    // Write to work::main with one value.
    ureq::post(&format!("{}/ray-exomem/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom":"work::main","branch":"main","actor":"alice",
            "predicate":"note/body","value":"work-note",
        })).unwrap();

    // Write to lab::main with a different value.
    ureq::post(&format!("{}/ray-exomem/api/actions/assert-fact", d.base_url))
        .send_json(json!({
            "exom":"lab::main","branch":"main","actor":"alice",
            "predicate":"note/body","value":"lab-note",
        })).unwrap();

    // The on-disk files must be in their respective tree paths.
    assert!(d.tree_root().join("work/main/fact.jsonl").exists(),
            "work::main facts must live under tree/work/main/");
    assert!(d.tree_root().join("lab/main/fact.jsonl").exists(),
            "lab::main facts must live under tree/lab/main/");

    // Read back via /api/tree?path=work/main and confirm fact_count is correct.
    let work: serde_json::Value = ureq::get(&format!("{}/ray-exomem/api/tree?path=work/main", d.base_url))
        .call().unwrap().into_json().unwrap();
    assert_eq!(work["fact_count"], 1, "work/main should have 1 fact, got: {}", work);
    let lab: serde_json::Value = ureq::get(&format!("{}/ray-exomem/api/tree?path=lab/main", d.base_url))
        .call().unwrap().into_json().unwrap();
    assert_eq!(lab["fact_count"], 1, "lab/main should have 1 fact, got: {}", lab);
}

#[test]
fn post_api_exoms_is_gone() {
    let d = TestDaemon::start();
    let err = ureq::post(&format!("{}/ray-exomem/api/exoms", d.base_url))
        .send_json(json!({"name":"foo"})).unwrap_err();
    if let ureq::Error::Status(s, _) = err { assert_eq!(s, 410); }
    else { panic!("expected 410"); }
}

#[test]
#[ignore] // FIXME(nested-exoms-task-4.4): session-join deferred to Task 4.4
fn session_join_claims_branch() {
    let d = TestDaemon::start();
    let _ = ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path": "work"})).unwrap();
    let s: serde_json::Value = ureq::post(&format!("{}/ray-exomem/api/actions/session-new", d.base_url))
        .send_json(json!({
            "project_path": "work",
            "type": "multi",
            "label": "test",
            "actor": "orch",
            "agents": ["agent_a"],
        })).unwrap().into_json().unwrap();
    let session_path = s["session_path"].as_str().unwrap().replace('/', "::");
    let r: serde_json::Value = ureq::post(&format!("{}/ray-exomem/api/actions/session-join", d.base_url))
        .send_json(json!({"session_path": session_path, "actor": "agent_a"}))
        .unwrap().into_json().unwrap();
    assert_eq!(r["ok"], true);
}

#[test]
#[ignore] // FIXME(nested-exoms-task-4.4): branch-create deferred to Task 4.4
fn branch_create_works() {
    let d = TestDaemon::start();
    let _ = ureq::post(&format!("{}/ray-exomem/api/actions/exom-new", d.base_url))
        .send_json(json!({"path": "work::main"})).unwrap();
    let r: serde_json::Value = ureq::post(&format!("{}/ray-exomem/api/actions/branch-create", d.base_url))
        .send_json(json!({"exom": "work::main", "branch": "feature-x"}))
        .unwrap().into_json().unwrap();
    assert_eq!(r["ok"], true);
}
