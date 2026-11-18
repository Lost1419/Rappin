mod support;

use rappin::{AvailabilityPolicy, Client};
use serde_json::json;
use std::time::Duration;
use support::{profile, response};

#[tokio::test]
async fn retry_traces_report_context_without_remote_or_request_secrets() {
    let capture = support::TraceCapture::default();
    capture.subscribe();
    let (base, mut requests, server) = support::mock(vec![
        response(520, json!({"error":"private-response-body"})),
        response(200, profile()),
        (
            429,
            vec![("Retry-After", "0.001")],
            json!({"error":"private-response-body"}).to_string(),
        ),
        response(200, profile()),
    ])
    .await;
    let policy = AvailabilityPolicy::new(2, Duration::from_secs(3))
        .unwrap()
        .backoff(Duration::from_millis(2), Duration::from_millis(8))
        .unwrap();
    let supervised = Client::builder("private-bot-token")
        .base_url(&base)
        .read_availability_policy(policy)
        .build();
    assert_eq!(supervised.get_me().await.unwrap().username, "bot");
    let basic = Client::builder("private-bot-token")
        .base_url(&base)
        .without_read_recovery()
        .build();
    assert_eq!(basic.get_me().await.unwrap().username, "bot");
    server.await.unwrap();
    assert_eq!((0..4).filter(|_| requests.try_recv().is_ok()).count(), 4);
    assert!(requests.try_recv().is_err());

    let trace = capture.text();
    assert!(trace
        .lines()
        .any(|line| line.contains("read availability retry scheduled")
            && line.contains("phase=Request")
            && line.contains("status=520")
            && line.contains(r#"cause="http_status""#)));
    assert!(trace
        .lines()
        .any(|line| line.contains("read availability recovered") && line.contains("attempts=2")));
    assert!(trace
        .lines()
        .any(|line| line.contains("API asked us to retry")
            && line.contains("route=/me")
            && line.contains("attempt=1")
            && line.contains("status=429")));
    for private in ["private-bot-token", "private-response-body", &base] {
        assert!(!trace.contains(private));
    }
}
