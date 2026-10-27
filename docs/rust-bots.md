# Rust bot workflows

These APIs are available starting with `rappin-rs` 0.1.7. Existing `.command(name, description, |ctx| async move { ... })`
handlers continue to work. Start with `cargo run --example typed_bot` and set
`YAPPER_BOT_TOKEN` in the environment.

## Typed commands and live help

Use ordinary Rust structs and `FromStr` parsers. No macro expansion, runtime type
coercion, or automatic member lookup is involved:

```rust,no_run
use rappin::prelude::*;

struct Roll { sides: BoundedU32<2, 100>, label: Option<String> }
impl FromArgs for Roll {
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError> {
        Ok(Self { sides: args.required("sides")?, label: args.optional("label")? })
    }
    fn describe() -> Vec<ArgumentInfo> {
        vec![ArgumentInfo::required("sides", "2 to 100"),
             ArgumentInfo::optional("label", "A label; quote spaces")]
    }
}

let bot = Bot::builder("yb_token")
    .command_spec(CommandSpec::<Roll>::new("dice roll", "Roll a die").alias("roll"),
        |ctx, args| async move {
            let value = uuid::Uuid::new_v4().as_u128() % u128::from(args.sides.0) + 1;
            ctx.reply(format!("{}: {value}", args.label.as_deref().unwrap_or("Roll"))).await?;
            Ok(())
        })
    .help_command("help")
    .build();
```

`!dice roll 20 "lucky die"`, `/dice roll 20`, and `!roll 20` share one handler.
Only the root `dice` is advertised to Yapin; nested paths and aliases are local
routing. Names are normalized to lowercase and paths have at most eight segments.
Invalid arguments and denied checks receive an error and usage reply without
terminating the event loop. Transport failures still follow normal error handling.
The same specs can be registered with `CogBuilder::command_spec` or returned by
`Cog::registered_commands`. Metadata, checks, and aliases follow cog replacement.
`BotHandle::command_info()` and `help(path)` reflect the current registry.

`Arguments` supports single/double quotes, backslash escapes, optional values, and
`rest()` (remaining parsed tokens joined with spaces). It never expands variables
or executes a shell. `HumanDuration` accepts `1h30m`, `UserId` accepts UUIDs or
`<@uuid>`, and `ChannelId` accepts UUIDs or `<#uuid>`. Resolve names explicitly with
Client methods. `describe()` is manually maintained help metadata; keep it aligned
with `parse()`. Existing `ctx.args` stays raw; `ctx.argv` now respects quotes and
falls back to whitespace splitting for malformed quotes in legacy handlers.

`prefixes_with(|message| async move { Ok(vec![...]) })` replaces static prefixes
for each message. `mention_prefix(true)` additionally enables `<@bot-id> command`
and `<@!bot-id> command`; mention prefixes are off by default. Empty prefixes are
ignored and longer matches take precedence.

## Reusable checks

Attach `.check(Check::owner_only())`, `.check(Check::server_only())`,
`.check(Check::roles([role_id]))`, or
`.check(Check::permissions([Permission::manage_messages()]))` to a CommandSpec.
Roles means **any** listed role; permissions means **all** listed permissions.
Server owners pass permission checks. Other members need an explicitly assigned
role granting the permission. Unknown automatic-role semantics are not inferred;
the API still authorizes every action and enforces channel overrides/hierarchy.
Bot-owner checks use the bot profile snapshot, separate from server ownership.

Membership and role checks refresh by default. `.freshness(Freshness::PreferCache)`
is an explicit opt-in to the configured cache TTL; revocations can remain invisible
until expiry or invalidation. Prefer fresh checks for moderation actions. Custom
checks implement `CommandCheck` or use `check_fn`:

```rust,no_run
use rappin::prelude::*;
let spec = CommandSpec::<()>::new("maintain", "Owner maintenance")
    .check(Check::owner_only())
    .check(check_fn(|ctx| async move {
        if ctx.server_id().is_none() {
            return Err(CommandError::Denied("Use a server channel".into()).into());
        }
        Ok(())
    }));
```

## Waiters and component collectors

Register **before** posting a prompt. `subscribe()` is synchronous; a builder's
`.wait().await` registers only when polled.

```rust,no_run
use rappin::prelude::*;
use std::time::Duration;
async fn ask(ctx: MessageContext) -> Result<()> {
    let reply = ctx.replies().timeout(Duration::from_secs(30)).subscribe()?;
    ctx.reply("What is your name?").await?;
    match reply.wait().await {
        Ok(message) => { ctx.reply(format!("Hello, {}", message.content.as_deref().unwrap_or("friend"))).await?; }
        Err(WaitError::Timeout) => { ctx.reply("Time is up.").await?; }
        Err(WaitError::Closed) => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
```

`ctx.replies()` scopes messages to the invoking author/channel.
`ctx.component_events(message_id)` scopes interactions to that message, channel,
and user; add `.custom_id_prefix("vote:")` or a typed `.filter(...)`.
For overlapping prompts, scope by message ID or give each prompt a unique custom-ID
prefix. A shared prefix alone does not isolate multiple prompts. If the message ID
is only available after posting, register a collector with a unique per-prompt
prefix before sending, then verify the returned interaction's message ID.
For other workflows use `bot.events().collect::<MessageEvent>()` or the interaction,
reaction, membership, voice, or raw Event types. Builders support absolute timeout,
idle timeout, item limit, capacity, and serializable `EventFilter` criteria.
Collectors implement `Stream`; `.next()` is also available without StreamExt.

Each subscription is bounded (default 64, maximum 4096 items), with at most 1024
subscriptions per Client. Overflow is a terminal `WaitError::Lagged`, not silent
loss. Drop unregisters; `.cancel()` explicitly closes a collector. Shutdown closes
active waiters. Handle `Closed` if shutdown should finish normally. Filters run
synchronously outside registry locks and should be quick. Collectors observe raw
received events before handler scheduling, including events ordinary command
filters may ignore. They are ephemeral observations, not durable event consumers.
Custom transports can call `Bot::process_event`; `EventHub::publish` only fans out
and does not dispatch handlers, invalidate caches, or commit checkpoints.

## Bounded event intake and checkpoints

The transport reads ahead while handlers run, so a handler can wait for a later
reply and gateway heartbeats continue during ordinary handler work. A global
`handler_concurrency` limit (default 8, 1–100) applies across batches; 1 preserves
handler event ordering. `pending_batches` (default 4, 1–32) bounds both the intake
queue and active batch futures separately. Normal API batches contain at most 100
events, so this is a batch bound rather than a byte-size bound.

A received cursor tracks prefetched delivery. The durable committed cursor moves
only after whole batches succeed, in delivery order. Failed batches remain
uncommitted even when a later handler finishes first. Delivery is at least once:
use idempotent actions where replay matters. One process must own a cursor file.

When queues fill, intake pauses. A gateway connection is dropped and later resumed
instead of leaving its heartbeat unattended; already received events stay queued.
Long waits can therefore exert backpressure under sustained traffic. Keep waiters
finite and choose concurrency/capacity for the workload. Shutdown stops intake,
cancels waiters, drains active handlers, and aborts pending automatic ack tasks.
Queued batches that were never started are replayed from the committed cursor.
`on_ready` runs before intake starts; initialize there, but wait for user events
from command handlers or tasks that outlive that callback.

## Transport and REST diagnostics

`BotHandle::transport_status()`/`subscribe_transport()` expose configured mode,
active transport, fallback reason, heartbeat RTT, received/committed cursors, and
active batch count. `Gateway::status()` offers the same transport view for direct
users. Defaults remain polling. With `gateway` enabled, `.gateway(url)` prefers
WebSocket with REST fallback; `.gateway_only(url)` fails if WebSocket cannot be
used. `.transport_mode(TransportMode::GatewayPreferred)` uses the default gateway.
Gateway mode without that Cargo feature returns a configuration error.

```rust,no_run
use rappin::prelude::*;
async fn monitor(handle: BotHandle) {
    let mut changes = handle.subscribe_transport();
    while changes.changed().await.is_ok() {
        eprintln!("{:?}", *changes.borrow_and_update());
    }
}
```

`Client::last_request()`/`subscribe_requests()` report method, sanitized route
shape, status, attempts, time waiting on rate limits, and elapsed execution through
response headers. These are latest-value watch streams; intermediate observations
can coalesce. Route diagnostics omit IDs, query strings, and callback tokens.
Elapsed REST time includes retries but excludes reading/deserializing the final
body. Heartbeat RTT is WebSocket-only and clears on disconnect. Neither measures
end-to-end chat/UI latency.

## Interaction response ownership

All Client clones, native contexts, and worker API proxies share bounded response
state per interaction (up to 4096 entries, retained for 15 minutes).
`InteractionContext::response_state()` reports Unanswered,
Acknowledged, Responded, Uncertain, or Expired. `.defer()` aliases `.ack()`; repeated
acks after a successful response are local no-ops. Replies/updates after an ack
remain real API calls, subject to Yapin's limits. Separate Client instances do not
share this state.

Explicitly call `.defer().await?` before slow button work, or opt into
`.auto_ack_after(Duration::from_millis(250))` on BotBuilder. Automatic acks start at
intake, account for delivery age when available, and reserve one second of the
initial deadline for the API. They are best effort, disabled by default, and
bounded to 128 pending tasks. They cannot make a late-delivered event or a slow API
meet its deadline. Manual and automatic responses serialize per interaction.
Cancellation/transport failure leaves Uncertain and prevents blind retries of an
ambiguous write; it does not prove whether the server posted the message.

## Rate limits and targeted caching

Client clones share server-advised retry deadlines, scoped by method/route and
major channel/server ID. A known global message limit delays channel sends while
leaving interaction callbacks eligible. Numeric and HTTP-date Retry-After values
are supported; individual advised delays over 60 seconds are rejected for
retry scheduling, with the existing retry budget. Headerless 429s and ambiguous network writes are not retried. There is no
speculative rate scheduler or blanket serialization of normal REST traffic.

Profile/server/channel snapshot caching remains explicit. New member/role lookup
caches default to disabled. Enable `.lookup_cache(CachePolicy::new(256,
Duration::from_secs(30)))` on ClientBuilder or BotBuilder; capacity applies to each
lookup cache, expiry uses a monotonic clock, and eviction removes the oldest
insertion. `fetch_member`/`fetch_roles` require a `Freshness` choice. Existing
`get_member`/`get_roles` always request the API. Cache getters never perform I/O;
`Cached::age()` reports snapshot age. Successful membership/role mutations
invalidate the server's lookup caches, and observed joins/leaves invalidate that
member. Unobserved changes elsewhere remain subject to TTL. Use
`invalidate_member`/`invalidate_server_lookups` when external changes are known.

## Hot workers

`WorkerCog::command_typed` uses the same FromArgs parser and advertises its help to
the parent. `WorkerContext::parse` works in existing handlers. Parent-side native
CommandSpecs can enforce checks; a worker may also explicitly request authorized
resource lookups through its API proxy. Closures are never serialized over IPC.

`WorkerContext::prompt_for_reply(text, timeout)` asks the parent to register a
same-user/channel message waiter before posting the prompt. `wait_for_event`
accepts an EventFilter for other flows. Each wait is bounded to five minutes and
also subject to WorkerProject's handler timeout, which includes queueing and API
work. Waits count toward the 128 requests per invocation budget. In-flight old
workers can finish their existing waits during reload; subscriptions do not migrate
into the new process. Worker protocol v2 adds help/waits; the host still accepts
v1 workers. Rebuild newer workers with a compatible host.

See [hot-reloading](hot-reloading.md) and `examples/hot_cog_worker.rs`. There is no
JavaScript-style manager hierarchy, hidden resource fetching, general middleware
pipeline, or advanced voice player added by these APIs.
