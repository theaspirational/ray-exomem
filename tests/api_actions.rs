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
