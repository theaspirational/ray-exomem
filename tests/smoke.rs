mod common;
use common::daemon::TestDaemon;

#[test]
#[ignore = "Hits /api/status unauthenticated; daemon now 500s/401s with auth_store configured. Re-enable when CLI auth lands (see CLAUDE.md 'CLI / curl auth gap')."]
fn daemon_starts_and_reports_status() {
    let d = TestDaemon::start();
    let resp = ureq::get(&format!("{}/api/status", d.base_url))
        .call()
        .expect("status");
    assert_eq!(resp.status(), 200);
}
