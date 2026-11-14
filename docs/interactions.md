# Messages and interactions

Use `CreateMessage` to send, `EditMessage` to patch, and `Message` to read a
message. The same models work through Client and the shared `ApiExt` on native,
worker and custom API contexts. These REST helpers require no optional feature.

## Send text, embeds and components

```rust,no_run
use rappin::{Client, CreateMessage, SelectMenu, SelectOption, Result};
async fn choose(client: &Client, channel: rappin::ChannelId) -> Result<()> {
    let menu = SelectMenu::new("topic")
        .option(SelectOption::new("Rust", "rust"))
        .option(SelectOption::new("Other", "other"));
    client.send_message(channel, &CreateMessage::new("Choose a topic").component(menu.row())).await?;
    Ok(())
}
```

`Component::row` accepts buttons or one select menu. A select has 1–25 unique
options. Validation checks option bounds, custom IDs, labels, emojis and nesting.
For Components V2, use `CreateMessage::layout` with layout components such as
`Component::container` and `TextDisplay`. Add a reply target with `.reply_to(id)`.
Send a standalone sticker with `CreateMessage::sticker(id)`.

`EmojiRef` accepts text or a custom UUID/name/animated object. Select options and
`Component::button(button, emoji)` preserve structured references. The button's
`with_emoji_ref` helper converts a reference to Yapin's text notation. Yapin's
server-membership and emoji-use permissions apply independently of lookup visibility.

`Message.components` retains selects and unknown platform fields using
`Component::Raw`. Additional message fields are available in `Message.extra`.
`get_message` returns `MessageDetailResponse` with `message` and an optional author.
`get_messages` returns an ordered `MessagesResponse` page.

## Edit and explicitly clear fields

Omission retains a field. The clear helpers serialize explicit JSON nulls.

```rust,no_run
use rappin::{Api, ApiExt, EditMessage, ChannelId, MessageId, Result};
async fn update(api: &impl Api, channel: ChannelId, message: MessageId) -> Result<()> {
    api.edit_message(channel, message, &EditMessage::new()
        .content("Updated").clear_components()).await?;
    Ok(())
}
```

Use `clear_content`, `clear_embeds` and `clear_components` for clearing.
`EditMessage` rejects create-only fields such as reply targets and stickers.
The transport removes response-only image proxy metadata before submitting typed
message payloads. Outbound component validation also applies to lossless trees.

## Receive buttons, selects and forms

Buttons, selects and modal submissions use `InteractionContext`. It carries the
original `event`, optional typed `values`, and slash `command`/`args` metadata.
Match custom IDs and validate submitted data before storing effects.

```rust,no_run
use rappin::{Bot, Modal, TextInput, InteractionValues, Result};
async fn run(token: String) -> Result<()> {
    Bot::builder(token)
        .on_select(|ctx| async move {
            if ctx.custom_id() == "topic" {
                if let Some(InteractionValues::Selected(values)) = &ctx.values {
                    println!("Selected {values:?}");
                }
                ctx.show_modal(&Modal::new("feedback", "Feedback")
                    .input(TextInput::paragraph("text", "Your feedback"))).await?;
            }
            Ok(())
        })
        .on_modal(|ctx| async move {
            if ctx.custom_id() == "feedback" { ctx.ephemeral("Received").await?; }
            Ok(())
        })
        .build()?.run().await
}
```

A modal contains up to five text or select fields. `InteractionValues::Selected`
contains selected strings. `Submitted` maps modal field IDs to `ModalValue::Text`
or selected lists. For a custom cog, implement fallible `Cog::on_interaction`
and enable `interactive_events(true)` on the parent.

The `on_select`, `on_modal` and `on_command_interaction` builders enable typed
routing. These event kinds use their registered handlers and the unified cog
listener. Button custom-ID prefixes handle button events. All handler failures
participate in the explicit completion/checkpoint policy.

## Response ownership

- `reply` and `reply_with` send public responses; `ephemeral` sends a private response.
- `defer` acknowledges a slow operation. An acknowledged interaction can receive an eligible follow-up.
- `update` changes message text. `update_with(&EditMessage)` supports full patches and null clearing.
- `show_modal` requires the initial response slot. An earlier acknowledgement, response or uncertain callback blocks opening a modal.
- A modal submission cannot open another modal. Command-origin interactions without a source message require an eligible reply or acknowledgement.

Context clones share response tracking. `response_state` reports the phase and
confirmed response count. Failed ambiguous submissions become `Uncertain` and
block automatic resubmission. Callback expiry is enforced. API reads, retries
and handler completion retain their separate meanings. A successful error reply
alone cannot resolve earlier uncertain application effects.

Custom API adapters implement their own response coordination through
`Api::track_interaction`. `ApiExt::respond` and `update_interaction` use that
coordinator and retain the original callback identity. The [failure guide](handler-failures.md)
explains failed replies, structured causes and cursor barriers.

## Slash interactions

`ApiExt::set_interaction_commands(commands, true)` requests platform slash
interactions. `BotBuilder::interaction_commands(true)` uses that mode during
startup and transactional cog publication. Registered commands use `CommandContext` automatically. For unregistered command
interaction handling, register `on_command_interaction`
or a cog listener and parse `ctx.args` with `ctx.parse::<A>()`.

These events have their own interaction identity. A server-message gate covers
server-message dispatch. Apply requester/bot authorization inside interaction
handlers when selecting actions. Platform rollout and permissions determine availability.

## Workers

With `hot-reload`, use `WorkerCog::on_interaction` and enable typed events on the
parent. `WorkerContext::ui_interaction()` exposes typed wire data. The same
`ApiExt` methods proxy operations through the parent.

```rust,no_run
# #[cfg(feature = "hot-reload")]
# async fn worker() -> rappin::Result<()> {
use rappin::{WorkerCog, ApiExt, Modal, TextInput};
WorkerCog::new("forms").on_interaction(|ctx| async move {
    if let Some(event) = ctx.ui_interaction() {
        if event.interaction.kind.as_deref() == Some("select") {
            ctx.show_modal(&event.interaction, &Modal::new("form", "Form")
                .input(TextInput::short("name", "Name"))).await?;
        }
    }
    Ok(())
}).run().await
# }
```

Legacy `Listener::Interaction` retains button-only routing. Use
`WorkerCog::on_interaction` for the unified typed interaction listener.

Protocol 12 supplies UI callbacks, single-attempt requests and operation identities
on every worker route. Rebuild parent and workers together when migrating. Workers
and build scripts execute trusted code. Keep stdout for IPC.

## Discover message emojis

`ApiExt::get_message_emojis` and the Client helper return `FoundEmoji` values
with ID, name, image URL, server-membership metadata and ready-to-paste text.
Deleted emojis are omitted. The endpoint requires Read Message History and is
subject to the documented rollout and rate limit.

CLI/MCP expose `get_message_emojis`. Validated library bindings check resource
scope and assess requester/bot history access separately. Inspect the installed
catalogue with `rappin capabilities`; platform calls accept explicit IDs.

The complete native form example is [interactive_forms.rs](../examples/interactive_forms.rs).
