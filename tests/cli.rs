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
