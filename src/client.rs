//! Thin HTTP client for talking to the ray-exomem daemon.
//!
//! Uses raw TCP + HTTP/1.1 — no async runtime or external HTTP crate needed.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// Base URL for the daemon API.
const DEFAULT_ADDR: &str = "127.0.0.1:9780";
const DEFAULT_BASE_PATH: &str = env!("RAY_EXOMEM_BASE_PATH");

pub struct Client {
    addr: String,
    base_path: String,
}

fn parse_endpoint(endpoint: &str) -> (String, String) {
    let without_scheme = endpoint
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let (addr, path) = match without_scheme.split_once('/') {
        Some((addr, rest)) => (addr, format!("/{}", rest.trim_matches('/'))),
        None => (without_scheme, DEFAULT_BASE_PATH.to_string()),
    };
    let base_path = if path == "/" { String::new() } else { path };
    (addr.to_string(), base_path)
}

impl Client {
    pub fn new(endpoint: Option<&str>) -> Self {
        let (addr, base_path) = parse_endpoint(endpoint.unwrap_or(DEFAULT_ADDR));
        Self { addr, base_path }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_path, path)
    }

    /// DELETE request, returns the response body.
    pub fn delete(&self, path: &str) -> Result<String> {
        self.delete_with_headers(path, &[])
    }

    /// DELETE with extra headers (e.g. X-Actor for branch archive).
    pub fn delete_with_headers(&self, path: &str, extra: &[(&str, &str)]) -> Result<String> {
        let url = self.url(path);
        let mut header_block = String::new();
        for (k, v) in extra {
            header_block.push_str(&format!("{}: {}\r\n", k, v));
        }
        let request = format!(
            "DELETE {} HTTP/1.1\r\nHost: {}\r\n{}Connection: close\r\n\r\n",
            url, self.addr, header_block
        );
        self.send_request(&request)
    }

    /// GET request, returns the response body.
    pub fn get(&self, path: &str) -> Result<String> {
        let url = self.url(path);
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            url, self.addr
        );
        self.send_request(&request)
    }

    /// POST request with JSON body, returns the response body.
    pub fn post_json(&self, path: &str, body: &str) -> Result<String> {
        self.post_json_with_headers(path, body, &[])
    }

    /// POST JSON with extra headers (e.g. X-Actor).
    pub fn post_json_with_headers(
        &self,
        path: &str,
        body: &str,
        extra: &[(&str, &str)],
    ) -> Result<String> {
        let url = self.url(path);
        let mut header_block = String::new();
        for (k, v) in extra {
            header_block.push_str(&format!("{}: {}\r\n", k, v));
        }
        let request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
            url,
            self.addr,
            body.len(),
            header_block,
            body
        );
        self.send_request(&request)
    }

    /// POST request with plain text body.
    pub fn post_text(&self, path: &str, body: &str) -> Result<String> {
        self.post_text_with_headers(path, body, &[])
    }

    /// POST plain text with extra headers (e.g. X-Actor, X-Session, X-Model).
    pub fn post_text_with_headers(
        &self,
        path: &str,
        body: &str,
        extra: &[(&str, &str)],
    ) -> Result<String> {
        let url = self.url(path);
        let mut header_block = String::new();
        for (k, v) in extra {
            header_block.push_str(&format!("{}: {}\r\n", k, v));
        }
        let request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
            url,
            self.addr,
            body.len(),
            header_block,
            body
        );
        self.send_request(&request)
    }

    fn send_request(&self, request: &str) -> Result<String> {
        let mut stream = TcpStream::connect(&self.addr).with_context(|| {
            format!("cannot connect to daemon at {} — is it running?", self.addr)
        })?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

        stream
            .write_all(request.as_bytes())
            .context("failed to send request to daemon")?;

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .context("failed to read response from daemon")?;

        let response_str = String::from_utf8_lossy(&response);

        // Parse HTTP response: find body after \r\n\r\n
        let body_start = response_str.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);

        let body = &response_str[body_start..];

        // Check for HTTP error status
        if let Some(status_line) = response_str.lines().next() {
            if let Some(code) = status_line.split_whitespace().nth(1) {
                let code: u16 = code.parse().unwrap_or(0);
                if code >= 400 {
                    bail!("daemon returned HTTP {}: {}", code, body.trim());
                }
            }
        }

        Ok(body.to_string())
    }

    /// Long-lived GET for Server-Sent Events: skip HTTP headers, then copy the remainder to `out`.
    pub fn stream_sse(&self, path: &str, out: &mut impl Write) -> Result<()> {
        let url = self.url(path);
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n",
            url, self.addr
        );
        let mut stream = TcpStream::connect(&self.addr).with_context(|| {
            format!("cannot connect to daemon at {} — is it running?", self.addr)
        })?;
        stream.set_read_timeout(None).ok();
        stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

        stream
            .write_all(request.as_bytes())
            .context("failed to send SSE request to daemon")?;

        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream
                .read(&mut chunk)
                .context("failed to read SSE response headers")?;
            if n == 0 {
                bail!("daemon closed connection before SSE headers completed");
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf
                .windows(4)
                .position(|w| w == [b'\r', b'\n', b'\r', b'\n'])
            {
                let after = pos + 4;
                out.write_all(&buf[after..])
                    .context("failed to write SSE prelude")?;
                out.flush().ok();
                break;
            }
            if buf.len() > 65536 {
                bail!("SSE response headers too large");
            }
        }

        loop {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            out.write_all(&chunk[..n])?;
            out.flush().ok();
        }
        Ok(())
    }
}
