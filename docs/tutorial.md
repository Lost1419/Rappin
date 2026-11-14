# Build your first bot

This tutorial assumes you can write a Rust struct, an async function and a
function returning `Result`. Work through the sections in order. The generated
project enables the `gateway` and optional `derive` features and prints startup logs.

## 1. Get a response

Install the development CLI and create a project using the commands in the
[README](../README.md#start-a-bot). Create a bot in Yapin's developer settings,
add it to your server, and set `YAPPER_BOT_TOKEN` in the bot process environment.
Run `cargo run` in the generated directory. Send `!ping` and expect `pong`.

The bot receives events, selects a registered command, and calls its async
handler. `ctx` contains the original message and methods for replying. Await the
reply and use `?` to retain an API failure. `Ok(())` completes the handler.

If the bot fails to start, check the token, Rust version and server membership.
Run `rappin doctor --human` for a bounded authenticated read, or add `--offline`
to inspect local configuration. The CLI reads its own configuration and process
environment. A token available to one process must also be available to the other.

## 2. Add typed arguments

The runnable [tutorial bot](../examples/tutorial_bot.rs) includes this argument
type and a `greet` command. Add the struct above `main` and register the command
before `.build()?`.

```rust,no_run
use rappin::prelude::*;

#[derive(FromArgs)]
struct Greet {
    /// Person to greet. Quote a name containing spaces.
    name: String,
    /// Optional greeting suffix.
    #[arg(rest, max = 80)]
    note: Option<String>,
}

fn add_greet(builder: BotBuilder) -> BotBuilder {
    builder.command_spec(CommandSpec::<Greet>::new("greet", "Greet someone"),
        |ctx, args| async move {
            ctx.reply(format!("Hello, {}! {}", args.name, args.note.unwrap_or_default())).await?;
            Ok(())
        })
}
```

Send `!greet "River Song" welcome back`. Send `!help greet` to see generated help.
A missing name receives an input-error reply. Required fields come first,
`Option<T>` fields are optional, and a final string can consume the remaining
words with `#[arg(rest)]`. Numeric fields support literal defaults and inclusive
`min`/`max` bounds. String bounds count Unicode characters.

The derive supports concrete structs with named fields. Custom field types use
their `FromStr` implementation. Write `FromArgs` manually for a custom parser.
Read-only invocation preparation reports custom parsing as deferred.

## 3. Add a button

Register a command that posts a row and a handler for its custom-ID prefix.

```rust,no_run
use rappin::prelude::*;
fn add_button(builder: BotBuilder) -> BotBuilder {
    builder
        .command("menu", "Show a button", |ctx| async move {
            ctx.reply_with(CreateMessage::new("Choose a fruit")
                .component(Component::row([Button::primary("Apple", "fruit:apple")]))).await?;
            Ok(())
        })
        .button("fruit:", |ctx| async move {
            ctx.ephemeral("You chose Apple").await?;
            Ok(())
        })
}
```

Send `!menu`, click Apple and expect a private confirmation. Command handlers receive `CommandContext` for either prefix messages or slash
interactions. Buttons, selects and forms receive `InteractionContext`. Natural-language
message listeners retain `MessageContext`.
Interaction replies have deadlines and shared acknowledgement ownership.
See the [interaction guide](interactions.md) for defer, update, follow-up and modal flows.

## 4. Share application state

Put your database pool, configuration or counters in an application-owned struct.
Register an `Arc` once, then retrieve that type in a handler.

```rust,no_run
use rappin::prelude::*;
use std::sync::Arc;
struct Settings { greeting: String }

fn configured() -> Result<Bot> {
    Bot::from_env()?
        .state(Arc::new(Settings { greeting: "Hello".into() }))
        .command("hello", "Read shared settings", |ctx| async move {
            let settings = ctx.state::<Settings>()?;
            ctx.reply(&settings.greeting).await?;
            Ok(())
        })
        .build()
}
```

Missing state produces an explicit error. Duplicate registrations of one type
fail during build. Your application controls mutation, synchronization and
durability. Each worker process initializes its own state for its generation.

## 5. Restrict a command

Attach a check to the command definition. This example permits the bot owner.

```rust,no_run
use rappin::prelude::*;
fn owner_command(builder: BotBuilder) -> BotBuilder {
    builder.command_spec(
        CommandSpec::<()>::new("owner", "Check bot ownership").check(Check::owner_only()),
        |ctx, ()| async move { ctx.reply("You own this bot").await?; Ok(()) })
}
```

Server membership, role and server-permission checks have their own constructors.
Resource mutations also require the relevant channel/category or hierarchy
authorization. Assess requester and bot separately using the
[resource permission APIs](resources.md). Missing platform evidence produces
`Unknown`; a successful bot read supplies information about the bot's access.

## 6. Organize commands into cogs

Use the same command specs inside a native cog.

```rust,no_run
use rappin::prelude::*;
use rappin::CogBuilder;
fn modular(builder: BotBuilder) -> BotBuilder {
    builder.cog(CogBuilder::new("utilities")
        .command_spec(CommandSpec::<()>::new("status", "Check the cog"),
            |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
        .build())
}
```

Native cog replacement selects code already linked into the running process.
Use a reloadable worker for edited Rust source. See [cogs and hot reloading](hot-reloading.md).

## 7. Reload edited Rust

Create a separate project with `rappin init worker-bot --template worker --human`.
Run it with the token available to the parent. Send `!ping`, edit
`examples/worker.rs`, save, wait for a successful replacement, then send `!ping` again.
The parent retains the connection and bot credential. A failed candidate keeps
the installed routes serving and exposes structured diagnostics.

Worker command specs support the same argument metadata, aliases and groups.
Built-in checks run through the parent-authenticated API. Custom checks need a
worker implementation. Read-only preparation reports worker checks as deferred.
These specs require protocol 12. Build the parent and workers from the same
development revision. Compilation executes trusted build scripts and procedural macros.

## 8. Understand completion before deploying

The starter stores checkpoints in `state/events.json`. Give a checkpoint one
runner. On shutdown, intake stops and accepted work drains. Higher concurrency
permits overlapping handlers; checkpoint progress follows ordered completion.

Expected `CommandError` input or permission rejections finish after the required
reply succeeds. Failed replies, unexpected errors and partial/uncertain effects
retain a checkpoint barrier. The SDK never automatically reruns a handler.
Use [explicit failure callbacks](handler-failures.md) when the application can
complete or reconcile a failure. Returning an error preserves the structured cause.

Use `configure` for deployment settings. For example, set
`settings.execution.concurrency = 1` for ordered handler execution, or supply a
custom lookup backend through `settings.cache.backend`. Applications select their
own databases, dynamic prefixes, approvals and durable replay policy.

Run the complete tutorial example from an SDK checkout using
`cargo run --features gateway,derive --example tutorial_bot`.
