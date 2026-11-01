//! Report inspection must not require hot-reload or CLI features.
mod support;
use rappin::{Bot, FailureClass, HotReloadFailure, RemoteError, YappinError};
use serde_json::json;

fn report() -> HotReloadFailure {
    serde_json::from_value(json!({
        "phase":"compilation","message":"candidate compilation failed","diagnostics":[],
        "truncated":false,"exit":{"code":101,"signal":null},"timed_out":false,
        "candidate_started":false,"publication_attempted":false,"routes_published":false,"source":null
    })).unwrap()
}

#[test]
fn structured_report_is_available_without_optional_features() {
    let error = YappinError::HotReload {
        failure: Box::new(report()),
    };
    let remote: RemoteError =
        serde_json::from_str(&serde_json::to_string(&error.to_remote()).unwrap()).unwrap();
    assert_eq!(remote.hot_reload_failure(), Some(report()));
    assert_eq!(error.to_json()["retry_safety"], "do_not_replay_workflow");
    assert_eq!(error.to_json()["outcome"], "incomplete");
    assert!(!error.is_command_rejection());
}

#[tokio::test]
async fn failed_native_callback_keeps_candidate_report_and_reply_cause() {
    let bot = Bot::builder("unused_fixture")
        .log(false)
        .sync_commands(false)
        .command("reload", "test", |_| async {
            Err(YappinError::HotReload {
                failure: Box::new(report()),
            })
        })
        .on_command_failure(|_ctx, failure| async move {
            assert_eq!(failure.class, FailureClass::Unexpected);
            assert_eq!(failure.error.hot_reload_failure(), Some(report()));
            // Simulate a failed error reply; completion must not be inferred.
            Err(YappinError::Api {
                status: 503,
                message: "reply failed".into(),
                retry_after: Some(0.25),
            })
        })
        .build();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let error = bot
        .process_event(serde_json::from_value(support::event(2, "reload")).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(error, YappinError::FailureHandling { .. }));
    assert_eq!(error.hot_reload_failure(), Some(report()));
    assert_eq!(error.to_remote().hot_reload_failure(), Some(report()));
    assert_eq!(error.to_json()["handling_error"]["status"], 503);
    assert_eq!(error.to_json()["handling_error"]["retry_after"], 0.25);
}
