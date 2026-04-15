use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[allow(dead_code)]
pub struct TestDaemon {
    pub data_dir: tempfile::TempDir,
    pub port: u16,
    pub base_url: String,
    pub auth_enabled: bool,
    child: Child,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct TestDaemonBuilder {
    auth_enabled: bool,
}

#[allow(dead_code)]
impl TestDaemonBuilder {
    pub fn new() -> Self {
        Self {
            auth_enabled: false,
        }
    }

    pub fn with_auth(mut self) -> Self {
        self.auth_enabled = true;
        self
    }

    pub fn start(self) -> TestDaemon {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let port = free_port();
        let bin = env!("CARGO_BIN_EXE_ray-exomem");
        let mut cmd = Command::new(bin);

        let bind_arg = format!("127.0.0.1:{port}");
        let data_dir_arg = data_dir
            .path()
            .to_str()
            .expect("tempdir is utf-8")
            .to_string();

        let mut args: Vec<String> = vec![
            "serve".into(),
            "--bind".into(),
            bind_arg,
            "--data-dir".into(),
            data_dir_arg,
        ];

        if self.auth_enabled {
            args.push("--auth-provider".into());
            args.push("mock".into());
        }

        cmd.args(&args).stdout(Stdio::null()).stderr(Stdio::null());

        // Put child in its own process group so Drop can kill the whole group.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }
        }
        let child = cmd.spawn().expect("spawn daemon");

        let base_url = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut ready = false;
        while Instant::now() < deadline {
            if let Ok(r) = ureq::get(&format!("{base_url}/ray-exomem/api/status")).call() {
                if r.status() == 200 {
                    ready = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !ready {
            panic!("daemon did not become healthy within 15 seconds at {base_url}");
        }

        TestDaemon {
            data_dir,
            port,
            base_url,
            auth_enabled: self.auth_enabled,
            child,
        }
    }
}

// ---------------------------------------------------------------------------
// TestDaemon
// ---------------------------------------------------------------------------

#[allow(dead_code)]
impl TestDaemon {
    /// Start a test daemon with default settings (no auth).
    pub fn start() -> Self {
        TestDaemonBuilder::new().start()
    }

    pub fn tree_root(&self) -> PathBuf {
        self.data_dir.path().join("tree")
    }

    /// Login as a mock user, return the session cookie value.
    ///
    /// Requires the daemon to have been started with auth enabled
    /// (via `TestDaemonBuilder::new().with_auth().start()`).
    pub fn mock_login(&self, email: &str, name: &str) -> String {
        assert!(
            self.auth_enabled,
            "mock_login requires auth to be enabled; use TestDaemonBuilder::new().with_auth().start()"
        );

        let resp = ureq::post(&format!("{}/auth/login", self.base_url))
            .send_json(serde_json::json!({
                "id_token": format!("mock:{email}:{name}"),
                "provider": "mock"
            }))
            .expect("mock login should succeed");

        // Extract session cookie from Set-Cookie header.
        let cookie = resp
            .header("set-cookie")
            .expect("login response should set a cookie");
        // Parse "ray_exomem_session=<value>; HttpOnly; ..." -> extract value.
        cookie
            .split(';')
            .next()
            .unwrap()
            .strip_prefix("ray_exomem_session=")
            .unwrap()
            .to_string()
    }

    /// Make an authenticated GET request using a session cookie.
    pub fn auth_get(&self, path: &str, session: &str) -> ureq::Response {
        ureq::get(&format!("{}{}", self.base_url, path))
            .set("Cookie", &format!("ray_exomem_session={session}"))
            .call()
            .expect("auth_get")
    }

    /// Make an authenticated POST request using a session cookie.
    pub fn auth_post(&self, path: &str, session: &str, body: serde_json::Value) -> ureq::Response {
        ureq::post(&format!("{}{}", self.base_url, path))
            .set("Cookie", &format!("ray_exomem_session={session}"))
            .send_json(body)
            .expect("auth_post")
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        // SIGKILL ensures cleanup even if the child ignores SIGTERM.
        // Also kill by process group to catch any grandchildren.
        #[cfg(unix)]
        {
            let pid = self.child.id() as i32;
            // Kill process group (negative PID)
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            // Also direct kill in case setsid wasn't used
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().unwrap().port()
}
