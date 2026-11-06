//! Loopback receiver with volatile admission and polling/cursor recovery.
//! Set YAPIN_BOT_TOKEN, YAPIN_BOT_ID and YAPIN_WEBHOOK_SECRET.
//! cargo run --features webhook-middleware --example shared_webhook_polling
use axum::{
    extract::{Extension, State},
    http::StatusCode,
    middleware,
    routing::post,
    Json, Router,
};
use rappin::{
    webhook::VerifiedEvent,
    webhook_middleware::{verify_middleware, WebhookConfig},
    AdmissionStatus, Bot, EventIngress, IngressConfig, IngressConflictPolicy, ReceiptStage, Result,
    YappinError,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::oneshot;
async fn receive(
    State(ingress): State<EventIngress>,
    Extension(event): Extension<VerifiedEvent>,
) -> (StatusCode, Json<Value>) {
    let admission = ingress.admit(event).await;
    let status = match admission.status {
        AdmissionStatus::Accepted | AdmissionStatus::Duplicate => {
            let receipt = admission.receipt.expect("admitted receipt").status();
            if matches!(
                receipt.stage,
                ReceiptStage::Failed | ReceiptStage::Unprocessed
            ) {
                return (
                    StatusCode::CONFLICT,
                    Json(
                        json!({"admission":"requires_reconciliation","event_id":receipt.event_id}),
                    ),
                );
            }
            return (
                StatusCode::ACCEPTED,
                Json(
                    json!({"admission":"accepted","durability":receipt.durability,"event_id":receipt.event_id}),
                ),
            );
        }
        AdmissionStatus::PollingRequired => {
            return (
                StatusCode::ACCEPTED,
                Json(json!({"admission":"polling_required","durability":"not_admitted"})),
            )
        }
        AdmissionStatus::Full | AdmissionStatus::Stopped => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::CONFLICT,
    };
    if let Some(conflict) = admission
        .error
        .as_ref()
        .and_then(YappinError::ingress_conflict)
    {
        // The bounded report contains sources, hashes and schema paths/types,
        // never message text, interaction credentials or arbitrary payload values.
        return (
            status,
            Json(json!({"admission":"conflict","conflict":conflict})),
        );
    }
    (status, Json(json!({"admission":"not_accepted"})))
}
fn required(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| YappinError::Config(format!("set {name}")))
}
#[tokio::main]
async fn main() -> Result<()> {
    let token = required("YAPIN_BOT_TOKEN")?;
    let bot_id = required("YAPIN_BOT_ID")?;
    bot_id
        .parse::<uuid::Uuid>()
        .map_err(|_| YappinError::Config("YAPIN_BOT_ID must be a UUID".into()))?;
    let secret = required("YAPIN_WEBHOOK_SECRET")?;
    let cursor =
        std::env::var("YAPIN_CURSOR_FILE").unwrap_or_else(|_| "shared-example-cursor.json".into());
    let mut config = IngressConfig::default();
    for arg in std::env::args().skip(1) {
        if arg == "--retain-completed-and-poll" {
            config.conflict_policy = IngressConflictPolicy::RetainCompletedAndPoll;
        } else {
            return Err(YappinError::Config(
                "supported argument: --retain-completed-and-poll".into(),
            ));
        }
    }
    let bot = Bot::builder(token)
        .shared_ingress(config)
        .cursor_file(cursor)
        .command("ping", "Reply once across transports", |ctx| async move {
            ctx.reply("pong").await?;
            Ok(())
        })
        .build();
    let handle = bot.handle();
    let ingress = handle.ingress().expect("shared ingress configured");
    let mut runner = tokio::spawn(bot.run());
    tokio::select! {
        ready=ingress.wait_ready()=>{if ready.is_err(){return runner.await.map_err(|_|YappinError::Config("runner task failed".into()))?;}},
        result=&mut runner=>return result.map_err(|_|YappinError::Config("runner task failed".into()))?,
    }
    // Signature middleware retains the exact raw body and expected bot identity.
    // HTTP admission is independent of handler completion. This example's
    // acknowledgement is VOLATILE; use an application-owned IngressStore for
    // durable receipts and uncertain-effect protection across process restarts.
    let app = Router::new()
        .route("/webhook", post(receive))
        .route_layer(middleware::from_fn_with_state(
            Arc::new(WebhookConfig::new(secret, bot_id)),
            verify_middleware,
        ))
        .with_state(ingress.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    let (shutdown, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let result = tokio::select! {
        result=&mut runner=>result.map_err(|_|YappinError::Config("runner task failed".into()))?,
        _=tokio::signal::ctrl_c()=>{handle.stop();runner.await.map_err(|_|YappinError::Config("runner task failed".into()))?},
    };
    let _ = shutdown.send(());
    let _ = server.await;
    for conflict in ingress.conflicts() {
        eprintln!("{}", serde_json::to_string(&conflict)?);
    }
    result
}
