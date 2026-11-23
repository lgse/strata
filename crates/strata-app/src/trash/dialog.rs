// SPDX-License-Identifier: MIT

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
    /// 10xer's confirmed Move to Trash (`DeleteDialog::Trash`).
    Trash,
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
            sizing: kind != Kind::Trash,
            cursor: None,
            generation,
        }
    }

    pub(super) fn needs_size_task(&self) -> bool {
        self.kind != Kind::Trash
            || self
                .entries
                .iter()
                .take(MAX_ROWS)
                .any(|entry| entry.kind == strata_core::model::EntryKind::FileSymbolicLink)
    }

    /// `title_and_confirm_label`.
    fn title_and_confirm(&self) -> (String, String) {
        let items = item_count_label(self.entries.len());
        if self.kind == Kind::Trash {
            return (
                rust_i18n::t!("Move %{value1} to Trash?", value1 = items).into_owned(),
                tr("Move to Trash"),
            );
        }
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
            Kind::Trash => vec![tr(if single {
                "This item can be restored from Trash."
            } else {
                "These items can be restored from Trash."
            })],
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

pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    super::empty::build(app, frame);
    let trash = &app.trash;
    let confirm = trash.confirm.as_ref()?;
    let (title, confirm_label) = confirm.title_and_confirm();
    let focus = match confirm.kind {
        Kind::TrashUnavailable => Focus::Secondary,
        Kind::Permanent | Kind::Trash => Focus::Primary,
    };
    let tone = match confirm.kind {
        Kind::Trash => Tone::Accent,
        Kind::Permanent | Kind::TrashUnavailable => Tone::Danger,
    };
    let loading = confirm.sizing.then(|| tr("Calculating total size…"));
    let key = format!("delete-confirm-{}", confirm.generation);
    let mut dialog = StrataDialog::new(key.clone(), title, super::cancelled)
        .open(trash.open)
        .subtitle(confirm.subtitle.clone())
        .icon(IconName::Trash)
        .tone(tone)
        .loading(loading)
        .child(item_list(app, confirm).key(format!("{key}-items")));
    for paragraph in confirm.paragraphs() {
        dialog = dialog.message(paragraph);
    }
    dialog = dialog
        .secondary(tr("Cancel"), super::cancelled)
        .primary(confirm_label, super::confirmed)
        .initial_focus(focus)
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
