#![allow(dead_code)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
pub type Response = (u16, Vec<(&'static str, &'static str)>, String);
pub async fn mock(
    responses: Vec<Response>,
) -> (
    String,
    mpsc::UnboundedReceiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        for (status, headers, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            let mut buffer = [0u8; 4096];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let size = header
                        .lines()
                        .find_map(|l| {
                            l.split_once(':')
                                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            tx.send(String::from_utf8(bytes).unwrap()).unwrap();
            let mut response=format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",body.len());
            for (key, value) in headers {
                response.push_str(&format!("{key}: {value}\r\n"));
            }
            response.push_str("\r\n");
            response.push_str(&body);
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (format!("http://{addr}"), rx, task)
}
pub fn response(status: u16, body: serde_json::Value) -> Response {
    (status, vec![], body.to_string())
}
pub fn profile() -> serde_json::Value {
    serde_json::json!({"id":"11111111-1111-4111-8111-111111111111","username":"bot","avatar_url":"https://example.com/avatar.png","bot":{"name":"bot","public":false,"owner_id":"22222222-2222-4222-8222-222222222222"}})
}
pub fn event(id: u64, command: &str) -> serde_json::Value {
    serde_json::json!({"id":id,"type":"message","data":{"id":uuid::Uuid::new_v4(),"channel_id":uuid::Uuid::nil(),"author_id":uuid::Uuid::nil(),"command":command,"author_bot":false}})
}

/// Route-aware fixtures for concurrent event intake and handler HTTP requests.
/// Exhausted routes stay pending until the fixture finishes, like an open long poll.
pub async fn routed_mock(
    routes: Vec<(String, Vec<Response>)>,
) -> (
    String,
    mpsc::UnboundedReceiver<String>,
    tokio::task::JoinHandle<()>,
) {
    use std::collections::{HashMap, VecDeque};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut routes: HashMap<String, VecDeque<Response>> = routes
            .into_iter()
            .map(|(key, values)| (key, values.into()))
            .collect();
        let mut parked = vec![];
        while routes.values().any(|queue| !queue.is_empty()) {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            let mut buffer = [0u8; 4096];
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let size = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            let parts: Vec<_> = request
                .lines()
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .collect();
            let key = format!(
                "{} {}",
                parts.first().unwrap_or(&""),
                parts
                    .get(1)
                    .unwrap_or(&"")
                    .split('?')
                    .next()
                    .unwrap_or_default()
            );
            let _ = tx.send(request);
            let Some(queue) = routes.get_mut(&key) else {
                panic!("unexpected request {key}");
            };
            let Some((status, headers, body)) = queue.pop_front() else {
                parked.push(socket);
                continue;
            };
            let mut reply=format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",body.len());
            for (key, value) in headers {
                reply.push_str(&format!("{key}: {value}\r\n"));
            }
            reply.push_str("\r\n");
            reply.push_str(&body);
            let _ = socket.write_all(reply.as_bytes()).await;
        }
        drop(parked);
    });
    (format!("http://{address}"), rx, task)
}

#[derive(Clone, Default)]
pub struct TraceCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for TraceCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl TraceCapture {
    pub fn subscribe(&self) {
        let capture = self.clone();
        tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(move || capture.clone())
                .with_max_level(tracing::Level::DEBUG)
                .with_ansi(false)
                .finish(),
        )
        .expect("one tracing capture per test executable");
    }

    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

// Each attached fixture must use a unique Cargo binary target name. Source,
// runtime state, and the worker's running executable remain project-specific.
pub struct WorkerBuildCache(std::path::PathBuf);

impl WorkerBuildCache {
    pub fn attach(project: &std::path::Path) -> Option<Self> {
        let cache = std::path::PathBuf::from(std::env::var_os("RAPPIN_TEST_WORKER_TARGET")?);
        let target = project.join("target");
        let link = target.join("rappin-hot");
        if !cache.is_absolute() {
            return None;
        }
        std::fs::create_dir_all(&cache).ok()?;
        std::fs::create_dir_all(&target).ok()?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&cache, &link).ok()?;
        #[cfg(windows)]
        {
            let status = std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile", "-NonInteractive", "-Command",
                    "New-Item -ItemType Junction -Path $env:RAPPIN_TEST_WORKER_LINK -Target $env:RAPPIN_TEST_WORKER_TARGET -ErrorAction Stop | Out-Null",
                ])
                .env("RAPPIN_TEST_WORKER_LINK", &link)
                .env("RAPPIN_TEST_WORKER_TARGET", &cache)
                .output().ok()?;
            if !status.status.success() {
                return None;
            }
        }
        #[cfg(any(unix, windows))]
        return Some(Self(link));
        #[cfg(not(any(unix, windows)))]
        None
    }
}

impl Drop for WorkerBuildCache {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.0);
        #[cfg(windows)]
        let _ = std::fs::remove_dir(&self.0);
    }
}
