//! The keys the editor gives its parts, so hosts and scripts can name them. The editor and its
//! field sit under the row (`row-{id}`); the message sits in the host's overlay layer.

/// The editor's box, which takes the label's place.
pub const EDITOR: &str = "rename";

/// The text field, under [`EDITOR`].
pub const FIELD: &str = "rename-field";

/// The validator's message ([`RenameEditor::message`](super::RenameEditor::message)).
pub const MESSAGE: &str = "rename-message";

/// The message's text, under [`MESSAGE`].
pub const MESSAGE_TEXT: &str = "rename-message-text";
