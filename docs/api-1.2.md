# Files, emojis, bot webhooks and DMs

Rappin supports the public Yapin API 1.2 specification. Message uploads, custom
emojis, bot-owned webhooks and DM delivery remain subject to Yapin's test-server
or tester rollout. The SDK preserves availability and permission failures; it
cannot enable an upstream feature for a server.

## Uploads

`Client::send_files` and `ApiExt::send_files` take reusable `UploadFile` byte
buffers and return every confirmed `Message`, not just the first attachment.
They never read a path implicitly. Limits: one to four files, 10 MiB per file,
12 MiB combined. Names cannot contain path separators or control characters.
Supported extensions are PNG/JPG/GIF/WebP, MP4/WebM/MOV, MP3/OGG/WAV/M4A, TXT,
PDF and ZIP. Yapin checks actual file contents, sound duration and upload permissions.

```rust,no_run
use rappin::{Client, CreateMessage, UploadFile, Result};
use uuid::Uuid;
async fn upload(client: &Client, channel: Uuid) -> Result<()> {
    let files = [UploadFile::new("report.txt", b"Generated report".to_vec())];
    let messages = client.send_files(channel, &CreateMessage::new("Report"), &files).await?;
    for message in messages {
        println!("{} {:?} {:?}", message.id, message.attachment_name, message.attachment_size);
    }
    Ok(())
}
```

A 201 response can contain confirmed `messages` **and an error for remaining
files**. The typed helper returns `PartialWrite` with confirmed receipts and the
next failed step; it also rejects a missing/mismatched per-file confirmation.
The remaining outcome is conservative/unknown. Do not replay the batch. Raw
`Client::request_upload` returns the response envelope for callers managing their
own reconciliation. `UploadedMessages::into_result_for(count)` applies the same
receipt checks. Filename extensions are not proof that content is safe.

Buffers use `Bytes`, so client retries clone references instead of file contents.
Only the existing server-advised 429/503 request retries apply. Transport failures,
ambiguous writes, partial HTTP-success responses and whole handlers are not retried.

## Custom emojis and bot-owned webhooks

The same helpers are available on `Client` and through `ApiExt` for native
contexts, workers and custom adapters:

| Workflow | Methods |
|---|---|
| Emojis | `get_emojis`, `create_emoji`, `upload_emoji`, `edit_emoji`, `delete_emoji` |
| Bot webhooks | `get_channel_webhooks`, `get_bot_webhooks`, `create_bot_webhook`, `delete_bot_webhook`, `post_bot_webhook` |
| Public metadata | `get_api_version`, `get_api_changelog` |

Emoji names require 2–32 ASCII letters/digits/underscores. Pictures are PNG, GIF
or WebP up to 1 MiB, supplied as JSON base64/data URLs with
`CreateEmoji::new(name, image)` or as `UploadFile` with `upload_emoji`.
`EmojiCreateResult { emoji: None, pending: true, .. }` means moderation is pending,
not a failed write. Do not upload again just because no emoji is returned yet.

Create webhooks with `CreateBotWebhook::new(name)` and post a `WebhookMessage`.
Bot-owned webhook operations use the parent bot's authentication; they do not
require or expose a webhook token. The existing `WebhookClient` is still the
separate channel-webhook URL/token interface. Per-message username/avatar,
link-button components and `allowed_mentions` remain supported. Staff-name rules,
actual ownership, image checks and effective permissions stay authoritative API checks.

Catalogue bindings verify server/channel references and bot ownership before
webhook mutations. Requester and bot grants remain independent. Scoped bot-webhook
lists filter other servers and require host evidence for visibility. Unknown
channel/category access is never inferred from a successful bot read or a
server-wide grant. Direct Client/Api helpers remain available for applications
that own their authorization layer.

## Incoming DMs and reactions

`EventData::DmMessage` carries `DmMessageEvent`, which exposes a `MessageEvent`
through `.0`/Deref. It preserves `attachments`, `author_name`, `dm`, `is_dm` and
`dm_channel_id`. Typed DM decoding rejects server scope and clears any command
metadata. DMs never invoke the bot's server commands or prefix resolver.

```rust,no_run
use rappin::prelude::*;
struct Handler;
#[async_trait::async_trait]
impl EventHandler for Handler {
    async fn on_dm(&self, ctx: &MessageContext) -> Result<()> {
        ctx.reply("Your DM arrived.").await?;
        Ok(())
    }
}
let bot = Bot::builder("yb_token").event_handler(Handler).build();
```

`EventHandler::on_dm` and `Cog::on_dm` return `Result<()>`: finish required work
before returning `Ok`; errors retain the event's checkpoint barrier. Typed
`CommandError` automatic rejection replies apply to commands/buttons, not DM
listeners. Handle known DM rejections explicitly. Do not discard failed replies.

Use `collect::<DmMessageEvent>()` for DM-only collectors. MessageEvent collectors
can receive both server messages and DMs; channel/server filters remain available.
Reaction events retain `dm`, `is_dm`, `dm_channel_id` and nullable `server_id`.
Native cogs also have fallible reaction hooks. Existing EventHandler reaction
callbacks keep their `()` observer contract. DM events go to `on_dm`, replacing
raw unknown-event delivery for that newly supported event type.

With `hot-reload`, workers use `Listener::DmMessage`, `ReactionAdd` and
`ReactionRemove`, with corresponding `CogEvent` variants. `WorkerContext` keeps
its parent proxy and gains the same `ApiExt` helpers. Protocol v4 adds public
reads and uploads: files cross fixed 64 KiB chunks, bounded to the same 12 MiB
total; ordinary frames remain limited to 1 MiB. Incomplete/invalid transfers do
not start the HTTP upload. Cancelled exchanges remain unusable and require
reconciliation before an explicit replacement. Rebuild parent and workers
together; the new parent continues accepting older v1–v3 workers.

## API metadata, retry budgets and image proxies

`Client::new("").get_api_version()` and `get_api_changelog(1..=50)` perform
unauthenticated reads. Client/native/worker public reads omit Authorization;
custom Api adapters may override `public_request`. No automatic update check or
version print is added to bot startup or MCP stdout.

Request observations expose optional `api_version` and `api_build` from
`X-Yapin-API-Version` and `X-Yapin-Build`. They describe the responding server;
REST timing and gateway heartbeat RTT remain separate measurements.

The total rate-limit wait budget defaults to 30 seconds, including shared route
and global message waits. Configure it with
`Client::builder(token).max_retry_wait(Duration::from_secs(10))`, the matching
`BotBuilder` method, CLI `--max-retry-wait 10`, or TOML `max_retry_wait = 10.0`.
HTTP duration is separate. Retry counts remain independently configurable on
ClientBuilder. If advice cannot fit, the SDK returns `RetryWaitExceeded` without
sleeping or submitting another request; the original status/retry hint is retained.
This never schedules a handler restart. Cloudflare HTML 429 responses are identified
in tracing without logging raw bodies or credentials.

`EmbedMedia::proxy_url` and `EmbedAuthor`/`EmbedFooter::proxy_icon_url` survive
response serialization and worker/receipt boundaries. Typed outbound helpers strip
these server-generated hints when reusing an embed. The original URL remains
available. Rappin does not add an image-host allowlist or fetch those images itself;
Yapin's proxy rollout, HTTPS rules and content checks remain server-enforced.

## CLI and MCP

```sh
rappin platform get_api_version
printf '%s\n' '{"limit":1}' | rappin platform get_api_changelog --arguments-file -
rappin upload "$CHANNEL" ./report.txt ./picture.png --body-file message.json
rappin emoji-upload "$SERVER" party ./party.gif --dry-run
```

The 56-operation catalogue is shared by Rust, CLI and MCP. New named operations:
`get_api_version`, `get_api_changelog`, `send_files`, `list_emojis`, `create_emoji`,
`edit_emoji`, `delete_emoji`, `list_channel_webhooks`, `list_bot_webhooks`,
`create_bot_webhook`, `delete_bot_webhook`, `post_bot_webhook`.

For `platform`, JSON arguments are optional only when the operation needs none;
otherwise supply `--arguments-file PATH` or `-` for stdin. `tools call` exposes the
same catalogue. MCP `send_files` takes `files: [{ name, data }]`, where data is
base64, and optional CreateMessage `body`. Emoji creation uses an inline
`{ name, image }` body. The existing **128 KiB inline-argument limit still applies**,
so larger files/images use the Rust SDK or the operator CLI upload commands.
MCP cannot select file paths. Dry runs omit file/base64 data, show arguments and
never assert authorization. Collection limits/field selection apply to emoji and
webhook lists. Doctor includes public API version information when online.

Applications still own databases, cache implementations, dynamic prefixes,
workflow state and durable write receipts. The platform still does not expose
enough category ACL evidence or permission precedence to infer arbitrary actors'
effective access; authorization policies must retain unknown checks.
