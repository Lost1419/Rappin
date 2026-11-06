//! Explicit maintenance for a consumer-owned JSON ingress journal (Unix).
//! Run with --help. The journal must already be the stopped consumer's durable
//! source of truth, not an edited/exported copy of another database. No handlers
//! run here. Consumers using a database should implement the same atomic hook
//! in their own transaction; do not copy this journal over their database.
#[cfg(all(test, unix))]
#[path = "../tests/support/mod.rs"]
mod support;

#[cfg(unix)]
#[path = "support/ingress_journal.rs"]
mod journal;
#[cfg(unix)]
mod consumer {
    use crate::journal::Journal;
    use rappin::*;
    use std::{path::PathBuf, sync::Arc};
    pub async fn run() -> Result<()> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.len() == 1 && args[0] == "--help" {
            println!("YAPPER_BOT_TOKEN=... cargo run --example reconcile_event_timestamp -- JOURNAL BOT_UUID EVENT_ID RETAINED_SHA256 POLLING_SHA256 [--approve AUDIT_ID]\nWithout --approve, only fresh GET /me, GET /events and journal reads occur. Hold the same exclusive lease as the stopped consumer. Journal format: {{\"ingress\": <IngressRecovery>, \"audit\": []}}. No source of truth is created or rewritten from an export.");
            return Ok(());
        }
        if !(args.len() == 5 || args.len() == 7 && args[5] == "--approve") {
            return Err(YappinError::Config("use --help for arguments".into()));
        }
        let bot = args[1]
            .parse()
            .map_err(|_| YappinError::Config("invalid bot UUID".into()))?;
        let event = args[2]
            .parse()
            .map_err(|_| YappinError::Config("invalid event ID".into()))?;
        let token = std::env::var("YAPPER_BOT_TOKEN")
            .map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;
        let store = Arc::new(Journal::open(PathBuf::from(&args[0]))?);
        let prepared = prepare_timestamp_reconciliation(
            &Client::new(token),
            store,
            TimestampReconciliationRequest::new(bot, event, &args[3], &args[4]),
            Default::default(),
        )
        .await?;
        println!("{}", serde_json::to_string_pretty(prepared.evidence())?);
        if args.len() == 7 {
            // This flag stands for the consumer's approval policy, not SDK authority.
            let decision = prepared.approve(OperationId::new(&args[6])?).await?;
            println!("Approved {}. Resume the consumer with the SAME journal adapter; normal matching polling must confirm before checkpoint release.",decision.decision_id);
        }
        Ok(())
    }
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> rappin::Result<()> {
    consumer::run().await
}
#[cfg(not(unix))]
fn main() {
    eprintln!("The SDK API is portable; this example's fsync/rename journal adapter requires Unix. Use a transactional database adapter on other systems.");
}
