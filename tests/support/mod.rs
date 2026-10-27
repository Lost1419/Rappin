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
