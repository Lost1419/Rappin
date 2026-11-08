# Yapin API support: Rappin 0.1.28

Generated: 2026-11-08T01:40:28.372777+00:00. Source review: 2026-11-08T01:36:41.480118+00:00.

API 1.3.0; exact published crate checksum `88639d2bd8d857039f02906bf28f1f56baa3a880243dfee04bd849055a7c5d83`.

[Published release](https://crates.io/crates/rappin-rs/0.1.28) · [Official API](https://www.yapin.gg/openapi.json) · [Developer guide](https://www.yapin.gg/developers)

Human/agent source review; live authenticated behavior was not tested.

Feature-level review. The operation inventory is deliberately unknown; a reviewed field or feature does not certify an entire endpoint.

Inventory: 76 documented operations. This report reviews 9 selected features; other coverage is unknown.

| Feature | Typed SDK | Low-level requests | Validated bindings | Worker | CLI/MCP |
|---|---|---|---|---|---|
| Sticker-only server messages | missing | supported | missing | partial | partial |
| use_activities permission | supported | supported | partial | partial | partial |
| Select menus | supported | supported | supported | supported | partial |
| Modal forms and submissions | supported | supported | missing | supported | partial |
| Custom emojis used in one message | supported | supported | supported | supported | supported |
| Emoji creation: documented 2 MB image limit | partial | partial | partial | partial | partial |
| Gateway event intake | supported | not_applicable | not_applicable | supported | missing |
| Gateway JSON API requests | supported | supported | supported | supported | missing |
| Hosted call activity runtime | not_applicable | not_applicable | not_applicable | not_applicable | not_applicable |

## Sticker-only server messages

Availability: Flag stickers; the sticker must belong to the server. It occupies the whole message: content, embeds, components and files stay empty.

- **typed — missing:** CreateMessage and UiMessageBody have no sticker_id field or typed sticker constructor. Cargo: no optional feature. Sources: [src/models/message.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/models/message.rs), [src/models/ui.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/models/ui.rs).
- **low_level — supported:** Client::request/Api::request can submit raw sticker_id JSON; callers supply exclusivity, UUID, scope and rollout checks. This is request transport support only. Cargo: no optional feature. Sources: [src/client.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client.rs), [src/api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/api.rs).
- **validated_bindings — missing:** send_message parses a typed UiMessageBody and validates against the bundled schema, which lacks sticker_id; there is no safe sticker binding. Cargo: no optional feature. Sources: [src/capabilities/validation.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/validation.rs), [src/capabilities/schema.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/schema.rs), [docs/openapi.json](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/docs/openapi.json).
- **worker — partial:** WorkerContext::api can send raw JSON through the parent. Typed message helpers and validated bindings do not implement sticker semantics. Workers require hot-reload. Cargo: `hot-reload`. Sources: [src/hot_reload/worker.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/hot_reload/worker.rs), [src/models/message.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/models/message.rs).
- **cli_mcp — partial:** CLI request can submit raw JSON. Named send_message CLI/MCP tools lack a sticker workflow and its validation; MCP has no arbitrary request tool. CLI/MCP compilation requires cli. Cargo: `cli`. Sources: [src/bin/rappin.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin.rs), [src/bin/rappin_mcp.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin_mcp.rs), [src/capabilities/validation.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/validation.rs).

Platform references: [source 1](https://www.yapin.gg/openapi.json), [source 2](https://www.yapin.gg/developers).

## use_activities permission

Availability: Documented role/everyone permission and channel/category override key. Activities remain in testing; no arbitrary-actor effective permission precedence is established.

- **typed — supported:** Permissions uses string keys, Permission::new accepts names, and PermissionOverride uses string lists. No new enum variant is needed for typed requests. Cargo: no optional feature. Sources: [src/models/server_features.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/models/server_features.rs), [src/commands.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/commands.rs), [src/client/server_features.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client/server_features.rs).
- **low_level — supported:** Raw JSON preserves the key. Platform authorization still determines access. Cargo: no optional feature. Sources: [src/client.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client.rs), [src/api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/api.rs).
- **validated_bindings — partial:** Role permission dictionaries are extensible, but bundled Override enums omit use_activities; validated channel/category override operations reject this documented key. Cargo: no optional feature. Sources: [src/capabilities/schema.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/schema.rs), [src/capabilities/validation.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/validation.rs), [docs/openapi.json](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/docs/openapi.json).
- **worker — partial:** Shared ApiExt and raw proxy calls can carry the key; validated override bindings retain the old schema restriction. Workers require hot-reload. Cargo: `hot-reload`. Sources: [src/api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/api.rs), [src/hot_reload/worker.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/hot_reload/worker.rs), [src/capabilities/schema.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/schema.rs).
- **cli_mcp — partial:** Raw/direct CLI requests can carry the key; named validated override tools inherit the bundled enum gap. CLI/MCP requires cli. Cargo: `cli`. Sources: [src/bin/rappin.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin.rs), [src/bin/server_features.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/server_features.rs), [src/capabilities/schema.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/schema.rs).

Platform references: [source 1](https://www.yapin.gg/openapi.json), [source 2](https://www.yapin.gg/developers).

## Select menus

Availability: Flag bot_selects; endpoint docs describe test-server rollout. Up to 25 options; a select occupies its action row.

- **typed — supported:** SelectMenu, SelectOption and UiMessageBody builders validate bounds; interactive_events and UI handlers are opt-in for receiving values. Cargo: no optional feature. Sources: [src/models/ui.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/models/ui.rs), [src/ui_api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/ui_api.rs), [src/bot/mod.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bot/mod.rs), [src/validation.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/validation.rs).
- **low_level — supported:** Raw JSON sends and callback requests are available; callers own validation/response state when bypassing helpers. Cargo: no optional feature. Sources: [src/client.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client.rs), [src/api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/api.rs).
- **validated_bindings — supported:** send_message accepts UiMessageBody with select validation and normalization; authorization/availability requirements still apply. This row covers sending selects, not a named callback binding. Cargo: no optional feature. Sources: [src/capabilities/validation.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/validation.rs), [src/capabilities/schema.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/schema.rs).
- **worker — supported:** on_ui_interaction or ui_api opts into parent protocol v7 for UI values/callbacks; parent interactive_events must be enabled. Requires hot-reload. Cargo: `hot-reload`. Sources: [src/hot_reload/worker.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/hot_reload/worker.rs), [src/ui_api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/ui_api.rs).
- **cli_mcp — partial:** Named message tools preserve select payloads. Polling exposes raw events, but CLI/MCP is not the native typed interaction-dispatch runtime. Requires cli. Cargo: `cli`. Sources: [src/capabilities/validation.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/validation.rs), [src/bin/agent_events.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/agent_events.rs), [src/bin/rappin_mcp.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin_mcp.rs).

Platform references: [source 1](https://www.yapin.gg/openapi.json), [source 2](https://www.yapin.gg/developers).

## Modal forms and submissions

Availability: Flag bot_modals; test-server rollout. Open as an initial eligible interaction response, not from modal_submit; up to five inputs.

- **typed — supported:** Modal/TextInput builders and show_modal coordinate validation, initial response ownership and opt-in UI dispatch. Cargo: no optional feature. Sources: [src/models/ui.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/models/ui.rs), [src/ui_api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/ui_api.rs), [src/validation.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/validation.rs).
- **low_level — supported:** Raw interaction callbacks are possible; direct callers must preserve deadline and response ownership. Cargo: no optional feature. Sources: [src/api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/api.rs), [src/client.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client.rs).
- **validated_bindings — missing:** The capability catalogue has no named modal-opening/submission binding; typed UiApiExt is a separate API. Cargo: no optional feature. Sources: [src/capabilities/catalog.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/catalog.rs), [src/ui_api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/ui_api.rs).
- **worker — supported:** Opt-in protocol v7 UI forwarding supports modal callbacks and values. Requires hot-reload and a compatible parent; interactive_events is enabled in the parent. Cargo: `hot-reload`. Sources: [src/hot_reload/worker.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/hot_reload/worker.rs), [src/ui_api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/ui_api.rs).
- **cli_mcp — partial:** CLI request can issue callbacks and event tools expose raw submissions. There is no named MCP modal workflow or native typed CLI dispatcher. Requires cli. Cargo: `cli`. Sources: [src/bin/rappin.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin.rs), [src/bin/rappin_mcp.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin_mcp.rs), [src/capabilities/catalog.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/catalog.rs).

Platform references: [source 1](https://www.yapin.gg/openapi.json), [source 2](https://www.yapin.gg/developers).

## Custom emojis used in one message

Availability: Flag bot_emoji_lookup; test-server restriction in endpoint/catalogue. Validate server/message scope and requester/bot access independently.

- **typed — supported:** Client and UiApiExt expose get_message_emojis and retain typed emoji metadata. Cargo: no optional feature. Sources: [src/ui_api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/ui_api.rs), [src/models/ui.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/models/ui.rs).
- **low_level — supported:** GET through Client/Api is available. A successful bot read does not prove requester authority. Cargo: no optional feature. Sources: [src/api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/api.rs), [src/client.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client.rs).
- **validated_bindings — supported:** get_message_emojis has schema, scoped authorization and rollout requirements. Unestablished checks block execution. Cargo: no optional feature. Sources: [src/capabilities/catalog.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/catalog.rs), [src/capabilities/platform.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/platform.rs), [src/capabilities/authorization.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/authorization.rs), [src/capabilities/mod.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/mod.rs).
- **worker — supported:** Shared UiApiExt/proxy reads preserve typed results. Requires hot-reload; no new worker event protocol is required for this ordinary read. Cargo: `hot-reload`. Sources: [src/ui_api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/ui_api.rs), [src/api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/api.rs), [src/hot_reload/worker.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/hot_reload/worker.rs).
- **cli_mcp — supported:** platform get_message_emojis and the matching MCP catalogue tool expose the operation. Requires cli. Cargo: `cli`. Sources: [src/capabilities/catalog.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/catalog.rs), [src/bin/rappin.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin.rs), [src/bin/rappin_mcp.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin_mcp.rs).

Platform references: [source 1](https://www.yapin.gg/openapi.json), [source 2](https://www.yapin.gg/developers).

## Emoji creation: documented 2 MB image limit

Availability: Endpoint description says PNG/GIF/WebP up to 2 MB, Manage Emoji, flag bot_emojis/test servers; tag text drops testing, so use the stricter endpoint restriction until clarified. Platform MB units are not defined here.

- **typed — partial:** CreateEmoji and multipart request_upload still cap decoded images at 1,048,576 bytes. Smaller images are supported, the full newly documented range is unavailable. Cargo: no optional feature. Sources: [src/models/platform.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/models/platform.rs), [src/client/platform.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client/platform.rs).
- **low_level — partial:** Raw JSON/base64 Client::request can avoid the typed cap; request_upload enforces 1 MiB. No authenticated test establishes acceptance above that limit. Cargo: no optional feature. Sources: [src/client.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client.rs), [src/client/platform.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client/platform.rs).
- **validated_bindings — partial:** create_emoji validation uses CreateEmoji and retains the 1 MiB bound. Cargo: no optional feature. Sources: [src/capabilities/validation.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/validation.rs), [src/models/platform.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/models/platform.rs).
- **worker — partial:** Shared typed/parent upload paths retain the 1 MiB limit; raw JSON remains possible. Requires hot-reload. Cargo: `hot-reload`. Sources: [src/api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/api.rs), [src/client/platform.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client/platform.rs), [src/hot_reload/worker.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/hot_reload/worker.rs).
- **cli_mcp — partial:** emoji-upload and validated create_emoji tools retain the 1 MiB cap. CLI request is a raw JSON escape hatch. Requires cli. Cargo: `cli`. Sources: [src/bin/rappin.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin.rs), [src/capabilities/validation.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/validation.rs).

Platform references: [source 1](https://www.yapin.gg/openapi.json), [source 2](https://www.yapin.gg/developers).

## Gateway event intake

Availability: Gateway is documented as beta; availability/denial is handled through preferred polling fallback or explicit gateway-only behavior.

- **typed — supported:** BotBuilder::gateway opts into WebSocket events, heartbeat/resume and polling fallback; gateway_only selects stricter behavior. Requires gateway. Cargo: `gateway`. Sources: [src/bot/mod.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bot/mod.rs), [src/gateway.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/gateway.rs).
- **low_level — not_applicable:** WebSocket event intake is a transport runner, not an arbitrary HTTP request. Cargo: no optional feature.
- **validated_bindings — not_applicable:** Command capability bindings execute API operations; they do not select event intake. Cargo: no optional feature.
- **worker — supported:** The parent owns gateway intake and dispatches admitted events to workers. Requires gateway in the parent and hot-reload for workers. Cargo: `gateway`, `hot-reload`. Sources: [src/bot/pipeline.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bot/pipeline.rs), [src/hot_reload/host.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/hot_reload/host.rs).
- **cli_mcp — missing:** The CLI/MCP event tools use polling; compiling gateway does not make the CLI open a gateway connection. Cargo: `cli`. Sources: [src/bin/agent_events.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/agent_events.rs), [src/bin/rappin.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin.rs).

Platform references: [source 1](https://www.yapin.gg/openapi.json), [source 2](https://www.yapin.gg/developers).

## Gateway JSON API requests

Availability: Opt-in; each connection must advertise ready.requests=true. Eligible JSON calls only; uploads, binary calls and /events use HTTPS.

- **typed — supported:** configure_gateway_requests opts into the parent-owned bounded response pump; unknown submitted mutations never fall back or replay. Requires gateway. Cargo: `gateway`. Sources: [src/gateway/requests.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/gateway/requests.rs), [src/client.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/client.rs).
- **low_level — supported:** Eligible Client/Api JSON requests use the configured connection. No connection/capability chooses HTTPS before submission. Requires gateway. Cargo: `gateway`. Sources: [src/gateway/requests.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/gateway/requests.rs), [src/api.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/api.rs).
- **validated_bindings — supported:** Bindings use the same Client transport without weakening validation or permission checks. Requires gateway for the configured parent transport. Cargo: `gateway`. Sources: [src/capabilities/execution.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/capabilities/execution.rs), [src/gateway/requests.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/gateway/requests.rs).
- **worker — supported:** Workers use the parent-selected transport. Requires gateway in the parent and hot-reload; api_once additionally requires protocol v6 opt-in. Cargo: `gateway`, `hot-reload`. Sources: [src/hot_reload/worker.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/hot_reload/worker.rs), [src/gateway/requests.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/gateway/requests.rs).
- **cli_mcp — missing:** CLI/MCP does not own a gateway connection or expose gateway-request configuration. Cargo: `cli`. Sources: [src/bin/rappin.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin.rs), [src/bin/rappin_mcp.rs](https://github.com/Lost1419/Rappin/blob/862cfd1a37e1004790388c5be3e0a54783c1640a/src/bin/rappin_mcp.rs).

Platform references: [source 1](https://www.yapin.gg/openapi.json), [source 2](https://www.yapin.gg/developers).

## Hosted call activity runtime

Availability: custom_activities remains in testing. Registered HTTPS pages use the browser JavaScript bridge; discovery/verification differs for verified bots, unverified bots and DM calls.

- **typed — not_applicable:** The documented activity runtime is a hosted browser page. It is not a Rust bot REST endpoint. Cargo: no optional feature.
- **low_level — not_applicable:** No bot REST activity lifecycle endpoint is established in this OpenAPI snapshot. Cargo: no optional feature.
- **validated_bindings — not_applicable:** There is no documented bot REST runtime operation to bind; use_activities permissions are reviewed separately. Cargo: no optional feature.
- **worker — not_applicable:** Worker cogs do not replace the hosted browser runtime. Cargo: no optional feature.
- **cli_mcp — not_applicable:** CLI/MCP is not the browser postMessage activity bridge. Cargo: no optional feature.

Platform references: [source 1](https://www.yapin.gg/openapi.json), [source 2](https://www.yapin.gg/developers).

## Evidence and freshness

OpenAPI SHA-256: `1dd36afdb61142dbd034ee85a44b4320338ad4656f7d51fa88b3b9ef623194b5`.

Developer text SHA-256: `eabba81860eac1a1529d060ddc6b704785e47f89937a3065fa9f6381b232829a`.

Source capture times (UTC):

- [api.html](https://www.yapin.gg/developers/api): 2026-11-08T01:34:25.611919+00:00; SHA-256 `960f32512680cd4c879b1baa91ed4219c62d4463746042767275dec9599431c5`.
- [changelog.json](https://www.yapin.gg/api/bot/v1/changelog?limit=10): 2026-11-08T01:34:26.244388+00:00; SHA-256 `73ab50b62674d31328e8b3ff96ecab167146e45cbeebd603b2ab9dc5d4826d8c`.
- [developers.html](https://www.yapin.gg/developers): 2026-11-08T01:34:24.647121+00:00; SHA-256 `661122b082ae5695da3e6d1ddfd08fbe32cf41e3517dc491fdbe85fc7a9dc9d9`.
- [openapi.json](https://www.yapin.gg/openapi.json): 2026-11-08T01:34:24.153423+00:00; SHA-256 `1dd36afdb61142dbd034ee85a44b4320338ad4656f7d51fa88b3b9ef623194b5`.
- [version.json](https://www.yapin.gg/api/bot/v1/version): 2026-11-08T01:34:25.922522+00:00; SHA-256 `71cb8f8f6691e03812bdc630dcc60a08e8bfa886cfba989ebdca8cfcb79652a6`.

A matching version number alone cannot refresh this review. Changed crate bytes, OpenAPI bytes or readable guide text make saved statuses unknown until reassessed. A low-level request can still be refused by platform permissions or rollout flags.

Reproduce with the public [audit skill](https://github.com/Lost1419/Rappin/tree/main/skills/yapin-api-audit). Its release downloader does not build or execute crate code.
