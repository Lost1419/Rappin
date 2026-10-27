#![cfg(feature = "gateway")]
mod support;
use std::sync::Mutex;
struct Capture(Mutex<Vec<String>>);
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().unwrap().push(format!("{}", record.args()));
    }
    fn flush(&self) {}
}
static LOG: Capture = Capture(Mutex::new(Vec::new()));
#[tokio::test]
async fn gateway_never_authenticates_when_raw_frame_logging_is_enabled() {
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (base, _requests, task) = support::mock(vec![support::response(
        200,
        serde_json::json!({"events":[],"last":2}),
    )])
    .await;
    let client = rappin::Client::builder("yb_private_fixture")
        .base_url(base)
        .build();
    let mut gateway = rappin::gateway::Gateway::new(client).url(format!("ws://{addr}"));
    assert_eq!(gateway.poll(1).await.unwrap().last, 2);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
    assert!(!LOG
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.contains("yb_private_fixture")));
    task.await.unwrap();
}
