mod common;
use common::daemon::TestDaemon;
use std::process::Command;

/// Run a ray-exomem command with --data-dir pointing at the test daemon's data dir.
fn run(d: &TestDaemon, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ray-exomem"))
        .args(args)
        .args(["--data-dir", d.data_dir.path().to_str().unwrap()])
        .output()
        .unwrap()
}

// ============================================================
// Task 5.1 — init and exom new subcommands
// ============================================================

#[test]
fn init_creates_project_on_disk() {
    let d = TestDaemon::start();
    let out = run(&d, &["init", "work::ath::lynx::orsl"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(d
        .tree_root()
        .join("work/ath/lynx/orsl/main/exom.json")
        .exists());
    assert!(d.tree_root().join("work/ath/lynx/orsl/sessions").is_dir());
}

#[test]
fn exom_new_creates_bare_exom() {
    let d = TestDaemon::start();
    let out = run(&d, &["exom-new", "work::notes"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(d.tree_root().join("work/notes/exom.json").exists());
}

// ============================================================
// Task 5.2 — session new/add-agent/join/rename/close/archive
// ============================================================

#[test]
fn session_new_creates_session_exom() {
    let d = TestDaemon::start();
    let url = format!("http://127.0.0.1:{}/ray-exomem", d.port);
    let out0 = run(&d, &["init", "work::ath"]);
    assert!(
        out0.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out0.stderr)
    );
    let out = Command::new(env!("CARGO_BIN_EXE_ray-exomem"))
        .args([
            "--data-dir",
            d.data_dir.path().to_str().unwrap(),
            "--daemon-url",
            &url,
            "session",
            "new",
            "work::ath",
            "--multi",
            "--name",
            "landing",
            "--actor",
            "orchestrator",
            "--agents",
            "agent_a,agent_b",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sessions_dir = d.tree_root().join("work/ath/sessions");
    let entries: Vec<_> = std::fs::read_dir(&sessions_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        entries.iter().any(|e| e.ends_with("_multi_agent_landing")),
        "no session matching pattern in {:?}",
        entries
    );
}

#[test]
#[ignore = "FIXME(nested-exoms-task-4.4): add-agent / branch-create is a 501 stub"]
fn session_add_agent_returns_501() {
    let d = TestDaemon::start();
    let url = format!("http://127.0.0.1:{}/ray-exomem", d.port);
    let _out = Command::new(env!("CARGO_BIN_EXE_ray-exomem"))
        .args([
            "--data-dir",
            d.data_dir.path().to_str().unwrap(),
            "--daemon-url",
            &url,
            "session",
            "add-agent",
            "work::ath::sessions::some-session",
            "--agent",
            "agent_x",
        ])
        .output()
        .unwrap();
    // 501 stub — test is ignored until Task 4.4 lands
}
