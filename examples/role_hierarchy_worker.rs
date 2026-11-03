//! Read-only hierarchy assessment in a worker; no credential enters this process.
use rappin::{capabilities::*, *};
use serde_json::json;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    WorkerCog::new("role-assessment")
        .single_attempt_api()
        .command(
            "role-check",
            "Assess a role UUID without deleting it",
            |ctx| async move {
                let message = ctx
                    .event
                    .message()
                    .ok_or_else(|| CommandError::Denied("server message required".into()))?;
                let server = message
                    .server_id
                    .ok_or_else(|| CommandError::Denied("server message required".into()))?;
                let context = OperationContext::server(server, message.author_id);
                let role =
                    ctx.args()
                        .trim()
                        .parse::<Uuid>()
                        .map_err(|_| CommandError::Invalid {
                            name: "role".into(),
                            reason: "expected UUID".into(),
                        })?;
                let operation = capability("delete_role")
                    .unwrap()
                    .validate(&json!({"id":server,"role":role}))?;
                let policy = RoleHierarchyPolicy::new(&ctx).owner_admin_strict();
                let report = operation
                    .assess_with_policy(&ctx, &context, &policy)
                    .await?;
                // Availability stays Unknown without independent host evidence.
                // To execute with a composed host policy, call execute_role_deletion
                // with the same Api/operation/context; never replay the command.
                ctx.reply(serde_json::to_string_pretty(&report)?).await
            },
        )
        .run()
        .await
}
