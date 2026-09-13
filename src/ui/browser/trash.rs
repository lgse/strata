// SPDX-License-Identifier: MIT

use crate::adapters::directory_summary::{DirectorySummary, summarize_directory};
use crate::adapters::trash::{EmptyTrashOutcome, empty_trash};
use crate::adapters::trash_restore::restore_destinations_for_locations;
use crate::model::{FileEntry, Location};
use crate::services::{LoadHandle, RestoreTrashItem};
use crate::ui::blur::BlurBin;
use crate::ui::browser::entry::{
    entry_icon, entry_kind_summary, format_file_size, item_count_label,
};
use crate::ui::browser::{ViewState, vim_focus_direction};
use crate::ui::controls::{
    ModalTone, message_dialog_description, message_dialog_layout, modal_layout,
};
use crate::ui::modal::{
    ModalHost, dismiss_modal_layer, dismiss_modal_layer_then, modal_layer, show_error_dialog,
};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(super) struct TrashLoadingView {
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<BlurBin>,
}

fn empty_trash_error_summary(outcome: &EmptyTrashOutcome) -> String {
    let mut summary = format!(
        "{} could not be deleted. The remaining items were processed.",
        item_count_label(outcome.failed)
    );
    for error in &outcome.errors {
        summary.push_str("\n\n• ");
        summary.push_str(error);
    }
    if outcome.failed > outcome.errors.len() {
        summary.push_str(&format!(
            "\n\n…and {} more",
            outcome.failed - outcome.errors.len()
        ));
    }
    summary
}

/// Narrows a just-attempted delete's entries down to the ones a completed
/// operation named as retryable, so a permanent-delete retry (issue #179)
/// re-targets exactly those and not, say, ones that already succeeded or
/// failed for an unrelated reason.
pub(super) fn retryable_delete_entries(
    entries: Vec<FileEntry>,
    retryable_locations: &[Location],
) -> Vec<FileEntry> {
    entries
        .into_iter()
        .filter(|entry| retryable_locations.contains(&entry.location))
        .collect()
}

fn restore_confirmation_title(count: usize) -> String {
    format!("Restore {}?", item_count_label(count))
}

fn restore_confirmation_confirm_label(count: usize) -> String {
    if count == 1 {
        "Restore".to_owned()
    } else {
        format!("Restore {}", item_count_label(count))
    }
}

fn restore_destination_text(destination: &Path) -> String {
    destination.to_string_lossy().into_owned()
}

fn restore_error_summary(errors: &[String]) -> String {
    let mut summary = if errors.len() == 1 {
        errors[0].clone()
    } else {
        format!("{} could not be restored.", item_count_label(errors.len()))
    };
    if errors.len() > 1 {
        for error in errors.iter().take(8) {
            summary.push_str("\n\n• ");
            summary.push_str(error);
        }
        if errors.len() > 8 {
            summary.push_str(&format!("\n\n…and {} more", errors.len() - 8));
        }
    }
    summary
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeleteConfirmationFocus {
    Cancel,
    Confirm,
}

fn delete_confirmation_focus_target(key: gtk::gdk::Key) -> Option<DeleteConfirmationFocus> {
    match key {
        gtk::gdk::Key::Left | gtk::gdk::Key::h => Some(DeleteConfirmationFocus::Cancel),
        gtk::gdk::Key::Right | gtk::gdk::Key::l => Some(DeleteConfirmationFocus::Confirm),
        _ => None,
    }
}

/// Rows rendered in the delete confirmation; anything beyond this collapses
/// into a summary so a thousand-file selection cannot stall the UI thread.
const DELETE_CONFIRMATION_MAX_ROWS: usize = 50;

/// Splits confirmation entries into rendered rows and the hidden remainder.
fn delete_confirmation_rows(entries: &[FileEntry]) -> (&[FileEntry], usize) {
    let visible = entries.len().min(DELETE_CONFIRMATION_MAX_ROWS);
    (&entries[..visible], entries.len() - visible)
}

/// Summary for the hidden remainder, or nothing when everything is shown.
fn delete_confirmation_overflow_label(hidden: usize) -> Option<String> {
    (hidden > 0).then(|| {
        format!(
            "… and {} more {}",
            hidden,
            if hidden == 1 { "item" } else { "items" }
        )
    })
}

impl ViewState {
    pub(super) fn trash_dropped_locations(self: &Rc<Self>, sources: Vec<Location>) {
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let mut entries = Vec::with_capacity(sources.len());
            for source in sources {
                match crate::adapters::query_file_entry(source).await {
                    Ok(entry) => entries.push(entry),
                    Err(error) => {
                        if let Some(state) = weak.upgrade() {
                            show_error_dialog(
                                &state.overlay,
                                "Unable to move to Trash",
                                &error.to_string(),
                            );
                        }
                        return;
                    }
                }
            }
            if let Some(state) = weak.upgrade() {
                state.request_delete(entries, false);
            }
        });
    }

    pub(super) fn delete_animation_source(&self) -> Option<gtk::Widget> {
        if self.mode_views.borrow().mode() == crate::ui::browser_modes::BrowserMode::Columns {
            let depth = self.browser.active_depth()?;
            self.columns
                .borrow()
                .get(depth)
                .map(|column| column.shell.clone().upcast())
        } else {
            Some(self.overlay.clone().upcast())
        }
    }

    pub(super) fn clear_delete_animation(&self) {
        self.pending_delete_dissolve.take();
    }

    pub(super) fn delete_animation_defers_empty_state(&self, depth: usize) -> bool {
        self.pending_delete_dissolve
            .borrow()
            .as_ref()
            .is_some_and(|(pending_depth, _)| *pending_depth == depth)
            || self.deferred_delete_empty_depth.get() == Some(depth)
    }

    /// Safe to call more than once: whichever of cancel or completion runs first leaves the
    /// other a no-op.
    fn clear_empty_trash(&self) {
        self.pending_empty_trash.borrow_mut().take();
        self.dismiss_file_operation_progress();
    }

    pub(super) fn load_trash_summary(self: &Rc<Self>) {
        let trash_empty = self
            .columns
            .borrow()
            .iter()
            .find(|column| column.empty_trash_button.is_some())
            .is_some_and(|column| column.entry_count.get() == 0);
        if trash_empty {
            return;
        }
        if !self.show_trash_loading_indicator(
            "Measuring Trash…",
            "Empty Trash",
            "Calculating the number and size of items. This may take a few seconds.",
        ) {
            return;
        }
        let weak = Rc::downgrade(self);
        let started = Instant::now();
        let task = glib::MainContext::default().spawn_local(async move {
            // Let GTK paint the loading dialog before beginning a walk whose GIO futures may be
            // immediately ready for long stretches on a fast local trash backend.
            glib::timeout_future(Duration::from_millis(16)).await;
            let trash = gio::File::for_uri("trash:///");
            match summarize_directory(&trash).await {
                Ok(summary) if summary.item_count > 0 => {
                    if summary.truncated() {
                        tracing::warn!(
                            item_count = summary.item_count,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "trash summary truncated"
                        );
                    } else {
                        tracing::info!(
                            item_count = summary.item_count,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "trash summary built"
                        );
                    }
                    if let Some(state) = weak.upgrade() {
                        state.clear_trash_loading();
                        state.show_empty_trash_confirmation(summary);
                    }
                }
                Ok(_) => {
                    if let Some(state) = weak.upgrade() {
                        state.clear_trash_loading();
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        error_domain = ?error.domain(),
                        error_code = error.code(),
                        "trash summary failed"
                    );
                    if let Some(state) = weak.upgrade() {
                        state.clear_trash_loading();
                        show_error_dialog(
                            &state.overlay,
                            "Unable to read Trash",
                            &error.to_string(),
                        );
                    }
                }
            }
        });
        self.pending_trash_lookup
            .replace(Some(LoadHandle::new(move || {
                tracing::debug!("trash summary cancelled");
                task.abort();
            })));
    }

    /// The walk is bounded but can still take a few seconds on a large trash, hence the indicator.
    fn show_trash_loading_indicator(
        self: &Rc<Self>,
        title: &str,
        action: &str,
        detail: &str,
    ) -> bool {
        self.clear_trash_loading();
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            show_error_dialog(
                &self.overlay,
                "Unable to continue",
                "The operation could not be confirmed.",
            );
            return false;
        };

        let layout = modal_layout(crate::assets::icons::TRASH, title, "", action);
        layout.set_loading(true, Some(title));
        layout.subtitle.set_visible(false);
        layout.confirm.set_visible(false);
        let explanation = message_dialog_description(detail);
        layout.body.append(&explanation);
        let content = layout.content;
        let cancel = layout.cancel;
        let close = layout.close;

        let layer = modal_layer(
            &content,
            &window_overlay,
            blurred_root.clone(),
            Some(Rc::new(|| true)),
        );
        window_overlay.add_overlay(&layer);
        self.trash_loading.replace(Some(TrashLoadingView {
            layer,
            overlay: window_overlay,
            blurred_root,
        }));

        for button in [&cancel, &close] {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.clear_trash_loading();
                }
            });
        }
        let escape = gtk::EventControllerKey::new();
        let weak_escape = Rc::downgrade(self);
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(state) = weak_escape.upgrade() {
                    state.clear_trash_loading();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        if let Some(view) = self.trash_loading.borrow().as_ref() {
            view.layer.add_controller(escape);
        }
        cancel.grab_focus();
        true
    }

    fn dismiss_trash_loading(&self) {
        let Some(view) = self.trash_loading.take() else {
            return;
        };
        dismiss_modal_layer(&view.layer, &view.overlay, view.blurred_root.as_ref());
    }

    /// Safe to call more than once: whichever of cancel or completion runs first leaves the
    /// other a no-op.
    fn clear_trash_loading(&self) {
        self.pending_trash_lookup.borrow_mut().take();
        self.dismiss_trash_loading();
    }

    fn show_empty_trash_confirmation(self: &Rc<Self>, summary: DirectorySummary) {
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };

        let layout = message_dialog_layout(
            crate::assets::icons::TRASH,
            "Empty Trash?",
            &format!(
                "{}{} · {}{} will be reclaimed",
                if summary.truncated() { "At least " } else { "" },
                item_count_label(summary.item_count),
                if summary.truncated() { "at least " } else { "" },
                format_file_size(summary.total_size)
            ),
            "Empty Trash",
            ModalTone::Danger,
        );
        let explanation = message_dialog_description(
            "Everything in Trash will be permanently deleted. This action cannot be undone.",
        );
        layout.body.append(&explanation);
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let empty = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        let cancel_layer = layer.clone();
        let cancel_overlay = window_overlay.clone();
        let cancel_root = blurred_root.clone();
        let cancel_browser = self.browser.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
            cancel_browser.focus_active();
        });
        let close_layer = layer.clone();
        let close_overlay = window_overlay.clone();
        let close_root = blurred_root.clone();
        let close_browser = self.browser.clone();
        close.connect_clicked(move |_| {
            dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
            close_browser.focus_active();
        });
        let empty_layer = layer.clone();
        let empty_overlay = window_overlay.clone();
        let empty_root = blurred_root.clone();
        let browser = self.browser.clone();
        let error_overlay = self.overlay.clone();
        let weak_ui = Rc::downgrade(self);
        empty.connect_clicked(move |_| {
            dismiss_modal_layer(&empty_layer, &empty_overlay, empty_root.as_ref());
            browser.focus_active();

            let cancel_ui = weak_ui.clone();
            let on_cancel: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(ui) = cancel_ui.upgrade() {
                    ui.clear_empty_trash();
                }
            });
            if let Some(ui) = weak_ui.upgrade() {
                ui.show_empty_trash_progress(on_cancel);
            }

            let error_overlay = error_overlay.clone();
            let progress_ui = weak_ui.clone();
            let finish_ui = weak_ui.clone();
            let finish_browser = browser.clone();
            let task = glib::MainContext::default().spawn_local(async move {
                let trash = gio::File::for_uri("trash:///");
                let result = empty_trash(&trash, move |processed| {
                    if let Some(ui) = progress_ui.upgrade() {
                        ui.update_empty_trash_progress(processed);
                    }
                })
                .await;
                if let Some(ui) = finish_ui.upgrade() {
                    ui.clear_empty_trash();
                }
                match result {
                    Ok(outcome) => {
                        // A wholesale refresh rather than tracking each deleted location, to
                        // keep this operation's own state bounded too.
                        finish_browser.refresh_columns_at(&Location::uri("trash:///"));
                        if outcome.failed > 0 {
                            show_error_dialog(
                                &error_overlay,
                                "Completed with errors",
                                &empty_trash_error_summary(&outcome),
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            error_domain = ?error.domain(),
                            error_code = error.code(),
                            "empty trash failed"
                        );
                        show_error_dialog(
                            &error_overlay,
                            "Unable to empty Trash",
                            &error.to_string(),
                        );
                    }
                }
            });
            if let Some(ui) = weak_ui.upgrade() {
                ui.pending_empty_trash
                    .replace(Some(LoadHandle::new(move || {
                        tracing::debug!("empty trash cancelled");
                        task.abort();
                    })));
            }
        });
        let keys = gtk::EventControllerKey::new();
        let escape_layer = layer.clone();
        let focused_layer = layer.clone();
        let escape_overlay = window_overlay.clone();
        let escape_root = blurred_root.clone();
        let escape_browser = self.browser.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
                escape_browser.focus_active();
                glib::Propagation::Stop
            } else if !modifiers
                .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
                && let Some(direction) = vim_focus_direction(key)
            {
                focused_layer.child_focus(direction);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(keys);
        let initial_focus = cancel.clone();
        glib::idle_add_local_once(move || {
            initial_focus.grab_focus();
            if let Some(window) = initial_focus.root().and_downcast::<gtk::Window>() {
                window.set_focus_visible(true);
            }
        });
    }

    pub(super) fn request_restore(self: &Rc<Self>, entries: Vec<FileEntry>) {
        if entries.is_empty() {
            return;
        }
        if !self.show_trash_loading_indicator(
            "Finding restore destinations…",
            "Restore",
            "Checking the original locations before confirmation. You can cancel this lookup.",
        ) {
            return;
        }
        let weak = Rc::downgrade(self);
        let task = glib::MainContext::default().spawn_local(async move {
            glib::timeout_future(Duration::from_millis(16)).await;
            let lookups = entries
                .iter()
                .map(|entry| (entry.location.clone(), entry.thumbnail_path.clone()))
                .collect::<Vec<_>>();
            let mut resolved = Vec::new();
            let mut errors = Vec::new();
            for (entry, destination) in entries
                .into_iter()
                .zip(restore_destinations_for_locations(lookups).await)
            {
                match destination {
                    Ok(destination) => resolved.push((entry, destination)),
                    Err(error) => errors.push(format!("{}: {error}", entry.display_name)),
                }
            }
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.clear_trash_loading();
            if resolved.is_empty() {
                show_error_dialog(
                    &state.overlay,
                    "Unable to restore",
                    &restore_error_summary(&errors),
                );
                return;
            }
            state.show_restore_confirmation(resolved, errors);
        });
        self.pending_trash_lookup
            .replace(Some(LoadHandle::new(move || task.abort())));
    }

    fn show_restore_confirmation(
        self: &Rc<Self>,
        resolved: Vec<(FileEntry, PathBuf)>,
        skipped: Vec<String>,
    ) {
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            show_error_dialog(
                &self.overlay,
                "Unable to restore",
                "The restore destinations could not be confirmed.",
            );
            return;
        };

        let count = resolved.len();
        let layout = message_dialog_layout(
            crate::assets::icons::FOLDER,
            &restore_confirmation_title(count),
            &entry_kind_summary(
                &resolved
                    .iter()
                    .map(|(entry, _)| entry.clone())
                    .collect::<Vec<_>>(),
            ),
            &restore_confirmation_confirm_label(count),
            ModalTone::Accent,
        );
        let files = gtk::Box::new(gtk::Orientation::Vertical, 3);
        files.add_css_class("delete-confirmation-files");
        for (entry, destination) in &resolved {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("delete-confirmation-file");
            let icon = crate::assets::primary_icon(entry_icon(entry), 16);
            let name = gtk::Label::new(Some(&entry.display_name));
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name.set_xalign(0.0);
            let destination_label = gtk::Label::new(Some(&restore_destination_text(destination)));
            destination_label.add_css_class("delete-confirmation-file-metadata");
            destination_label.set_wrap(true);
            destination_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            destination_label.set_hexpand(true);
            destination_label.set_xalign(1.0);
            destination_label.set_selectable(true);
            destination_label.set_tooltip_text(Some(&destination.to_string_lossy()));
            row.append(&icon);
            row.append(&name);
            row.append(&destination_label);
            files.append(&row);
        }
        let file_scroller = gtk::ScrolledWindow::builder()
            .child(&files)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .max_content_height(256)
            .propagate_natural_height(true)
            .build();
        file_scroller.add_css_class("delete-confirmation-list");
        layout.body.append(&file_scroller);
        let mut explanation =
            "These items will be moved back to the original locations shown above.".to_owned();
        if !skipped.is_empty() {
            explanation.push_str("\n\n");
            explanation.push_str(&restore_error_summary(&skipped));
        }
        layout
            .body
            .append(&message_dialog_description(&explanation));
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let confirm = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        let cancelled_layer = layer.clone();
        let cancelled_overlay = window_overlay.clone();
        let cancelled_root = blurred_root.clone();
        let cancelled_browser = self.browser.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(
                &cancelled_layer,
                &cancelled_overlay,
                cancelled_root.as_ref(),
            );
            cancelled_browser.focus_active();
        });
        let closed_layer = layer.clone();
        let closed_overlay = window_overlay.clone();
        let closed_root = blurred_root.clone();
        let closed_browser = self.browser.clone();
        close.connect_clicked(move |_| {
            dismiss_modal_layer(&closed_layer, &closed_overlay, closed_root.as_ref());
            closed_browser.focus_active();
        });
        let confirmed_layer = layer.clone();
        let confirmed_overlay = window_overlay.clone();
        let confirmed_root = blurred_root.clone();
        let browser = self.browser.clone();
        let items = resolved
            .into_iter()
            .map(|(entry, destination)| RestoreTrashItem { entry, destination })
            .collect::<Vec<_>>();
        confirm.connect_clicked(move |_| {
            dismiss_modal_layer(
                &confirmed_layer,
                &confirmed_overlay,
                confirmed_root.as_ref(),
            );
            browser.restore(items.clone());
            browser.focus_active();
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        let escaped_browser = self.browser.clone();
        let focused_layer = layer.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                escaped_browser.focus_active();
                glib::Propagation::Stop
            } else if !modifiers
                .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
                && let Some(direction) = vim_focus_direction(key)
            {
                focused_layer.child_focus(direction);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(keys);
        let initial_focus = cancel.clone();
        glib::idle_add_local_once(move || {
            initial_focus.grab_focus();
            if let Some(window) = initial_focus.root().and_downcast::<gtk::Window>() {
                window.set_focus_visible(true);
            }
        });
    }

    pub(super) fn request_delete(self: &Rc<Self>, entries: Vec<FileEntry>, permanent: bool) {
        if entries
            .iter()
            .any(|entry| !super::paths::can_remove_location(&entry.location))
        {
            return;
        }
        if permanent {
            self.show_delete_confirmation(entries);
        } else {
            self.pending_delete_entries.replace(entries.clone());
            let weak = Rc::downgrade(self);
            let entries_for_anim = Rc::new(entries.clone());
            let run_delete = move || {
                if let Some(state) = weak.upgrade() {
                    state.browser.delete((*entries_for_anim).clone(), false);
                    state.browser.focus_active();
                }
            };
            if let Some(trash_button) = self.trash_button.borrow().as_ref() {
                super::fly_to_trash::fly_to_trash(
                    self.overlay.upcast_ref(),
                    &entries,
                    trash_button,
                    run_delete,
                );
            } else {
                run_delete();
            }
        }
    }

    pub(super) fn show_delete_confirmation(self: &Rc<Self>, entries: Vec<FileEntry>) {
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };

        let count = entries.len();
        let title = format!("Permanently delete {}?", item_count_label(count));
        let confirm_label = format!("Permanently delete {}", item_count_label(count));
        let layout = message_dialog_layout(
            crate::assets::icons::TRASH,
            &title,
            &entry_kind_summary(&entries),
            &confirm_label,
            ModalTone::Danger,
        );
        let files = gtk::Box::new(gtk::Orientation::Vertical, 3);
        files.add_css_class("delete-confirmation-files");
        let (visible, hidden) = delete_confirmation_rows(&entries);
        for entry in visible {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("delete-confirmation-file");
            let icon = crate::assets::primary_icon(entry_icon(entry), 16);
            let name = gtk::Label::new(Some(&entry.display_name));
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name.set_hexpand(true);
            name.set_xalign(0.0);
            name.set_tooltip_text(Some(&entry.location.display_path()));
            let metadata = gtk::Label::new(Some(&if entry.is_directory() {
                "Folder".to_owned()
            } else {
                match entry.size {
                    crate::model::MetadataValue::Known(size) => format_file_size(size),
                    crate::model::MetadataValue::Unknown
                    | crate::model::MetadataValue::Unavailable => "—".to_owned(),
                }
            }));
            metadata.add_css_class("delete-confirmation-file-metadata");
            row.append(&icon);
            row.append(&name);
            row.append(&metadata);
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
        file_scroller.add_css_class("fixed-scrollbar");
        layout.body.append(&file_scroller);
        let explanation = message_dialog_description(
            "These items will be permanently deleted. This action cannot be undone.",
        );
        layout.body.append(&explanation);
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let confirm = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        let cancelled_layer = layer.clone();
        let cancelled_overlay = window_overlay.clone();
        let cancelled_root = blurred_root.clone();
        let cancelled_browser = self.browser.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(
                &cancelled_layer,
                &cancelled_overlay,
                cancelled_root.as_ref(),
            );
            cancelled_browser.focus_active();
        });
        let closed_layer = layer.clone();
        let closed_overlay = window_overlay.clone();
        let closed_root = blurred_root.clone();
        let closed_browser = self.browser.clone();
        close.connect_clicked(move |_| {
            dismiss_modal_layer(&closed_layer, &closed_overlay, closed_root.as_ref());
            closed_browser.focus_active();
        });
        let confirmed_layer = layer.clone();
        let confirmed_overlay = window_overlay.clone();
        let confirmed_root = blurred_root.clone();
        let browser = self.browser.clone();
        let entries_for_dissolve = entries.clone();
        let weak_ui = Rc::downgrade(self);
        confirm.connect_clicked(move |_| {
            let browser = browser.clone();
            let entries_for_dissolve = entries_for_dissolve.clone();
            let weak_ui = weak_ui.clone();
            dismiss_modal_layer_then(
                &confirmed_layer,
                &confirmed_overlay,
                confirmed_root.as_ref(),
                move || {
                    if let Some(ui) = weak_ui.upgrade() {
                        let dissolve = ui.delete_animation_source().and_then(|source| {
                            super::dissolve_delete::prepare_dissolve(&source, &entries_for_dissolve)
                        });
                        ui.clear_delete_animation();
                        if let (Some(depth), Some(dissolve)) = (ui.browser.active_depth(), dissolve)
                        {
                            ui.pending_delete_dissolve.replace(Some((depth, dissolve)));
                        }
                    }
                    browser.delete(entries_for_dissolve, true);
                    browser.focus_active();
                },
            );
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        let escaped_browser = self.browser.clone();
        let focused_cancel = cancel.clone();
        let focused_confirm = confirm.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                escaped_browser.focus_active();
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                focused_confirm.emit_clicked();
                glib::Propagation::Stop
            } else if !modifiers
                .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
            {
                match delete_confirmation_focus_target(key) {
                    Some(DeleteConfirmationFocus::Cancel) => focused_cancel.grab_focus(),
                    Some(DeleteConfirmationFocus::Confirm) => focused_confirm.grab_focus(),
                    None => return glib::Propagation::Proceed,
                };
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(keys);
        let initial_focus = confirm.clone();
        glib::idle_add_local_once(move || {
            initial_focus.grab_focus();
            if let Some(window) = initial_focus.root().and_downcast::<gtk::Window>() {
                window.set_focus_visible(true);
            }
        });
    }
}

#[cfg(test)]
mod tests;
