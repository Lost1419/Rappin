//! Explicit operator assessment of failed/uncertain work; never replays handlers.
//! This uses the SAME consumer-owned journal/lease as the timestamp example.
//! Do not export/edit another database to manufacture an approval.
#[cfg(unix)]
#[path = "support/ingress_journal.rs"]
mod journal;
#[cfg(all(test, unix))]
#[path = "../tests/support/mod.rs"]
mod support;
#[cfg(unix)]
#[tokio::main]
async fn main() -> rappin::Result<()> {
    use rappin::*;
    use std::{path::PathBuf, sync::Arc};
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() == 1 && args[0] == "--help" {
        println!("YAPPER_BOT_TOKEN=... cargo run --example reconcile_event_retirement -- JOURNAL BOT_UUID EVENT_ID ORIGINAL_SHA256 POLLING_SHA256|retained-execution [--approve AUDIT_ID --assessment accept-unknown|attested-applied|attested-not-applied --evidence EVIDENCE_ID]\nDefault: read-only preparation. Hold the stopped consumer's exclusive lease. Approval retires this execution without replay; it never rewrites its original Started/Failed receipt or infers remote success. Resume with the same atomic journal adapter and polling authority.");
        return Ok(());
    }
    if !(args.len() == 5
        || args.len() == 11
            && args[5] == "--approve"
            && args[7] == "--assessment"
            && args[9] == "--evidence")
    {
        return Err(Error::Config("use --help for arguments".into()));
    }
    let bot = args[1]
        .parse()
        .map_err(|_| Error::Config("invalid bot UUID".into()))?;
    let event = args[2]
        .parse()
        .map_err(|_| Error::Config("invalid event ID".into()))?;
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| Error::Config("set YAPPER_BOT_TOKEN".into()))?;
    let store = Arc::new(journal::Journal::open(PathBuf::from(&args[0]))?);
    let request = if args[4] == "retained-execution" {
        EventRetirementRequest::retained_execution(bot, event, &args[3])
    } else {
        EventRetirementRequest::new(bot, event, &args[3], &args[4])
    };
    let prepared =
        prepare_event_retirement(&Client::new(token), store, request, Default::default()).await?;
    println!("{}", serde_json::to_string_pretty(prepared.evidence())?);
    if args.len() == 11 {
        let assessment = match args[8].as_str() {
            "accept-unknown" => EventRetirementAssessment::AcceptUnknown,
            "attested-applied" => EventRetirementAssessment::AttestedApplied,
            "attested-not-applied" => EventRetirementAssessment::AttestedNotApplied,
            _ => {
                return Err(Error::Config(
                    "select an explicit assessment from --help".into(),
                ))
            }
        };
        // The consumer, not this SDK, establishes authority and evidentiary quality.
        let decision = prepared
            .approve(
                OperationId::new(&args[6])?,
                assessment,
                OperationId::new(&args[10])?,
            )
            .await?;
        println!("Retirement {} approved with assessment {:?}. Original error and receipts are preserved. Normal matching polling is still required before checkpoint progression.",decision.decision_id,decision.assessment);
    }
    Ok(())
}
#[cfg(not(unix))]
fn main() {
    eprintln!("The SDK API is portable; this example's fsync/rename journal adapter requires Unix. Use a transactional database adapter on other systems.");
}
