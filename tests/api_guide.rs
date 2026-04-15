mod common;
use common::daemon::TestDaemon;

#[test]
fn guide_endpoint_returns_markdown() {
    let d = TestDaemon::start();
    let body = ureq::get(&format!("{}/ray-exomem/api/guide", d.base_url))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(
        body.contains("# Ray-exomem agent guide"),
        "guide missing title: {}",
        &body[..200.min(body.len())]
    );
    assert!(
        body.contains("Branching"),
        "guide missing Branching section"
    );
}
