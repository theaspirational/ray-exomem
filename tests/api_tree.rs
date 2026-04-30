mod common;
use common::daemon::TestDaemon;

#[test]
fn api_tree_empty_root_returns_folder() {
    let d = TestDaemon::start();
    let body: serde_json::Value = ureq::get(&format!("{}/api/tree", d.base_url))
        .call()
        .unwrap()
        .into_json()
        .unwrap();
    assert_eq!(body["kind"], "folder");
}
