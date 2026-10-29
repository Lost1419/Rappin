# REST transport and application-owned caches

Available in `rappin-rs` 0.1.8.

## API origins and retries

`DEFAULT_API_BASE` is now `https://bots.yapper.website/api/bot/v1`, matching
[Yapin's OpenAPI specification](https://www.yapin.gg/openapi.json). The previous
`https://www.yapin.gg/api/bot/v1` remains the documented fallback origin.

A connection failure or HTTP 502/504 from the dedicated host activates the
website origin for ten minutes, shared across `Client` clones. GET/HEAD requests
can retry there within `max_retries`. An ambiguous write returns its original
error; it is never automatically replayed on the other host. Later requests use
the fallback. After ten minutes, requests try the primary again.

Custom `.base_url(...)` origins never fall back, including the legacy website
URL. Redirects remain disabled. To pin the dedicated host, use
`.api_fallback(false)` on either `ClientBuilder` or `BotBuilder`.
`Client::base_url()` returns the configured origin;
`Client::active_base_url()` reports the current destination.
Server-advised 429/503 retries retain their existing behavior.

```rust,no_run
use rappin::Client;
let client = Client::builder("yb_token")
    .base_url("https://your-api.example/api/bot/v1")
    .max_retries(2)
    .build();
assert_eq!(client.base_url(), client.active_base_url());
```

## Your database, your cache policy

Lookup caching remains disabled by default. The built-in `CachePolicy` provides
bounded, expiring member/role snapshots. Alternatively implement `LookupCache`
with your application's SQL, Redis, or other storage and supply it to either
builder. No database dependency or schema is imposed by Rappin.

```rust,no_run
use rappin::{Bot, Client, LookupCache};
use std::sync::Arc;

fn configure(token: &str, store: Arc<dyn LookupCache>) -> (Client, Bot) {
    let client = Client::builder(token)
        .lookup_cache_backend(store.clone())
        .build();
    let bot = Bot::builder(token)
        .lookup_cache_backend(store)
        .build();
    (client, bot)
}
```

`LookupCache` supplies member/role reads, stores, and invalidation. A custom
backend takes precedence over built-in lookup storage. It owns expiration,
capacity, database errors and persistence. Return `None` on a miss or backend
failure to fetch from Yapin; handle store/invalidation failures in your backend.
Methods borrow fetched values so the SDK need not copy them before storage.
Keep backend calls bounded: Rappin awaits stores and invalidations.

`get_member`/`get_roles` and `Freshness::Refresh` always fetch from the API and
update the selected lookup backend. `Freshness::PreferCache` explicitly permits
a cached snapshot. Member join/leave events invalidate that member; successful
server member/role/moderation writes invalidate server lookups. Changes by other
clients may remain invisible until expiry: prefer fresh authorization checks.
The existing profile/server/channel snapshot cache remains separate.
Applications may also manage their own database entirely outside this trait.

## Dynamic prefixes remain dynamic

`.prefix(...)` still adds custom static prefixes. Static prefixes are prepared
once and longest matches are tried first. `.prefixes_with(...)` is still called
for every message requiring text-prefix resolution, so it can query a database
or consult application state. Results are not retained across messages.
Mention prefixes can be combined with either form.

```rust,no_run
use rappin::prelude::*;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

let prefixes = Arc::new(RwLock::new(HashMap::<Option<uuid::Uuid>, String>::new()));
let bot = Bot::builder("yb_token")
    .prefixes_with(move |message| {
        let prefixes = prefixes.clone();
        async move {
            // Replace this lookup with an awaited database query if desired.
            let prefix = prefixes.read().await.get(&message.server_id)
                .cloned().unwrap_or_else(|| "!".into());
            Ok(vec![prefix])
        }
    })
    .mention_prefix(true)
    .build();
```

Cog installs, replacements and removals update the command-root index under the
same routing lock as the handlers; failed replacements retain the old routes.

## Metadata and interaction expiry

Typed events now preserve message channel/author/attachment metadata, reaction
user/author metadata, member-leave metadata and interaction type/expiry.
`InteractionEvent::expires_at` is authoritative when supplied; otherwise the SDK
uses `created_at + 15 minutes` where available. Reobserving an interaction never
extends an already recorded deadline. Expired callbacks are rejected locally.
Bot intake and context helpers track deadlines automatically. Custom transports
using raw callback methods should call `Client::track_interaction(&event).await`
before responding. Custom collectors still need an authenticated event source.

`WebhookClient::get_info()` inspects a channel webhook without posting. Returned
`WebhookInfo` deliberately omits credential fields, even if the server sends them.

The developer guide now documents 300-character bot biographies, while OpenAPI
still declares 190. Local validation accepts up to 300 Unicode characters and
preserves any stricter server rejection. This is a guide-aligned client limit,
not a claim that every deployment has enabled 300 characters.

## Migration

Updating changes the default REST destination. Explicit custom bases remain
unchanged. New optional fields in `MessageEvent`, `InteractionEvent`,
`ReactionEvent` and `MemberLeaveEvent` require updating manual struct literals;
existing JSON without those fields continues to decode.

Unchanged event checkpoints no longer trigger file writes. Advancing checkpoints
still use atomic replacement and file synchronization after ordered handler
completion. Handler failures still prevent committing their batch.
