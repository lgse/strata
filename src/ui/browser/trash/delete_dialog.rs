// SPDX-License-Identifier: MIT

use super::{DeletionReach, TrashModal, deletion_reach, reach_description};
use crate::app::Browser;
use crate::model::{EntryKind, FileEntry, MetadataValue};
use crate::ui::browser::ViewState;
use crate::ui::browser::entry::{
    aggregate_directory_summary, delete_entry_size, entry_icon, entry_kind_summary,
    format_file_size, item_count_label,
};
use crate::ui::controls::{
    ModalLayout, ModalTone, focus_button, message_dialog_description, message_dialog_layout,
};
use crate::ui::keys::{
    self, KeyPress,
    bindings::{ENTER_ANY, ESCAPE_ANY, LEFT, RIGHT, TENXER_DELETE_CONFIRM},
};
use crate::ui::modal::{ModalHost, remember_modal_focus_for_listing};
use gtk::gdk::ModifierType as Modifiers;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeleteDialog {
    Permanent,
    /// Permanent fallback the user did not ask for: Trash is unsupported here.
    TrashUnavailable,
    Trash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeleteConfirmationFocus {
    Cancel,
    Confirm,
}

fn delete_confirmation_focus_target(press: &KeyPress) -> Option<DeleteConfirmationFocus> {
    const ARROW_MODIFIERS: Modifiers = Modifiers::SHIFT_MASK.union(Modifiers::SUPER_MASK);
    if press.is(LEFT.ignoring(ARROW_MODIFIERS).or_alternates()) {
        Some(DeleteConfirmationFocus::Cancel)
    } else if press.is(RIGHT.ignoring(ARROW_MODIFIERS).or_alternates()) {
        Some(DeleteConfirmationFocus::Confirm)
    } else {
        None
    }
}

/// Rows rendered in the delete confirmation; anything beyond this collapses
/// into a summary so a thousand-file selection cannot stall the UI thread.
pub(super) const DELETE_CONFIRMATION_MAX_ROWS: usize = 50;

/// Splits confirmation entries into rendered rows and the hidden remainder.
fn delete_confirmation_rows(entries: &[FileEntry]) -> (&[FileEntry], usize) {
    let visible = entries.len().min(DELETE_CONFIRMATION_MAX_ROWS);
    (&entries[..visible], entries.len() - visible)
}

/// Summary for the hidden remainder, or nothing when everything is shown.
fn delete_confirmation_overflow_label(hidden: usize) -> Option<String> {
    (hidden > 0).then(|| crate::i18n::count("more_items", hidden))
}

fn delete_confirmation_file_size(size: &MetadataValue<u64>) -> String {
    match size {
        MetadataValue::Known(size) => format_file_size(*size),
        MetadataValue::Unknown | MetadataValue::Unavailable => "—".to_owned(),
    }
}

/// Names the mount a failed trash attempt recorded, when the dialog's first
/// item is on one.
fn trash_unavailable_reason(entries: &[FileEntry]) -> Option<String> {
    let first = &entries.first()?.location;
    if first.is_administrator() {
        return Some(crate::i18n::tr(
            "Trash isn't available when browsing as administrator.",
        ));
    }
    let parent = first.parent()?;
    let (mount, _) = crate::adapters::trash_support::unsupported_reason_for(parent.native_path()?)?;
    Some(
        rust_i18n::t!(
            "Trash isn't available on %{mount_point} (%{fs_type}).",
            mount_point = mount.mount_point.display(),
            fs_type = mount.fs_type
        )
        .into_owned(),
    )
}

/// What a delete dialog was opened for, shared by its builders and handlers.
struct DeleteRequest {
    entries: Rc<Vec<FileEntry>>,
    kind: DeleteDialog,
    reach: DeletionReach,
    confirmed: Cell<bool>,
}

impl DeleteRequest {
    fn trash(&self) -> bool {
        self.kind == DeleteDialog::Trash
    }

    fn title_and_confirm_label(&self) -> (String, String) {
        let items = item_count_label(self.entries.len());
        let (title, confirm_label) = match (self.trash(), self.reach) {
            (true, _) => (
                rust_i18n::t!("Move %{value1} to Trash?", value1 = items),
                crate::i18n::tr("Move to Trash").into(),
            ),
            (false, DeletionReach::Local) => (
                rust_i18n::t!("Permanently delete %{value1}?", value1 = items),
                rust_i18n::t!("Permanently delete %{value1}", value1 = items),
            ),
            (false, DeletionReach::Remote) => (
                rust_i18n::t!("Delete %{value1} without Trash?", value1 = items),
                rust_i18n::t!("Delete %{value1}", value1 = items),
            ),
        };
        (title.into_owned(), confirm_label.into_owned())
    }

    fn layout(&self) -> ModalLayout {
        let (title, confirm_label) = self.title_and_confirm_label();
        let layout = message_dialog_layout(
            crate::assets::icons::TRASH,
            &title,
            &entry_kind_summary(&self.entries),
            &confirm_label,
            if self.trash() {
                ModalTone::Accent
            } else {
                ModalTone::Danger
            },
        );
        if !self.trash() {
            layout.set_loading(true, Some(&crate::i18n::tr("Calculating total size…")));
        }
        layout
    }

    fn paragraphs(&self) -> Vec<String> {
        let single = self.entries.len() == 1;
        match self.kind {
            DeleteDialog::Trash => vec![crate::i18n::tr(if single {
                "This item can be restored from Trash."
            } else {
                "These items can be restored from Trash."
            })],
            DeleteDialog::TrashUnavailable => [
                Some(crate::i18n::tr("This location doesn't support Trash.")),
                trash_unavailable_reason(&self.entries),
                Some(reach_description(self.reach, single)),
            ]
            .into_iter()
            .flatten()
            .collect(),
            DeleteDialog::Permanent => vec![reach_description(self.reach, single)],
        }
    }

    /// A trash dialog only measures the file links it shows; the others total everything.
    fn needs_size_task(&self, visible: &[FileEntry]) -> bool {
        !self.trash()
            || visible
                .iter()
                .any(|entry| entry.kind == EntryKind::FileSymbolicLink)
    }
}

fn delete_entry_metadata_text(entry: &FileEntry) -> String {
    if entry.kind == EntryKind::DirectorySymbolicLink {
        crate::i18n::tr("Folder link")
    } else if entry.is_directory() {
        crate::i18n::tr("Folder")
    } else {
        let size = if entry.kind == EntryKind::FileSymbolicLink {
            &MetadataValue::Unknown
        } else {
            &entry.size
        };
        delete_confirmation_file_size(size)
    }
}

fn delete_entry_row(entry: &FileEntry) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("delete-confirmation-file");
    let icon = crate::assets::primary_icon(entry_icon(entry), 16);
    let name = gtk::Label::new(Some(&entry.display_name));
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name.set_hexpand(true);
    name.set_xalign(0.0);
    crate::ui::accessibility::set_description(&name, Some(&entry.location.display_path()));
    let metadata = gtk::Label::new(Some(&delete_entry_metadata_text(entry)));
    metadata.add_css_class("delete-confirmation-file-metadata");
    row.append(&icon);
    row.append(&name);
    row.append(&metadata);
    (row, metadata)
}

/// Returns the scroller and the metadata label of each rendered row, in entry order.
fn delete_file_list(
    visible: &[FileEntry],
    hidden: usize,
    count: usize,
) -> (gtk::ScrolledWindow, Vec<glib::WeakRef<gtk::Label>>) {
    let files = gtk::Box::new(gtk::Orientation::Vertical, 3);
    files.add_css_class("delete-confirmation-files");
    let mut metadata_labels = Vec::with_capacity(visible.len());
    for entry in visible {
        let (row, metadata) = delete_entry_row(entry);
        metadata_labels.push(metadata.downgrade());
        files.append(&row);
    }
    if let Some(summary) = delete_confirmation_overflow_label(hidden) {
        let overflow = gtk::Label::new(Some(&summary));
        overflow.set_xalign(0.0);
        overflow.add_css_class("delete-confirmation-file-metadata");
        files.append(&overflow);
    }
    let file_scroller = gtk::ScrolledWindow::builder()
        .child(&files)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(if count > 10 {
            gtk::PolicyType::Automatic
        } else {
            gtk::PolicyType::Never
        })
        .max_content_height(256)
        .propagate_natural_height(true)
        .build();
    file_scroller.add_css_class("delete-confirmation-list");
    (file_scroller, metadata_labels)
}

impl ViewState {
    pub(super) fn show_delete_dialog(self: &Rc<Self>, entries: Vec<FileEntry>, kind: DeleteDialog) {
        let Some(host) = ModalHost::blurred_for(&self.overlay) else {
            return;
        };

        let reach = deletion_reach(&entries);
        let request = Rc::new(DeleteRequest {
            entries: Rc::new(entries),
            kind,
            reach,
            confirmed: Cell::new(false),
        });
        let layout = request.layout();
        let (visible, hidden) = delete_confirmation_rows(&request.entries);
        let (file_scroller, metadata_labels) =
            delete_file_list(visible, hidden, request.entries.len());
        layout.body.append(&file_scroller);
        for paragraph in request.paragraphs() {
            layout.body.append(&message_dialog_description(&paragraph));
        }

        let modal = TrashModal::new(&layout.content, host);
        // Connected first, so the listing's focus then follows the cursor it puts back.
        self.return_cursor_on_cancel(&modal.layer, &request);
        remember_modal_focus_for_listing(&modal.layer, &modal.overlay);
        modal.show();
        for button in [&layout.cancel, &layout.close] {
            let modal = modal.clone();
            button.connect_clicked(move |_| modal.dismiss());
        }
        self.connect_delete_confirm(&layout.confirm, &modal, &request);
        install_delete_dialog_keys(&modal, &layout, request.trash());
        if kind == DeleteDialog::TrashUnavailable {
            focus_button(&layout.cancel);
        } else {
            focus_button(&layout.confirm);
        }
        if !request.needs_size_task(visible) {
            return;
        }
        spawn_delete_size_task(&modal.layer, &layout, &request, metadata_labels);
    }

    fn connect_delete_confirm(
        self: &Rc<Self>,
        confirm: &gtk::Button,
        modal: &TrashModal,
        request: &Rc<DeleteRequest>,
    ) {
        let modal = modal.clone();
        let pending_entries = Rc::new(RefCell::new(Some(request.entries.clone())));
        let trash = request.trash();
        let reach = request.reach;
        let browser = self.browser.clone();
        let weak_ui = Rc::downgrade(self);
        let confirmed = Rc::clone(request);
        confirm.connect_clicked(move |_| {
            let Some(entries) = pending_entries.borrow_mut().take() else {
                return;
            };
            confirmed.confirmed.set(true);
            // Closing the dialog returns focus to the listing, which leaves it without a
            // current allocation to snapshot until the next frame.
            if !trash && let Some(ui) = weak_ui.upgrade() {
                ui.prepare_delete_dissolve(&entries);
            }
            let browser = browser.clone();
            let weak_ui = weak_ui.clone();
            modal.dismiss_then(move || {
                if trash {
                    move_confirmed_to_trash(&weak_ui, entries);
                } else {
                    delete_confirmed_permanently(&weak_ui, &browser, entries, reach);
                }
            });
        });
    }

    /// A completion that moves the cursor while the dialog is open, such as a
    /// compression selecting its archive, gives way when the dialog is cancelled:
    /// the cursor and selection return to the items it asked about.
    fn return_cursor_on_cancel(self: &Rc<Self>, layer: &gtk::Box, request: &Rc<DeleteRequest>) {
        let Some((depth, _, cursor)) = self.browser.focused_item() else {
            return;
        };
        let locations = std::iter::once(&cursor)
            .chain(request.entries.iter())
            .map(|entry| entry.location.clone())
            .collect::<Vec<_>>();
        let weak = Rc::downgrade(self);
        let request = Rc::clone(request);
        layer.connect_parent_notify(move |layer| {
            if layer.parent().is_some() || request.confirmed.get() {
                return;
            }
            let Some(state) = weak.upgrade() else {
                return;
            };
            let stayed = state
                .browser
                .focused_item()
                .is_some_and(|(now, _, entry)| now == depth && entry.location == locations[0]);
            if !stayed {
                state
                    .browser
                    .select_shown_entries_by_location_at(depth, &locations);
            }
        });
    }

    fn prepare_delete_dissolve(&self, entries: &[FileEntry]) {
        self.reset_delete_animation();
        let depth = self.deletion_depth(entries);
        let dissolve = self.delete_animation_source_at(depth).and_then(|source| {
            crate::ui::browser::dissolve_delete::prepare_dissolve(&source, entries)
        });
        if let (Some(depth), Some(dissolve)) = (depth, dissolve) {
            self.pending_delete_dissolve
                .replace(Some((depth, dissolve)));
        }
    }
}

fn unshare_entries(entries: Rc<Vec<FileEntry>>) -> Vec<FileEntry> {
    Rc::try_unwrap(entries).unwrap_or_else(|shared| shared.as_ref().clone())
}

fn move_confirmed_to_trash(weak_ui: &Weak<ViewState>, entries: Rc<Vec<FileEntry>>) {
    if let Some(ui) = weak_ui.upgrade() {
        let entries = unshare_entries(entries);
        ui.move_to_trash(entries);
    }
}

fn delete_confirmed_permanently(
    weak_ui: &Weak<ViewState>,
    browser: &Rc<Browser>,
    entries: Rc<Vec<FileEntry>>,
    reach: DeletionReach,
) {
    if let Some(ui) = weak_ui.upgrade() {
        ui.leave_folder_being_deleted(&entries);
    }
    let entries = unshare_entries(entries);
    browser.delete_permanently(entries, reach == DeletionReach::Remote);
    browser.focus_active();
}

fn install_delete_dialog_keys(modal: &TrashModal, layout: &ModalLayout, trash: bool) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let escaped = modal.clone();
    let focused_cancel = layout.cancel.clone();
    let focused_confirm = layout.confirm.clone();
    let enter_buttons = [
        layout.cancel.clone(),
        layout.confirm.clone(),
        layout.close.clone(),
    ];
    keys::on_press(&keys, move |_, press| {
        if press.is(ESCAPE_ANY) {
            escaped.dismiss();
            glib::Propagation::Stop
        } else if press.is(ENTER_ANY) {
            activate_focused_button(&enter_buttons)
        } else if trash && is_tenxer_delete_confirm(press) {
            focused_confirm.emit_clicked();
            glib::Propagation::Stop
        } else if let Some(target) = delete_confirmation_focus_target(press) {
            move_delete_focus(target, &focused_cancel, &focused_confirm);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    modal.layer.add_controller(keys);
}

fn is_tenxer_delete_confirm(press: &KeyPress) -> bool {
    press.is(TENXER_DELETE_CONFIRM)
        && crate::ui::preferences::PreferenceManager::shared().tenxer_mode()
}

fn activate_focused_button(buttons: &[gtk::Button]) -> glib::Propagation {
    let Some(button) = buttons.iter().find(|button| button.has_focus()) else {
        return glib::Propagation::Proceed;
    };
    button.emit_clicked();
    glib::Propagation::Stop
}

fn move_delete_focus(target: DeleteConfirmationFocus, cancel: &gtk::Button, confirm: &gtk::Button) {
    match target {
        DeleteConfirmationFocus::Cancel => cancel.grab_focus(),
        DeleteConfirmationFocus::Confirm => confirm.grab_focus(),
    };
    if let Some(window) = cancel.root().and_downcast::<gtk::Window>() {
        window.set_focus_visible(true);
    }
}

fn permanent_delete_subtitle(
    reach: DeletionReach,
    summary: &crate::adapters::directory_summary::DirectorySummary,
) -> String {
    rust_i18n::t!(
        match (reach, summary.truncated()) {
            (DeletionReach::Local, true) => {
                "At least %{items} · at least %{size} will be permanently deleted"
            }
            (DeletionReach::Local, false) => {
                "%{items} · %{size} will be permanently deleted"
            }
            (DeletionReach::Remote, true) => {
                "At least %{items} · at least %{size} will be deleted"
            }
            (DeletionReach::Remote, false) => "%{items} · %{size} will be deleted",
        },
        items = item_count_label(summary.item_count),
        size = format_file_size(summary.total_size)
    )
    .into_owned()
}

fn set_metadata_size(
    metadata_labels: &[glib::WeakRef<gtk::Label>],
    index: usize,
    size: &MetadataValue<u64>,
) {
    if let Some(label) = metadata_labels.get(index).and_then(|label| label.upgrade()) {
        label.set_label(&delete_confirmation_file_size(size));
    }
}

async fn size_visible_file_links(
    entries: &[FileEntry],
    metadata_labels: &[glib::WeakRef<gtk::Label>],
) {
    for (index, entry) in entries.iter().take(metadata_labels.len()).enumerate() {
        if entry.kind == EntryKind::FileSymbolicLink {
            set_metadata_size(metadata_labels, index, &delete_entry_size(entry).await);
        }
    }
}

/// Aborted once the dialog starts closing or leaves the window.
fn spawn_delete_size_task(
    layer: &gtk::Box,
    layout: &ModalLayout,
    request: &Rc<DeleteRequest>,
    metadata_labels: Vec<glib::WeakRef<gtk::Label>>,
) {
    let weak_subtitle = layout.subtitle.downgrade();
    let weak_spinner = layout.loading.downgrade();
    let request = request.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        if request.trash() {
            size_visible_file_links(&request.entries, &metadata_labels).await;
            return;
        }
        let summary = aggregate_directory_summary(&request.entries, |index, size| {
            set_metadata_size(&metadata_labels, index, size);
        })
        .await;
        let (Some(subtitle), Some(spinner)) = (weak_subtitle.upgrade(), weak_spinner.upgrade())
        else {
            return;
        };
        subtitle.set_label(&permanent_delete_subtitle(request.reach, &summary));
        spinner.stop();
        spinner.set_visible(false);
    });
    let task = Rc::new(task);
    let closing_task = task.clone();
    layer.connect_sensitive_notify(move |layer| {
        if !layer.is_sensitive() {
            closing_task.abort();
        }
    });
    layer.connect_unrealize(move |_| task.abort());
}
