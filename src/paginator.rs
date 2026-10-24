use uuid::Uuid;

use crate::models::component::{ActionRow, Button};
use crate::models::embed::Embed;
use crate::models::interaction::InteractionCallback;
use crate::models::message::CreateMessage;

/// An interactive multi-page embed navigator with next/previous buttons.
#[derive(Debug, Clone)]
pub struct Paginator {
    embeds: Vec<Embed>,
    id: String,
}

impl Paginator {
    /// Creates a new Paginator with a list of embeds to display across pages.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::models::embed::Embed;
    /// use rappin::paginator::Paginator;
    ///
    /// let pages = vec![
    ///     Embed::new().title("Page 1"),
    ///     Embed::new().title("Page 2"),
    /// ];
    /// let paginator = Paginator::new(pages);
    /// assert_eq!(paginator.total_pages(), 2);
    /// ```
    pub fn new(embeds: impl IntoIterator<Item = Embed>) -> Self {
        let unique = Uuid::new_v4().simple().to_string();
        Self {
            embeds: embeds.into_iter().collect(),
            id: format!("pg_{}", &unique[..8]),
        }
    }

    /// Sets a custom prefix for pagination button interaction IDs.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Returns the pagination identifier prefix.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Total number of pages.
    pub fn total_pages(&self) -> usize {
        self.embeds.len()
    }

    /// Builds the `ActionRow` navigation buttons for a specific page.
    pub fn build_buttons(&self, current_page: usize) -> ActionRow {
        let total = self.total_pages();
        let is_first = current_page == 0;
        let is_last = current_page + 1 >= total;

        let prev_page = if is_first { 0 } else { current_page - 1 };
        let next_page = if is_last {
            current_page
        } else {
            current_page + 1
        };

        let prev_btn =
            Button::secondary("◀ Prev", format!("{}:{}", self.id, prev_page)).disabled(is_first);

        let indicator_btn = Button::secondary(
            format!("{}/{}", current_page + 1, total),
            format!("{}:noop", self.id),
        )
        .disabled(true);

        let next_btn =
            Button::secondary("Next ▶", format!("{}:{}", self.id, next_page)).disabled(is_last);

        ActionRow::new(vec![prev_btn, indicator_btn, next_btn])
    }

    /// Builds the initial `CreateMessage` for page 0 (or a given page).
    pub fn build_message(&self, page: usize) -> CreateMessage {
        if self.embeds.is_empty() {
            return CreateMessage::new("No content to display.");
        }

        let clamped = page.min(self.total_pages().saturating_sub(1));
        let embed = self.embeds[clamped].clone();
        let buttons = self.build_buttons(clamped);

        CreateMessage::new("").embed(embed).action_row(buttons)
    }

    /// Builds an `InteractionCallback` to update the message when a user navigates pages.
    pub fn build_update(&self, page: usize) -> InteractionCallback {
        if self.embeds.is_empty() {
            return InteractionCallback::update("No content to display.");
        }

        let clamped = page.min(self.total_pages().saturating_sub(1));
        let embed = self.embeds[clamped].clone();
        let buttons = self.build_buttons(clamped);

        InteractionCallback::update("")
            .embed(embed)
            .component(buttons)
    }

    /// Parses the target page number from a button's `custom_id` (e.g. `pg_abc123:2`).
    pub fn parse_page_from_custom_id(&self, custom_id: &str) -> Option<usize> {
        let expected_prefix = format!("{}:", self.id);
        if let Some(rest) = custom_id.strip_prefix(&expected_prefix) {
            rest.parse::<usize>().ok()
        } else {
            None
        }
    }
}

/// Helper for creating a prompt with Confirm and Cancel buttons.
#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    id: String,
    content: String,
    confirm_label: String,
    cancel_label: String,
}

impl ConfirmDialog {
    /// Creates a new interactive confirmation prompt.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::paginator::ConfirmDialog;
    ///
    /// let dialog = ConfirmDialog::new("Are you sure?")
    ///     .with_labels("Yes", "No");
    /// ```
    pub fn new(content: impl Into<String>) -> Self {
        let unique = Uuid::new_v4().simple().to_string();
        Self {
            id: format!("cnf_{}", &unique[..8]),
            content: content.into(),
            confirm_label: "Confirm".to_string(),
            cancel_label: "Cancel".to_string(),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_labels(mut self, confirm: impl Into<String>, cancel: impl Into<String>) -> Self {
        self.confirm_label = confirm.into();
        self.cancel_label = cancel.into();
        self
    }

    pub fn confirm_custom_id(&self) -> String {
        format!("{}:yes", self.id)
    }

    pub fn cancel_custom_id(&self) -> String {
        format!("{}:no", self.id)
    }

    pub fn build_message(&self) -> CreateMessage {
        let row = ActionRow::new(vec![
            crate::models::component::Button::success(
                &self.confirm_label,
                self.confirm_custom_id(),
            ),
            crate::models::component::Button::danger(&self.cancel_label, self.cancel_custom_id()),
        ]);

        CreateMessage::new(&self.content).action_row(row)
    }
}
