mod common;
use common::daemon::TestDaemon;
use serde_json::json;

#[test]
fn rename_folder_cascades() {
    let d = TestDaemon::start();
    ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path":"work::ath"})).unwrap();
    let resp: serde_json::Value = ureq::post(&format!("{}/ray-exomem/api/actions/rename", d.base_url))
        .send_json(json!({"path":"work::ath","new_segment":"foo"})).unwrap().into_json().unwrap();
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["new_path"], "work/foo");
    let tree: serde_json::Value = ureq::get(&format!("{}/ray-exomem/api/tree?path=work", d.base_url))
        .call().unwrap().into_json().unwrap();
    assert!(serde_json::to_string(&tree).unwrap().contains("\"foo\""));
}

#[test]
fn rename_rejects_case_collision() {
    let d = TestDaemon::start();
    ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path":"work::foo"})).unwrap();
    ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path":"work::bar"})).unwrap();
    let err = ureq::post(&format!("{}/ray-exomem/api/actions/rename", d.base_url))
        .send_json(json!({"path":"work::bar","new_segment":"FOO"}))
        .unwrap_err();
    if let ureq::Error::Status(s, _) = err { assert_eq!(s, 400); }
    else { panic!("expected case-collision error"); }
}

#[test]
fn rename_rejects_session_id() {
    let d = TestDaemon::start();
    ureq::post(&format!("{}/ray-exomem/api/actions/init", d.base_url))
        .send_json(json!({"path":"work"})).unwrap();
    let s: serde_json::Value = ureq::post(&format!("{}/ray-exomem/api/actions/session-new", d.base_url))
        .send_json(json!({"project_path":"work","type":"single","label":"x","actor":"me"}))
        .unwrap().into_json().unwrap();
    let session_path = s["session_path"].as_str().unwrap().replace('/', "::");
    let err = ureq::post(&format!("{}/ray-exomem/api/actions/rename", d.base_url))
        .send_json(json!({"path":session_path,"new_segment":"y"})).unwrap_err();
    assert!(matches!(err, ureq::Error::Status(400, _)));
}
