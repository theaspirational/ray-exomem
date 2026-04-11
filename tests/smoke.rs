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
