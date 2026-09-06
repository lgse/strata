// SPDX-License-Identifier: GPL-3.0-or-later

use crate::adapters::gio_file_for_location;
use crate::model::{FileEntry, Location};
use crate::services::{MoveRecord, PasteItem, TransferConflict, UndoMoveItem};
use crate::ui::browser::ViewState;
use crate::ui::browser::destination::{
    folder_input_path, resolve_destination_path, setup_transfer_search,
};
use crate::ui::browser::entry::item_count_label;
use crate::ui::browser::paths::{compact_display_path, compact_native_path};
use crate::ui::controls::{
    ModalTone, form_check_button, form_entry, form_label, message_dialog_description,
    message_dialog_layout, modal_layout,
};
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

#[derive(Clone, Copy)]
enum ConflictChoice {
    Replace,
    Skip,
}

fn location_exists(location: &Location) -> bool {
    gio_file_for_location(location).query_exists(None::<&gio::Cancellable>)
}

fn transfer_has_collision(source: &Location, destination: &Location) -> bool {
    let source = gio_file_for_location(source);
    let destination = gio_file_for_location(destination);
    let Some(name) = source.basename() else {
        return false;
    };
    let target = destination.child(name);
    if source.equal(&target) || source.equal(&destination) || destination.has_prefix(&source) {
        return false;
    }
    target.query_exists(None::<&gio::Cancellable>)
}

pub(super) fn duplicate_transfer(entries: &[FileEntry]) -> Option<(Location, Vec<Location>)> {
    let destination = entries.first()?.location.parent()?;
    if !entries
        .iter()
        .all(|entry| entry.location.parent().as_ref() == Some(&destination))
    {
        return None;
    }
    let sources = entries.iter().map(|entry| entry.location.clone()).collect();
    Some((destination, sources))
}

impl ViewState {
    pub(super) fn start_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        let mut accepted = Vec::new();
        let mut collisions = Vec::new();
        for source in sources {
            if transfer_has_collision(&source, &destination) {
                collisions.push(source);
            } else {
                accepted.push(PasteItem {
                    source,
                    conflict: TransferConflict::FailIfExists,
                });
            }
        }
        self.resolve_transfer_collisions(destination, collisions, accepted, move_sources);
    }

    fn resolve_transfer_collisions(
        self: &Rc<Self>,
        destination: Location,
        mut collisions: Vec<Location>,
        accepted: Vec<PasteItem>,
        move_sources: bool,
    ) {
        if collisions.is_empty() {
            self.browser.transfer(destination, accepted, move_sources);
            return;
        }
        let source = collisions.remove(0);
        let name = source.display_name();
        let explanation = format!(
            "An item named \u{201c}{name}\u{201d} already exists in {}. Replacing it will overwrite its contents.",
            compact_display_path(&destination)
        );
        let state = self.clone();
        self.confirm_replace_conflict(
            &name,
            &explanation,
            !collisions.is_empty(),
            Rc::new(move |choice, apply_to_all| {
                let mut accepted = accepted.clone();
                let mut remaining = collisions.clone();
                match choice {
                    ConflictChoice::Replace => {
                        accepted.push(PasteItem {
                            source: source.clone(),
                            conflict: TransferConflict::ReplaceExisting,
                        });
                        if apply_to_all {
                            accepted.extend(remaining.drain(..).map(|source| PasteItem {
                                source,
                                conflict: TransferConflict::ReplaceExisting,
                            }));
                        }
                    }
                    ConflictChoice::Skip if apply_to_all => remaining.clear(),
                    ConflictChoice::Skip => {}
                }
                state.resolve_transfer_collisions(
                    destination.clone(),
                    remaining,
                    accepted,
                    move_sources,
                );
            }),
        );
    }

    /// Moves the latest completed transfer back, confirming any item that would
    /// overwrite something created since the move.
    pub(super) fn undo_move(self: &Rc<Self>, generation: u64, records: Vec<MoveRecord>) -> bool {
        let mut accepted = Vec::new();
        let mut collisions = Vec::new();
        for record in records {
            if !location_exists(&record.current) {
                continue;
            }
            if location_exists(&record.original) {
                collisions.push(record);
            } else {
                accepted.push(UndoMoveItem {
                    record,
                    conflict: TransferConflict::FailIfExists,
                });
            }
        }
        if accepted.is_empty() && collisions.is_empty() {
            self.browser.discard_pending_undo(generation);
            return false;
        }
        self.resolve_undo_collisions(generation, collisions, accepted);
        true
    }

    pub(super) fn undo_copy(self: &Rc<Self>, generation: u64, locations: Vec<Location>) -> bool {
        let existing = locations
            .into_iter()
            .filter(location_exists)
            .collect::<Vec<_>>();
        if existing.is_empty() {
            self.browser.discard_pending_undo(generation);
            return false;
        }
        self.browser.undo_copy(generation, existing)
    }

    fn resolve_undo_collisions(
        self: &Rc<Self>,
        generation: u64,
        mut collisions: Vec<MoveRecord>,
        accepted: Vec<UndoMoveItem>,
    ) {
        if collisions.is_empty() {
            if accepted.is_empty() {
                self.browser.discard_pending_undo(generation);
            } else {
                self.browser.undo_move(generation, accepted);
            }
            return;
        }
        let record = collisions.remove(0);
        let name = record.original.display_name();
        let parent = record
            .original
            .parent()
            .unwrap_or_else(|| record.original.clone());
        let explanation = format!(
            "An item named \u{201c}{name}\u{201d} already exists in {}. Undoing the move will overwrite its contents.",
            compact_display_path(&parent)
        );
        let state = self.clone();
        self.confirm_replace_conflict(
            &name,
            &explanation,
            !collisions.is_empty(),
            Rc::new(move |choice, apply_to_all| {
                let mut accepted = accepted.clone();
                let mut remaining = collisions.clone();
                match choice {
                    ConflictChoice::Replace => {
                        accepted.push(UndoMoveItem {
                            record: record.clone(),
                            conflict: TransferConflict::ReplaceExisting,
                        });
                        if apply_to_all {
                            accepted.extend(remaining.drain(..).map(|record| UndoMoveItem {
                                record,
                                conflict: TransferConflict::ReplaceExisting,
                            }));
                        }
                    }
                    ConflictChoice::Skip if apply_to_all => remaining.clear(),
                    ConflictChoice::Skip => {}
                }
                state.resolve_undo_collisions(generation, remaining, accepted);
            }),
        );
    }

    /// Asks whether one conflicting item should be replaced or skipped.
    /// Cancelling abandons the whole operation, so `on_choice` never runs.
    fn confirm_replace_conflict(
        &self,
        name: &str,
        explanation: &str,
        has_more_conflicts: bool,
        on_choice: Rc<dyn Fn(ConflictChoice, bool)>,
    ) {
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };

        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            "File already exists",
            name,
            "Replace",
            ModalTone::Danger,
        );
        layout.body.append(&message_dialog_description(explanation));
        let apply_all = form_check_button("Apply this choice to all remaining conflicts");
        apply_all.set_visible(has_more_conflicts);
        layout.body.append(&apply_all);
        let skip = gtk::Button::with_label("Skip");
        skip.add_css_class("action-dialog-cancel");
        layout
            .actions
            .insert_child_after(&skip, Some(&layout.cancel));
        let content = layout.content;
        let cancel = layout.cancel;
        let replace = layout.confirm;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);
        let cancel_layer = layer.clone();
        let cancel_overlay = window_overlay.clone();
        let cancel_root = blurred_root.clone();
        let dismiss_layer = cancel_layer.clone();
        let dismiss_overlay = cancel_overlay.clone();
        let dismiss_root = cancel_root.clone();
        cancel.connect_clicked(move |_| {
            dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
        });
        layout.close.connect_clicked(move |_| {
            dismiss_modal_layer(&dismiss_layer, &dismiss_overlay, dismiss_root.as_ref());
        });

        for (button, choice) in [
            (skip.clone(), ConflictChoice::Skip),
            (replace.clone(), ConflictChoice::Replace),
        ] {
            let chosen_layer = layer.clone();
            let chosen_overlay = window_overlay.clone();
            let chosen_root = blurred_root.clone();
            let chosen_apply_all = apply_all.clone();
            let chosen = on_choice.clone();
            button.connect_clicked(move |_| {
                dismiss_modal_layer(&chosen_layer, &chosen_overlay, chosen_root.as_ref());
                chosen(choice, chosen_apply_all.is_active());
            });
        }

        let escape = gtk::EventControllerKey::new();
        escape.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        let enter_replace = replace.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                enter_replace.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        replace.grab_focus();
    }

    pub(super) fn show_transfer_dialog(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        move_sources: bool,
    ) {
        if entries.is_empty() {
            return;
        }
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };

        let base = self
            .browser
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf))
            .unwrap_or_else(glib::home_dir);
        let layout = modal_layout(
            if move_sources {
                crate::assets::icons::FOLDER
            } else {
                crate::assets::icons::COPY
            },
            if move_sources { "Move to" } else { "Copy to" },
            &format!(
                "Choose a destination for {}",
                item_count_label(entries.len())
            ),
            if move_sources {
                "Move here"
            } else {
                "Copy here"
            },
        );
        layout.content.add_css_class("wide");
        let field_label = form_label("Destination folder");
        let field = form_entry();
        field.set_hexpand(true);
        field.set_placeholder_text(Some("Search for a folder…"));
        field.set_text(&folder_input_path(&base));
        field.set_position(-1);
        layout.body.append(&field_label);
        layout.body.append(&field);

        let suggestions = gtk::Box::new(gtk::Orientation::Vertical, 2);
        suggestions.add_css_class("transfer-suggestions");
        let suggestion_scroll = gtk::ScrolledWindow::builder()
            .child(&suggestions)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(150)
            .max_content_height(220)
            .propagate_natural_height(true)
            .build();
        suggestion_scroll.add_css_class("transfer-suggestion-scroll");
        layout.body.append(&suggestion_scroll);
        let error = gtk::Label::new(None);
        error.add_css_class("form-message");
        error.add_css_class("error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        layout.body.append(&error);
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let confirm = layout.confirm;

        let generation = Rc::new(Cell::new(0_u64));
        let pending_creation = Rc::new(RefCell::new(None::<std::path::PathBuf>));
        let creating_destination = Rc::new(Cell::new(false));
        let suggestions_box = suggestions.clone();
        let suggestions_error = error.clone();
        let changed_confirm = confirm.clone();
        let changed_creation = pending_creation.clone();
        setup_transfer_search(
            &field,
            &suggestions_box,
            &generation,
            base.clone(),
            self.browser.preferences().show_hidden,
            move |field| {
                field.remove_css_class("error");
                suggestions_error.set_visible(false);
                suggestions_error.remove_css_class("warning");
                suggestions_error.add_css_class("error");
                changed_creation.borrow_mut().take();
                changed_confirm.set_label(if move_sources {
                    "Move here"
                } else {
                    "Copy here"
                });
            },
        );

        let initial_text = folder_input_path(&base);
        let dirty_field = field.clone();
        let dirty_creating = creating_destination.clone();
        let layer = modal_layer(
            &content,
            &window_overlay,
            blurred_root.clone(),
            Some(Rc::new(move || {
                dirty_creating.get() || dirty_field.text() != initial_text
            })),
        );
        window_overlay.add_overlay(&layer);
        let cancel_layer = layer.clone();
        let cancel_overlay = window_overlay.clone();
        let cancel_root = blurred_root.clone();
        let cancel_creating = creating_destination.clone();
        cancel.connect_clicked(move |_| {
            if !cancel_creating.get() {
                dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
            }
        });
        let close_layer = layer.clone();
        let close_overlay = window_overlay.clone();
        let close_root = blurred_root.clone();
        let close_creating = creating_destination.clone();
        close.connect_clicked(move |_| {
            if !close_creating.get() {
                dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
            }
        });
        let confirm_layer = layer.clone();
        let confirm_overlay = window_overlay.clone();
        let confirm_root = blurred_root.clone();
        let transfer_state = self.clone();
        let confirm_field = field.clone();
        let confirm_error = error.clone();
        let confirm_base = base.clone();
        let confirm_creation = pending_creation;
        let confirm_creating = creating_destination.clone();
        let confirm_cancel = cancel.clone();
        let confirm_close = close.clone();
        let sources = entries
            .iter()
            .map(|entry| entry.location.clone())
            .collect::<Vec<_>>();
        confirm.connect_clicked(move |button| {
            let path =
                resolve_destination_path(&confirm_field.text(), &confirm_base, &glib::home_dir());
            if path.exists() && !path.is_dir() {
                confirm_error.remove_css_class("warning");
                confirm_error.add_css_class("error");
                confirm_error.set_text("The destination exists, but it is not a folder.");
                confirm_error.set_visible(true);
                confirm_field.add_css_class("error");
                confirm_field.grab_focus();
                return;
            }
            if !path.exists() && confirm_creation.borrow().as_ref() != Some(&path) {
                confirm_creation.replace(Some(path.clone()));
                confirm_error.remove_css_class("error");
                confirm_error.add_css_class("warning");
                confirm_error.set_text(&format!(
                    "{} does not exist. It will be created before the items are transferred.",
                    compact_native_path(&path)
                ));
                confirm_error.set_visible(true);
                button.set_label(if move_sources {
                    "Create and move"
                } else {
                    "Create and copy"
                });
                button.grab_focus();
                return;
            }
            if path.is_dir() {
                transfer_state
                    .pending_navigate
                    .replace(Some(Location::local(path.clone())));
                let names: Vec<String> = sources
                    .iter()
                    .filter_map(|s| s.native_path()?.file_name()?.to_str().map(String::from))
                    .collect();
                transfer_state.pending_select.borrow_mut().extend(names);
                transfer_state.start_transfer(Location::local(path), sources.clone(), move_sources);
                dismiss_modal_layer(&confirm_layer, &confirm_overlay, confirm_root.as_ref());
                return;
            }

            confirm_creating.set(true);
            button.set_sensitive(false);
            button.set_label("Creating folder…");
            confirm_field.set_sensitive(false);
            confirm_cancel.set_sensitive(false);
            confirm_close.set_sensitive(false);
            let created_state = transfer_state.clone();
            let created_sources = sources.clone();
            let created_layer = confirm_layer.clone();
            let created_overlay = confirm_overlay.clone();
            let created_root = confirm_root.clone();
            let created_button = button.clone();
            let created_field = confirm_field.clone();
            let created_error = confirm_error.clone();
            let created_creating = confirm_creating.clone();
            let created_cancel = confirm_cancel.clone();
            let created_close = confirm_close.clone();
            glib::MainContext::default().spawn_local(async move {
                let created_path = path.clone();
                let result =
                    gio::spawn_blocking(move || std::fs::create_dir_all(&created_path)).await;
                match result {
                    Ok(Ok(())) => {
                        created_state
                            .pending_navigate
                            .replace(Some(Location::local(path.clone())));
                        let names: Vec<String> = created_sources
                            .iter()
                            .filter_map(|s| {
                                s.native_path()?.file_name()?.to_str().map(String::from)
                            })
                            .collect();
                        created_state.pending_select.borrow_mut().extend(names);
                        created_state.start_transfer(
                            Location::local(path),
                            created_sources,
                            move_sources,
                        );
                        dismiss_modal_layer(
                            &created_layer,
                            &created_overlay,
                            created_root.as_ref(),
                        );
                    }
                    Ok(Err(error)) => {
                        created_creating.set(false);
                        created_cancel.set_sensitive(true);
                        created_close.set_sensitive(true);
                        created_error.remove_css_class("warning");
                        created_error.add_css_class("error");
                        created_error.set_text(&format!("Unable to create that folder: {error}"));
                        created_error.set_visible(true);
                        created_field.add_css_class("error");
                        created_field.set_sensitive(true);
                        created_field.grab_focus();
                        created_button.set_sensitive(true);
                        created_button.set_label(if move_sources {
                            "Move here"
                        } else {
                            "Copy here"
                        });
                    }
                    Err(_) => {
                        created_creating.set(false);
                        created_cancel.set_sensitive(true);
                        created_close.set_sensitive(true);
                        created_error.remove_css_class("warning");
                        created_error.add_css_class("error");
                        created_error.set_text("Unable to create that folder.");
                        created_error.set_visible(true);
                        created_field.add_css_class("error");
                        created_field.set_sensitive(true);
                        created_field.grab_focus();
                        created_button.set_sensitive(true);
                        created_button.set_label(if move_sources {
                            "Move here"
                        } else {
                            "Copy here"
                        });
                    }
                }
            });
        });
        let activate_confirm = confirm.clone();
        field.connect_activate(move |_| activate_confirm.emit_clicked());
        let escape = gtk::EventControllerKey::new();
        let escape_layer = layer.clone();
        let escape_overlay = window_overlay;
        let escape_root = blurred_root;
        let escape_creating = creating_destination;
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if escape_creating.get() {
                    return glib::Propagation::Stop;
                }
                dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);

        field.emit_by_name::<()>("changed", &[]);
        field.grab_focus();
    }
}

#[cfg(test)]
mod tests;
