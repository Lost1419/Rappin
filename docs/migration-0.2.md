# Migrate from 0.1 to 0.2.0

This development branch contains breaking changes. Keep an existing deployment's
dependency and worker binaries pinned while updating a separate checkout. Use
the `dev` branch for the preview. Published 0.1 references remain version-specific.
If you're upgrading from 0.1.34 or 0.1.35, see the
[late 0.1 migration notes](#migrate-features-added-in-0134-and-0135).

## Update bot construction

In 0.1 a raw command handler received a message context and `build()` returned
the bot directly.

```text
let bot = Bot::builder("example")
    .command("ping", "Check the bot", |ctx| async move {
        ctx.reply("pong").await?;
        Ok(())
    })
    .build();
```

In 0.2.0 the handler receives a `CommandContext` and construction validates
the configuration and command definitions.

```rust
use rappin::{Bot, CommandContext, Result};

fn configure() -> Result<Bot> {
    Bot::builder("example")
        .command("ping", "Check the bot", |ctx: CommandContext| async move {
            ctx.reply("pong").await
        })
        .build()
}
```

## Public API changes

| 0.1 usage | 0.2.0 usage |
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

Gateway transport failures surface as `Error::Gateway`. Use
`gateway_close_code()` to read a WebSocket close code and `status_code()` for an
HTTP status, including an HTTP handshake rejection. Gateway authentication
rejections still satisfy `is_unauthorized()`. Server cooldowns now isolate
DM requesters. Reset a server-scoped DM bucket with `dm:{requester_uuid}`.

## Migrate timestamps and history cursors

Use `rappin::Timestamp` (Jiff's UTC instant type) for timestamp fields and history
filters. Use `Timestamp::now()` for the current instant and
`rappin::parse_timestamp` for an RFC 3339 timestamp from user input.

```rust
use rappin::{parse_timestamp, Embed, Result, Timestamp};

fn dated_embed() -> Result<Embed> {
    let at: Timestamp = parse_timestamp("2026-11-18T12:34:56.123456789Z")?;
    Ok(Embed::new().timestamp(at))
}
```

Models preserve up to nine fractional digits with the existing UTC JSON encoding.
History cursor parameters keep their UTC offset and fractional precision. Durable
event fingerprints and receipts keep their encoding for supported timestamps.
Timestamp inputs with leap seconds or more than nine fractional digits return an
error, which protects against silently changing the instant.

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

All workers require protocol 13. Rebuild the parent and every worker together.
The parent rejects every other protocol version during candidate startup, before
publishing routes. UI callbacks and single-attempt API requests are built in.
Operation IDs travel with checked message-command invocations. Remove `.ui_api()`,
`.operation_identity()` and `.single_attempt_api()`.
Typed and runtime worker commands share registration, aliases, groups and schemas.
Custom `CommandCheck` implementations use one `check(&CommandContext)` for native
and worker commands. Preparation reports
worker checks as deferred and execution evaluates them before the handler.

`InteractionCallback` now stores its payload in `message: EditMessage`. Existing
constructors and builder methods remain available. Direct field access uses
`callback.message.content`, `embeds`, `components` and `flags`. Wire JSON stays flat.
`send_long_message` returns `Error::PartialWrite` on a failed part, including
confirmed messages, the failed step and the original structured cause. Each part
is submitted once, including when the server supplies retry advice.

Initialize worker state with `WorkerCog::state`. Process-local values are created
by each generation. Your application owns database state and durable receipts.
Consumer operation IDs, authenticated message provenance and uncertain-write
outcomes retain their separate meanings across command execution and callbacks.

## Default read recovery and command contexts

`Bot::run` now supervises startup/intake reads by default. Ordinary Client and
handler GET/HEAD requests also have bounded recovery. Configure the two policies
under `TransportConfig::{availability, read_availability}`. Set either to `None`
to disable that policy. Configure known-unapplied write attempts independently
with `max_write_retries`. Explicit `request_once` still requests one attempt.

Command handlers, command checks and capability bindings now use `CommandContext`.
`CommandSpec::bind`, native/cog `command_spec` and worker `command_spec` share the
same handler/check signatures. Slash interaction publication is enabled by default.
There is one command registration path across message and interaction delivery.

| Previous command code | Current command code |
| --- | --- |
| `ctx.args` | `ctx.args()` |
| `ctx.command` | `ctx.command_name()` |
| `ctx.client` / worker `api(...)` | Direct `Api`/`ApiExt` methods on `ctx` |
| `ctx.event` | `ctx.message()`, `ctx.interaction()` or `ctx.worker()` |
| Infallible `Cog::check(&MessageContext)` | `Cog::check(&CommandContext) -> Result<bool>` |
| Separate `check_worker` | One `CommandCheck::check(&CommandContext)` |
| Message-only command failure callback | `on_command_failure(CommandContext, HandlerFailure)` |

Actor/channel/server getters return `Option<Uuid>` when the original delivery
can omit that fact. Handle missing identity before authorizing or selecting a
mutation target. `reply` and `reply_with` return completion for either reply route.
Use `say` for a sent-message receipt, or borrow the original message context for
message-specific APIs. Paginator/confirmation replies return their helper.
`prompt_for_reply` supports native and worker commands with registration before
prompt publication. Process-local `state(Arc<T>)` works in every command context.

Checked invocation still requires an authenticated message. Pass
`ctx.message().ok_or_else(...)` from a command, or the original `MessageContext`
from a listener. Operation IDs and message identity retain their existing scope.
Duplicate command/alias definitions fail at build/registration before network I/O.
Worker protocol 13 carries slash option declarations and change events. It
separates resolved interaction command metadata from its raw source.


## Migrate features added in 0.1.34 and 0.1.35

### Read recovery and shared commands from 0.1.34

| 0.1.34 | 0.2.0 |
| --- | --- |
| `BotBuilder::max_read_retries(n)` / `max_write_retries(n)` | Set `settings.transport.max_read_retries` and `max_write_retries` in `configure` |
| `BotBuilder::read_availability_policy(policy)` | Set `settings.transport.read_availability = Some(policy)`. Set it to `None` to disable request read recovery |
| `Bot::run_supervised(policy)` | Set `settings.transport.availability = Some(policy)` before `build()`, then call `run()` |
| `ClientBuilder` retry and read policy setters | Keep using the setters. `without_read_recovery()` opts out of the new default |
| Native or worker `command_shared` | Register with `command`, or use `command_spec` for typed arguments. Use `registered_command` with `CommandSpec::<()>::bind_raw` for free text plus slash options |
| `ctx.api().get_me()` | Import `ApiExt` and call `ctx.get_me()` |

Startup/intake and request read recovery are separate budgets. A handler that
needs one attempt still uses `request_once`. Use
`Client::{read_availability_status, subscribe_read_availability}` for request
recovery and `BotHandle::{availability_status, subscribe_availability}` for
startup and intake. See [availability and hierarchy](availability-and-hierarchy.md).
The [native example](../examples/shared_commands.rs) and
[worker example](../examples/shared_command_worker.rs) show the shared handler.

### Yapin API additions from 0.1.35

| 0.1.35 | 0.2.0 |
| --- | --- |
| `NovemberApiExt` thread operations and `moderate_reactions` | Import `ApiExt` and use the same operation names. Use a `CreateMessage` for a thread's first message |
| `send_with_nonce(channel, body, nonce)` and `NonceMessage` | Call `ApiExt::send_message(channel, &CreateMessage::new(content).nonce(nonce))`. Read `nonce` and `duplicate` from the returned `Message` |
| `create_private_channel` | Call `create_channel` with a `CreateChannelRequest` configured using `private_audience(roles, users)` |
| `edit_channel_audience` and `ChannelAudience` | Call `edit_channel` with `EditChannelRequest::user_ids(users)` and read the returned `Channel::user_ids`. An empty list clears the member allowlist |
| `SlashCommandApiExt`, `SlashCommand`, and `list_slash_commands` / `publish_slash_commands` | Use `Command` with its `options` and call `ApiExt::get_commands()` / `publish_commands(commands, interactions)` |
| `BotBuilder::slash_options(command, options)` | Attach declarations to `CommandSpec::slash_options` on the published command root |
| `parse_slash_options(&options)` in a shared handler | Call `ctx.parse_slash_options()` on the interaction path. It uses the installed command's declarations |
| `Event::parse_change()` and raw change-event routing | `parse_change()` is still available. `parse_data()` now returns `EventData::Change`; handle it in `on_change_event` or through worker `Listener::ChangeEvent` / `CogEvent::ChangeEvent` |
| `structured_api_errors(true)` | Remove the setter. Read structured platform fields through `Error::to_remote()` or `Error::to_json()` by default |
| Unpublished `SdkPackages::npm` / `pypi` represented as empty strings | The fields are `Option<String>`. Check for `None` |
| Classic ephemeral reply buttons and `reply_id` | Use `InteractionCallback` with the existing ephemeral/update constructors and the shared `EditMessage` payload. The original `reply_id` still identifies the reply |

The following native registration handles message commands and slash forms.
Workers use the same registration and parser. In a worker handler, test for
`CogEvent::InteractionCommand` via `ctx.worker()` to identify the slash path.

```rust
use rappin::{Bot, CommandSpec, Result, SlashOption, SlashOptionKind};

fn configure() -> Result<Bot> {
    let find = CommandSpec::<()>::new("find", "Find matching items")
        .slash_options([SlashOption::new("query", SlashOptionKind::String)])
        .bind_raw(|ctx| async move {
            if ctx.interaction().is_some() {
                let values = ctx.parse_slash_options()?;
                let query = values.get("query").and_then(serde_json::Value::as_str).unwrap_or("");
                ctx.reply(format!("Query: {query}")).await
            } else {
                ctx.reply(format!("Query: {}", ctx.args())).await
            }
        });
    Bot::builder("unused-token").registered_command(find).build()
}
```

On the CLI and MCP, use `send_message` with the original nonce inside
`body.nonce` in place of `send_with_nonce`. The shared tool response contains
one entry under `messages` for a nonce send. Inspect that entry's `message`,
`nonce` and `duplicate` fields. The direct `messages send` command also accepts
a body file with a nonce. Keep the same caller-supplied nonce for the same
file-free channel send within Yapin's deduplication window. The request uses one
attempt. See [CLI and MCP workflows](cli-mcp.md#threads-reactions-and-slash-publication).

The CLI/MCP operation names for threads, reaction cleanup, and slash publication
are unchanged. For private channels, use the existing `create_channel` and
`edit_channel` operations with audience fields inside `body`. Platform rollout,
permissions, and the bot's own access still apply. A successful local preview
does not establish visibility or permission for that server.

## Free-form commands and direct messages

Use `CommandSpec::<()>::bind_raw` with `registered_command` on a bot, cog or
worker to keep advertised slash options and checks while parsing `ctx.args()`
yourself. Typed command specs continue to tokenize their arguments.

Message-origin and interaction-origin handlers share `ApiExt` methods for
`get_server`, `get_member`, `get_roles`, `get_channel` and `delete_message`.
The typed reads expose resources visible to the bot. Requester authorization
belongs to the handler or an operation binding.

DMs go to `on_dm` by default. Opt in to prefix command dispatch with
`config.execution.dm_commands`. The bot uses the same installed routes
and check chain for DM commands and preserves the original DM identity.
Read availability reports expose a credential-safe `transport_kind` for HTTP
failures. A connection failure or timeout is represented as a failed read in
the nested cause, while handler and mutation errors keep their existing
workflow and write-outcome semantics.
