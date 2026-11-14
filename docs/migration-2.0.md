# Migrate from 0.1 to 2.0

This development branch contains breaking changes. Keep an existing deployment's
dependency and worker binaries pinned while updating a separate checkout. Use
the `dev` branch for the preview. Published 0.1 references remain version-specific.

## Public API changes

| 0.1 usage | 2.0 usage |
|---|---|
| `Bot::builder(token).build()` | `Bot::builder(token).build()?` validates definitions and configuration before startup |
| Manual environment lookup and `Config` conversion | `Bot::from_env()?` returns a builder using `YAPPER_BOT_TOKEN` |
| `YappinError` | `Error` retains structured variants, causes, status, retry hints and receipts |
| `ApiExt`, `UiApiExt`, `ResourceApiExt` | Import `ApiExt` for the shared native/worker typed operations |
| `ui_api` module | Message helpers live on `ApiExt` and `Client` |
| `CreateMessage::with_content`, `with_reply_to` | `content`, `reply_to` |
| `CreateMessage::layout(components, reply)` | `layout(components)` and optional `.reply_to(id)` |
| `WorkerContext::respond(callback)` | `respond_callback(callback)` for raw callbacks, or shared `ApiExt::respond(event, kind, body)` |
| `api_1_2_guide`, `api_1_3_guide`, `api_1_4_guide` | `uploads_and_webhooks_guide`, `interactions_guide`, `resources_guide` |
| `UiMessageBody` for creation | `CreateMessage`, including selects, layouts and standalone stickers |
| `UiMessageBody` for edits | `EditMessage`, with omission and explicit null clearing |
| `UiMessage`, `UiMessagesResponse` | `Message`, `MessagesResponse`, with retained unknown fields |
| `UiComponent` | `Component`, including a lossless `Raw` response representation |
| `UiInteractionContext` | `InteractionContext`, with `values`, `command`, `args`, `parse` and tracked response methods |
| `on_message_result` | Fallible `on_message` returning `Result<MessageDisposition>` |
| Void event/cog listeners | Return `Result<()>`; message listeners return an explicit disposition |
| `on_ui_interaction` cog listener | Fallible `on_interaction` for buttons, selects and forms |
| `on_command_error` / `on_button_error` | `on_command_failure` / `on_button_failure`, returning an explicit disposition |
| Captured state cloned into each handler | Optional `.state(Arc<T>)` and `ctx.state::<T>()?` |
| Separate worker typed definitions | `WorkerCog::command_spec` accepts the native `CommandSpec<A>` metadata and checks |

The smaller `prelude` contains common bot types. Import advanced policy, transport,
collector and persistence types explicitly from their modules or the crate root.

## Move advanced configuration

`BotBuilder::configure` exposes typed transport, execution, persistence and cache
groups. Changes apply in builder order. Use `transport(TransportConfig::gateway())`
for the default gateway and polling fallback.

| Previous setter | Field inside `configure` |
|---|---|
| `handler_concurrency(n)` | `settings.execution.concurrency = n` |
| `pending_batches(n)` | `settings.execution.pending_batches = n` |
| `drain_timeout(value)` | `settings.execution.drain_timeout = value` |
| `max_event_age(seconds)` | `settings.execution.max_event_age = Some(seconds)` |
| `auto_ack_after(delay)` | `settings.execution.auto_ack_after = Some(delay)` |
| `availability_policy(policy)` | `settings.transport.availability = Some(policy)` |
| `transport_mode(mode)` | `settings.transport.mode = mode` |
| `gateway(url)` / `gateway_only(url)` | Set `settings.transport.gateway_url` and the corresponding mode |
| `cursor_file(path)` | `settings.persistence = PersistenceConfig::file(path)` |
| `shared_ingress(config)` | `settings.persistence.ingress = Some(config)` |
| `ingress_store(store)` | `settings.persistence.store = Some(store)` |

```rust
use rappin::{Bot, bot::{PersistenceConfig, TransportConfig}};
let bot = Bot::builder("unused-token")
    .transport(TransportConfig::polling())
    .configure(|settings| {
        settings.execution.concurrency = 1;
        settings.persistence = PersistenceConfig::file("./state/events.json");
    })
    .build()?;
# Ok::<(), rappin::Error>(())
```

Build rejects invalid scheduling/draining limits and endpoint configuration.
Application-owned caches use `settings.cache.backend`; in-memory lookup policy
uses `settings.cache.policy`. Dynamic prefixes use `prefixes_with`.

## Generate argument parsing and help

Enable the optional `derive` feature. Replace a manually synchronized
`FromArgs::parse`/`describe` pair with a concrete named-field struct.

```rust
use rappin::FromArgs;
#[derive(FromArgs)]
struct Roll {
    /// Number of sides.
    #[arg(default = 6, min = 2, max = 100)]
    sides: u32,
}
```

Manual `FromArgs` implementations continue to describe custom parsers. The new
optional `schema` method supplies pure read-only argument validation. Preparation
reports unavailable schemas and arbitrary checks as deferred. Execution rechecks
the installed command and runs its parser and authorization before the handler.

## Update messages and interaction handlers

Use the same `CreateMessage`/`EditMessage` and response models through Client and
`ApiExt`. `get_message` returns `MessageDetailResponse` with its `message` and
optional `author`. `send_message` and `edit_message` return `Message`.

```rust
let patch = rappin::EditMessage::new().clear_content().clear_components();
assert_eq!(serde_json::to_value(patch)?, serde_json::json!({"content":null,"components":null}));
# Ok::<(), serde_json::Error>(())
```

`InteractionContext` holds one original `event` identity plus typed `values` and
command arguments. Use `ctx.values` in selects/forms. Resource-specific SDK IDs
are accepted by typed API methods. UUID interoperability is available through
the ID conversion implementations. IDs carry resource kind information, and
authorization still requires the original authenticated context and fresh evidence.

## Make listener completion explicit

```rust
use rappin::{EventHandler, MessageContext, MessageDisposition, Result};
struct Greetings;
#[async_trait::async_trait]
impl EventHandler for Greetings {
    async fn on_message(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
        if ctx.content() == "hello" { ctx.reply("Hello!").await?; }
        Ok(MessageDisposition::Continue)
    }
}
```

`on_message` runs after normal command dispatch. The shared message gate runs
before dispatch. Return `Handled` only after taking responsibility for completion.
For error callbacks, return `FailureDisposition::Default` to retain default
classification while recording the structured cause. Expected rejections complete
after their reply succeeds. Failed replies, partial writes and unknown outcomes
retain the event barrier. Logging an error alone cannot establish completion.

## Rebuild workers

New worker command specs require protocol v10 for schemas, aliases and groups.
Upgrade the parent before enabling those definitions. The host negotiates protocol
support before installing routes. Existing protocols retain their wire versions.
Built-in checks execute through the parent-authenticated API. Custom
`CommandCheck` implementations need `check_worker` for worker use. Preparation
reports worker checks as deferred, then execution evaluates them again.

Initialize worker state with `WorkerCog::state`. Process-local values are created
by each generation. Your application owns database state and durable receipts.
Consumer operation IDs, authenticated message provenance and uncertain-write
outcomes retain their separate meanings across command execution and callbacks.
