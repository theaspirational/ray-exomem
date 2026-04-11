use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[allow(dead_code)]
pub struct TestDaemon {
    pub data_dir: tempfile::TempDir,
    pub port: u16,
    pub base_url: String,
    child: Child,
}

#[allow(dead_code)]
impl TestDaemon {
    pub fn start() -> Self {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let port = free_port();
        let bin = env!("CARGO_BIN_EXE_ray-exomem");
        let child = Command::new(bin)
            .args([
                "serve",
                "--bind", &format!("127.0.0.1:{port}"),
                "--data-dir", data_dir.path().to_str().expect("tempdir is utf-8"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon");

        let base_url = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut ready = false;
        while Instant::now() < deadline {
            if let Ok(r) = ureq::get(&format!("{base_url}/ray-exomem/api/status")).call() {
                if r.status() == 200 { ready = true; break; }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !ready {
            panic!("daemon did not become healthy within 5 seconds at {base_url}");
        }

        TestDaemon { data_dir, port, base_url, child }
    }

    pub fn tree_root(&self) -> PathBuf {
        self.data_dir.path().join("tree")
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().unwrap().port()
}
