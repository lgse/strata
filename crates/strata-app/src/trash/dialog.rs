// SPDX-License-Identifier: MIT

//! The delete confirmation, GTK's `show_delete_dialog` (`src/ui/browser/trash/delete_dialog.rs`)
//! on C15's [`StrataDialog`]: what will be deleted, how far the deletion reaches, Cancel and
//! the delete itself.
//!
//! - **The panel.** A Trash icon in a danger box; "Permanently delete 2 items?" (or "Delete …
//!   without Trash?" where the provider may keep a copy); the items' kind summary under it,
//!   with a spinner and "Calculating total size…" until the total is known, which then takes
//!   its place. The body lists up to 50 items, each with its size, "Folder" or "Folder link",
//!   and a line for the rest; then why: Trash unsupported here, the mount's reason, and what
//!   cannot be undone.
//! - **The keyboard.** The delete has it when the dialog opens; Cancel has it where Trash was
//!   asked for and is unsupported, since the user did not ask to delete. Escape, a click
//!   outside and the close button cancel. Left and Right move between the buttons (C15).
//! - **Cancel** puts the cursor back on the items asked about if something moved it while the
//!   dialog was open (`return_cursor_on_cancel`).
//! - **The size task** runs on GLib's context and answers through `Setup::later`. It stops
//!   when the dialog closes, and a dialog that has closed, or another that has opened, takes
//!   no answer.
//! - **Return** presses the focused button only (`activate_focused_button`): the delete is no
//!   default button. The keyboard returns to the listing on close
//!   (`remember_modal_focus_for_listing`).
//! - **One at a time.** A confirmation asked while one is open waits for it to close, under a
//!   key of its own, so it takes its own first focus; GTK stacks it over the first.
//!
//! Left out: the dissolve of the deleted rows (`prepare_delete_dissolve`), and 10xer's key
//! that confirms a trash (`TENXER_DELETE_CONFIRM`, A11's). A file link's size shows when the
//! whole total does, where GTK shows each as it is found.

use rune::El;
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::{
    i18n::{count, tr},
    model::{FileEntry, Location},
};
use strata_kit::icons::IconName;

use super::sizes::{
    Reach, entry_kind_summary, item_count_label, metadata_text, reach_description,
    trash_unavailable_reason,
};
use crate::{App, shell, slots::Frame};

/// `DELETE_CONFIRMATION_MAX_ROWS`: more collapse into one line.
const MAX_ROWS: usize = 50;

/// Why the confirmation asks (`DeleteDialog`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    /// The user asked for a permanent delete, or a delete in the Trash.
    Permanent,
    /// The user asked for the Trash, which this location does not support.
    TrashUnavailable,
}

/// One confirmation: what it was opened for, and what the size task has said since.
pub(super) struct Confirm {
    pub(super) entries: Vec<FileEntry>,
    pub(super) kind: Kind,
    pub(super) reach: Reach,
    /// The text beside each listed item, in their order.
    pub(super) metadata: Vec<String>,
    /// The summary under the title: the kinds, then the total once it is known.
    pub(super) subtitle: String,
    pub(super) sizing: bool,
    /// The cursor when the dialog opened, to put back if Cancel finds it moved.
    pub(super) cursor: Option<(usize, Location)>,
    /// Told apart from the confirmation before, for the size task's answer.
    pub(super) generation: u64,
}

impl Confirm {
    pub(super) fn new(entries: Vec<FileEntry>, kind: Kind, reach: Reach, generation: u64) -> Self {
        let metadata = entries.iter().take(MAX_ROWS).map(metadata_text).collect();
        Self {
            subtitle: entry_kind_summary(&entries),
            metadata,
            entries,
            kind,
            reach,
            sizing: true,
            cursor: None,
            generation,
        }
    }

    /// `title_and_confirm_label`.
    fn title_and_confirm(&self) -> (String, String) {
        let items = item_count_label(self.entries.len());
        let (title, confirm) = match self.reach {
            Reach::Local => (
                rust_i18n::t!("Permanently delete %{value1}?", value1 = items),
                rust_i18n::t!("Permanently delete %{value1}", value1 = items),
            ),
            Reach::Remote => (
                rust_i18n::t!("Delete %{value1} without Trash?", value1 = items),
                rust_i18n::t!("Delete %{value1}", value1 = items),
            ),
        };
        (title.into_owned(), confirm.into_owned())
    }

    /// `paragraphs`.
    fn paragraphs(&self) -> Vec<String> {
        let single = self.entries.len() == 1;
        match self.kind {
            Kind::TrashUnavailable => [
                Some(tr("This location doesn't support Trash.")),
                trash_unavailable_reason(&self.entries),
                Some(reach_description(self.reach, single)),
            ]
            .into_iter()
            .flatten()
            .collect(),
            Kind::Permanent => vec![reach_description(self.reach, single)],
        }
    }
}

/// One listed item: its name, described by its path, and its size or kind.
fn item_row(app: &App, entry: &FileEntry, metadata: &str) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let name = rune::text(entry.display_name.clone())
        .font(kit.ty.ui)
        .color(kit.c.text_base)
        .single_line()
        .grow(1.0)
        .min_w(0.0)
        .description(entry.location.display_path());
    let size = rune::text(metadata.to_owned())
        .font(kit.ty.small)
        .color(kit.c.text_muted);
    rune::row()
        .w_full()
        .gap(look.space.md)
        .child(name)
        .child(size)
}

/// The listed items, and a line for those past the first 50 (`delete_file_list`).
fn item_list(app: &App, confirm: &Confirm) -> El {
    let mut list = rune::col().w_full().gap(app.look.space.xxs);
    for (entry, metadata) in confirm.entries.iter().zip(&confirm.metadata) {
        list = list.child(item_row(app, entry, metadata));
    }
    let hidden = confirm.entries.len().saturating_sub(MAX_ROWS);
    if hidden > 0 {
        let kit = &app.look.kit;
        list = list.child(
            rune::text(count("more_items", hidden))
                .font(kit.ty.small)
                .color(kit.c.text_muted),
        );
    }
    list
}

/// The dialog, while one is open or closing.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let trash = &app.trash;
    let confirm = trash.confirm.as_ref()?;
    let (title, confirm_label) = confirm.title_and_confirm();
    let focus = match confirm.kind {
        Kind::TrashUnavailable => Focus::Secondary,
        Kind::Permanent => Focus::Primary,
    };
    let loading = confirm.sizing.then(|| tr("Calculating total size…"));
    // A key of its own for each, so a confirmation that opens as another closes takes its own
    // first focus.
    let key = format!("delete-confirm-{}", confirm.generation);
    let mut dialog = StrataDialog::new(key.clone(), title, super::cancelled)
        .open(trash.open)
        .subtitle(confirm.subtitle.clone())
        .icon(IconName::Trash)
        .tone(Tone::Danger)
        .loading(loading)
        .child(item_list(app, confirm).key(format!("{key}-items")));
    for paragraph in confirm.paragraphs() {
        dialog = dialog.message(paragraph);
    }
    dialog = dialog
        .secondary(tr("Cancel"), super::cancelled)
        .primary(confirm_label, super::confirmed)
        .initial_focus(focus)
        // GTK's Return presses the focused button only (`activate_focused_button`): with
        // nothing focused it deletes nothing.
        .default_button(false)
        .close_label(tr("Close dialog"))
        .width(MESSAGE_WIDTH);
    // The keyboard goes back to the listing (`remember_modal_focus_for_listing`).
    if let Some(view) = shell::view_focus(app) {
        dialog = dialog.return_focus(view);
    }
    dialog.build(&app.look, frame.layer);
    None
}
