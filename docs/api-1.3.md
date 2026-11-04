# API 1.3: selects, modals and message emojis

Rappin 0.1.21 adds the public API 1.3 contracts checked on 4 November 2026.
The bundled OpenAPI snapshot is 1.3.0. Platform flags/permissions still determine
availability; a successful local validation or preview does not grant access.

## Select messages and structured emojis

Enable no optional Cargo feature for these models or REST helpers:

```rust,no_run
use rappin::{Client, UiApiExt, UiMessageBody, SelectMenu, SelectOption, Result};
use uuid::Uuid;
async fn send(client:&Client, channel:Uuid)->Result<()> {
    let menu=SelectMenu::new("topic")
        .option(SelectOption::new("Rust","rust"))
        .option(SelectOption::new("Other","other"));
    client.send_ui_message(channel,
        &UiMessageBody::new("Choose a topic").component(menu.row())).await?;
    Ok(())
}
```

A menu is alone in its row, has 1–25 uniquely valued options, and uses
`min_values` / `max_values` within the documented bounds. Local validation also
checks duplicate custom IDs across buttons/menus, labels, emoji references and
V2 nesting. `UiComponent::container` and `UiMessageBody::layout` support V2 trees.
`UiComponent::from(Component)` and `UiComponent::from(Button)` accept existing
components. Numeric/string send aliases and server-assigned component IDs are
accepted by the shared request schemas without changing the upstream snapshot.

`EmojiRef` accepts text or a custom UUID/name/animated object. Select options and
`UiComponent::button(button, emoji)` retain the structured form. Existing
`Button::with_emoji_ref` converts a reference to the supported text notation,
preserving the existing `Button.emoji: Option<String>` field. Yapin enforces
server membership for emoji use: lookup visibility is not permission to use an
external emoji. Text/embeds can fall back to `:name:`; button/menu/reaction uses
may be refused by the service.

`UiApiExt` works with Client, native contexts, WorkerContext and custom Api
adapters. It provides `send_ui_message`, `edit_ui_message`, `get_ui_message` and
`get_ui_messages`. `UiMessageBody` preserves omitted versus explicit null patch
fields. `UiMessage` / `UiMessagesResponse` preserve new component trees and extra
message fields. Legacy `Component`, `ActionRow`, `Button`, `CreateMessage`,
`InteractionCallback` and their exhaustive enums keep their public shapes;
use the new UI payload/response types when selects or structured emoji data are
needed. Existing typed legacy reads retain their original component contract.
CLI/MCP history and rich send/edit paths use lossless UI data automatically.

## Forms and typed event values

`Modal::new(id, title)` takes up to five `TextInput` / `ModalSelect` fields.
Use `TextInput::short` or `paragraph`; select fields need a label. Validation
checks field IDs, title/label limits, input bounds and documented row/label
wrappers. `InteractionContext::show_modal` and `UiApiExt::show_modal` open a form.
A modal must be the first response; acknowledgement, an earlier response or an
uncertain result blocks opening it. A modal submission cannot open another modal.
Automatic defer can consume that first response; avoid it on routes that must
open a form. Callback expiry and response ownership remain coordinated in Client.
Custom Api adapters own their response coordination through `track_interaction`.

```rust,no_run
use rappin::{Bot, Modal, TextInput, Result};
async fn run(token:String)->Result<()> {
    Bot::builder(token)
        .on_select(|ctx|async move {
            if ctx.custom_id()=="topic" {
                ctx.show_modal(&Modal::new("feedback","Feedback")
                    .input(TextInput::paragraph("text","Your feedback"))).await?;
            }
            Ok(())
        })
        .on_modal(|ctx|async move {
            if ctx.custom_id()=="feedback" {ctx.ephemeral("Received").await?;}
            Ok(())
        })
        .build().run().await
}
```

`UiInteraction` retains the redacted-debug base `InteractionEvent` plus
`InteractionValues::Selected(Vec<String>)` or
`Submitted(BTreeMap<String, ModalValue>)`. Modal values are text or selected
lists. `UiInteraction::from_event` and typed collectors preserve these values.
Native handlers use `UiInteractionContext.data`; match custom IDs and validate
submitted data before storing effects. `on_select`, `on_modal` and
`on_command_interaction` opt into extended routing. For custom native cogs,
implement fallible `Cog::on_ui_interaction` and enable
`BotBuilder::interactive_events(true)`.

Extended events use these new handlers/cog hooks instead of button-prefix
routing. Existing routing is unchanged until opted in. Failures use the existing
`on_button_failure` contract, with `ui:select`, `ui:modal_submit` or `ui:command`
for a global handler, or the cog name for a cog hook. Expected rejection replies,
failed replies, structured causes and explicit completion keep their established
meaning. Duplicate IDs and ordered cursor barriers work through polling and
WebSocket intake. No handler or uncertain callback is automatically replayed.
An ambiguous callback response now retains Uncertain instead of reopening the
initial-response slot.

## Optional slash interaction mode

`UiApiExt::set_interaction_commands(commands, true)` adds `interactions: true` to
PUT `/commands`. `BotBuilder::interaction_commands(true)` uses the same mode for
startup synchronization and transactional cog command publication. Existing
`set_commands` and bot defaults keep the field omitted.

Register `on_command_interaction` or a cog UI hook to answer these events.
`data.command` / `data.args` retain the selection and arguments; native UI contexts
and workers support typed `parse::<A: FromArgs>()`. Slash interactions do not post
a chat message and are separate from ordinary `MessageContext` command handlers.
The server-message gate does not authorize an interaction callback: evaluate the
application's requester/bot policy inside the interaction handler. Commands and
command-origin modal submissions have no source message to update; use reply,
ephemeral, acknowledgement or an initial modal as appropriate.

## Workers

`WorkerCog::on_ui_interaction` opts into protocol v7; upgrade the parent and enable
`interactive_events(true)` before loading it. `WorkerContext::ui_interaction()`
exposes typed data, and `UiApiExt` sends requests through the parent. For example:

```rust,no_run
# #[cfg(feature="hot-reload")]
# async fn worker()->rappin::Result<()> {
use rappin::{WorkerCog, UiApiExt, Modal, TextInput};
WorkerCog::new("forms").on_ui_interaction(|ctx|async move {
    if let Some(event)=ctx.ui_interaction() {
        if event.interaction.kind.as_deref()==Some("select") {
            ctx.show_modal(&event.interaction,
                &Modal::new("form","Form").input(TextInput::short("name","Name"))).await?;
        }
    }
    Ok(())
}).run().await
# }
```

Existing handlers still use v4, fallible message hooks v5, and single-attempt
requests v6 unless extended UI events are selected. Original structured errors,
retry hints and unknown outcomes cross the boundary and failure callbacks.
Worker builds/macros remain trusted executable code, not a sandbox.

## Message emoji discovery

`Client::get_message_emojis(channel, message)` or the same `UiApiExt` method reads
GET `/channels/{id}/messages/{mid}/emojis`. `FoundEmoji` retains ID, name, animated,
URL, `in_server` and ready-to-paste `text`, including visible emojis from other
servers. Deleted emojis are omitted. The published endpoint requires Read Message
History, has a 60-lookups/minute limit, and is initially test-server-only behind
`bot_emoji_lookup`. Copy workflows can download the picture under application-owned
network policy and pass bytes to `upload_emoji`, with Manage Emoji in the destination.

The shared catalogue/CLI/MCP operation is `get_message_emojis`, with `channel` and
`message` UUIDs. Library bindings verify channel/message server scope and separate
requester/bot history access plus trusted rollout evidence. The catalogue now has
57 operations. CLI invocation:

```sh
rappin platform get_message_emojis --arguments-file emoji-lookup.json
```

Use `rappin platform --help` / `tools list` for the installed CLI's argument form.
The complete native select/form example is `examples/interactive_forms.rs`.
