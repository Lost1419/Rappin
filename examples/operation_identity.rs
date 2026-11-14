//! Run with YAPPER_BOT_TOKEN set; send !record-plan in a server channel.
//! The in-memory journal is illustrative and does NOT provide restart safety.
use rappin::{Bot, Check, CommandError, CommandSpec, OperationId, Result};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

struct Recipient(String);
impl rappin::FromArgs for Recipient {
    fn parse(args: &mut rappin::Arguments) -> std::result::Result<Self, CommandError> {
        Ok(Self(args.required("recipient")?))
    }
    fn describe() -> Vec<rappin::ArgumentInfo> {
        vec![rappin::ArgumentInfo::required(
            "recipient",
            "Record recipient",
        )]
    }
}
// Include authenticated scope in the consumer's key, never in a replacement message.
type Journal = Arc<Mutex<HashMap<(Uuid, Uuid, OperationId), String>>>;
#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| rappin::Error::Config("set YAPPER_BOT_TOKEN".into()))?;
    let journal: Journal = Arc::default();
    Bot::builder(token)
        .command_spec(
            CommandSpec::<Recipient>::new("record", "Record a recipient")
                .check(Check::server_only()),
            move |ctx, Recipient(recipient)| {
                let journal = journal.clone();
                async move {
                    let operation = ctx.operation_id().ok_or_else(|| {
                        CommandError::Denied("record requires a consumer operation key".into())
                    })?;
                    // Run application authorization here even for a previously completed key.
                    let key = (
                        ctx.server_id().expect("server check"),
                        ctx.author_id()
                            .ok_or_else(|| CommandError::Denied("Requester required".into()))?,
                        operation,
                    );
                    let mut rows = journal.lock().await;
                    if let Some(previous) = rows.get(&key) {
                        if previous != &recipient {
                            return Err(CommandError::Denied(
                                "operation key is already bound to different arguments".into(),
                            )
                            .into());
                        }
                        return Ok(()); // Consumer's completed-result policy.
                    }
                    // This INSERT is the complete demo mutation. For external effects,
                    // durably record Pending before sending and reconcile uncertainty;
                    // never turn a Pending/Unknown row into an automatic retry.
                    rows.insert(key, recipient);
                    Ok(())
                }
            },
        )
        .command(
            "record-plan",
            "Assess and execute two records",
            |ctx| async move {
                let message = ctx
                    .message()
                    .ok_or_else(|| CommandError::Denied("Send !record-plan as a message".into()))?;
                let bot = ctx.bot().expect("framework context");
                // In production, load the saved plan and stable IDs from consumer storage.
                // These IDs remain distinct while authenticated message provenance stays intact.
                let mut plans = vec![];
                for (step, recipient) in [("first", "alice"), ("second", "bob")] {
                    plans.push(
                        bot.prepare_invocation_with_operation_id(
                            message,
                            "record",
                            recipient,
                            OperationId::new(format!("{}/{step}", message.message_id()))?,
                        )
                        .await?,
                    );
                }
                // Inspect every report before the first effect. Opaque parsing, cooldowns,
                // cog checks and handler-owned authorization can remain deferred.
                for plan in &plans {
                    eprintln!("deferred: {:?}", plan.report().deferred);
                }
                for plan in plans {
                    let outcome = bot.invoke_prepared(plan).await;
                    // A durable consumer journals outcome.operation_id(), receipts and errors
                    // before propagating an incomplete result. The SDK never retries it.
                    outcome.into_result()?;
                }
                ctx.reply("Both records completed.").await?;
                Ok(())
            },
        )
        .build()
        .expect("valid bot configuration")
        .run()
        .await
}
