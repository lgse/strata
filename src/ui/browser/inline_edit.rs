// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::{FileEntry, Location};
use crate::services::validate_basename;
use crate::ui::browser::ViewState;
use crate::ui::browser::paths::is_trash_location;
use crate::ui::browser_modes::BrowserMode;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(super) struct ActiveRename {
    pub(super) entry: FileEntry,
    pub(super) field: gtk::Entry,
    pub(super) label: gtk::Label,
    pub(super) spacer: gtk::Box,
    pub(super) size: gtk::Label,
    viewport_tick: gtk::TickCallbackId,
}

fn constrain_rename_to_viewport(field: &gtk::Entry, viewport: &gtk::ScrolledWindow) {
    let Some(editor) = field.parent() else { return };
    let Some(bounds) = editor.compute_bounds(viewport) else {
        return;
    };
    if bounds.width() <= 0.0 || viewport.width() <= 0 {
        return;
    }
    // Columns can be wider than the viewport. GtkText must scroll within the
    // visible slice, not an allocation clipped by the outer horizontal scroller.
    let start = (-bounds.x()).ceil().max(0.0) as i32;
    let end = (bounds.x() + bounds.width() - viewport.width() as f32)
        .ceil()
        .max(0.0) as i32;
    field.set_margin_start(start.min(editor.width().saturating_sub(1)));
    field.set_margin_end(end.min(editor.width().saturating_sub(start + 1).max(0)));
}

pub(super) struct PendingEntryRename {
    depth: usize,
    parent: Location,
}

pub(super) struct CreatedEntryRename {
    depth: usize,
    parent: Location,
    source: Location,
    target: RefCell<Option<Location>>,
    scroll_positions: RefCell<Option<Vec<(gtk::Adjustment, f64)>>>,
}

// Empty fields are an ordinary editing state, although they cannot be submitted.
fn basename_field_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        None
    } else {
        validate_basename(name).err()
    }
}

pub(in crate::ui) fn update_basename_validation(field: &gtk::Entry) -> bool {
    let text = field.text();
    match basename_field_error(text.as_str()) {
        None => {
            field.remove_css_class("error");
            field.set_tooltip_text(None);
            !text.is_empty()
        }
        Some(message) => {
            field.add_css_class("error");
            field.set_tooltip_text(Some(message));
            false
        }
    }
}

pub(in crate::ui) fn rename_stem_end(name: &str) -> i32 {
    let end = name
        .rfind('.')
        .filter(|position| *position > 0)
        .unwrap_or(name.len());
    name[..end].chars().count().min(i32::MAX as usize) as i32
}

fn capture_scroll_positions(root: &gtk::Widget) -> Vec<(gtk::Adjustment, f64)> {
    let mut positions = Vec::new();
    if let Some(scroller) = root.downcast_ref::<gtk::ScrolledWindow>() {
        positions.push((scroller.hadjustment(), scroller.hadjustment().value()));
        positions.push((scroller.vadjustment(), scroller.vadjustment().value()));
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        positions.extend(capture_scroll_positions(&current));
    }
    positions
}

fn restore_scroll_positions(positions: &[(gtk::Adjustment, f64)]) {
    for (adjustment, value) in positions {
        adjustment.set_value(*value);
    }
}

impl super::BrowserView {
    pub(in crate::ui) fn install_inline_edit_dismissal(&self, root: &impl IsA<gtk::Widget>) {
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.state);
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(state) = weak.upgrade() else { return };
            state.pending_new_entry.take();
            let target = gesture
                .widget()
                .and_then(|root| root.pick(x, y, gtk::PickFlags::DEFAULT));
            let field = state
                .active_rename
                .borrow()
                .as_ref()
                .map(|active| active.field.clone())
                .or_else(|| state.mode_views.borrow().active_rename_field());
            if let Some(field) = field.as_ref()
                && !target
                    .as_ref()
                    .is_some_and(|target| target == field || target.is_ancestor(field))
            {
                state.submit_rename(field);
            } else if field.is_none() {
                state.clear_created_entry_rename();
            }
        });
        root.add_controller(click);
    }
}

pub(in crate::ui) fn queue_rename(
    browser: &Rc<crate::app::Browser>,
    entry: FileEntry,
    name: String,
) {
    if name == entry.display_name || validate_basename(&name).is_err() {
        return;
    }
    // A rename can synchronously refresh models; dispatch after GTK's focus walk.
    let browser = Rc::downgrade(browser);
    gtk::glib::idle_add_local_once(move || {
        if let Some(browser) = browser.upgrade() {
            browser.rename(entry, name);
        }
    });
}

impl ViewState {
    pub(super) fn begin_new_entry(
        self: &Rc<Self>,
        depth: usize,
        location: Location,
        is_directory: bool,
    ) {
        if is_trash_location(&location) {
            return;
        }
        self.cancel_new_entry();
        self.cancel_rename();
        if let Some(column) = self.columns.borrow().get(depth) {
            column.filter_entry.set_text("");
        }
        self.mode_views.borrow().clear_filter(depth);
        self.pending_new_entry
            .replace(Some(Rc::new(PendingEntryRename {
                depth,
                parent: location.clone(),
            })));
        if is_directory {
            self.browser.create_new_folder(location);
        } else {
            self.browser.create_new_file(location);
        }
    }

    pub(super) fn cancel_new_entry(&self) -> bool {
        self.pending_new_entry.take().is_some()
    }

    pub(super) fn begin_rename(self: &Rc<Self>) -> bool {
        self.cancel_new_entry();
        self.cancel_rename();
        self.sync_mode_selection();
        let Some((depth, source_position, entry)) = self.browser.rename_item() else {
            return false;
        };
        self.begin_rename_item(depth, source_position, entry)
    }

    fn begin_rename_item(
        self: &Rc<Self>,
        depth: usize,
        source_position: usize,
        entry: FileEntry,
    ) -> bool {
        if is_trash_location(&entry.location) {
            return false;
        }
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return self
                .mode_views
                .borrow()
                .begin_rename(depth, source_position, &entry);
        }
        self.cancel_rename();
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return false;
        };
        let Some(filtered_position) = column.map.view_position(source_position) else {
            return false;
        };
        let row = column.bound_rows.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == filtered_position).then(|| bound.row.upgrade())?
        });
        let Some(row) = row else { return false };
        if !row.is_mapped() || row.width() <= 0 || column.presentation.stack.is_transition_running()
        {
            return false;
        }
        let Some(icon) = row.first_child() else {
            return false;
        };
        let Some(middle) = icon.next_sibling().and_downcast::<gtk::Overlay>() else {
            return false;
        };
        let Some(editor) = middle
            .child()
            .and_then(|content| content.first_child())
            .and_downcast::<gtk::Box>()
        else {
            return false;
        };
        let Some(label) = editor.first_child().and_downcast::<gtk::Label>() else {
            return false;
        };
        let Some(field) = label.next_sibling().and_downcast::<gtk::Entry>() else {
            return false;
        };
        let Some(spacer) = field.next_sibling().and_downcast::<gtk::Box>() else {
            return false;
        };
        let Some(size) = middle.last_child().and_downcast::<gtk::Label>() else {
            return false;
        };
        super::prepare_collection_inline_edit(column.list.upcast_ref(), filtered_position);
        field.remove_css_class("error");
        field.set_tooltip_text(None);
        field.set_sensitive(true);
        field.set_text(&entry.display_name);
        label.set_visible(false);
        spacer.set_visible(false);
        size.set_visible(false);
        field.set_visible(true);
        constrain_rename_to_viewport(&field, &self.scroller);
        let viewport = self.scroller.downgrade();
        let viewport_tick = field.add_tick_callback(move |field, _| {
            let Some(viewport) = viewport.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            constrain_rename_to_viewport(field, &viewport);
            gtk::glib::ControlFlow::Continue
        });
        field.grab_focus();
        field.select_region(
            0,
            if entry.is_directory() {
                -1
            } else {
                rename_stem_end(&entry.display_name)
            },
        );
        self.active_rename.replace(Some(ActiveRename {
            entry,
            field,
            label,
            spacer,
            size,
            viewport_tick,
        }));
        true
    }

    pub(super) fn cancel_rename(&self) -> bool {
        self.pending_created_rename.take();
        self.suppress_created_focus_scroll.set(false);
        self.mode_views.borrow().clear_focus_scroll_suppression();
        if self.mode_views.borrow().cancel_rename() {
            return true;
        }
        let Some(rename) = self.active_rename.take() else {
            return false;
        };
        rename.viewport_tick.remove();
        rename.field.set_margin_start(0);
        rename.field.set_margin_end(0);
        rename.field.remove_css_class("error");
        rename.field.set_tooltip_text(None);
        rename.field.set_visible(false);
        rename.field.set_sensitive(true);
        rename.label.set_visible(true);
        rename.spacer.set_visible(true);
        rename.size.set_visible(!rename.size.label().is_empty());
        true
    }

    pub(in crate::ui) fn submit_rename(self: &Rc<Self>, field: &gtk::Entry) {
        let mode_field = self.mode_views.borrow().active_rename_field();
        let entry = if mode_field.as_ref() == Some(field) {
            self.mode_views.borrow().active_rename_entry()
        } else {
            self.active_rename
                .borrow()
                .as_ref()
                .filter(|active| active.field == *field)
                .map(|active| active.entry.clone())
        };
        let Some(entry) = entry else { return };
        let name = field.text().to_string();
        let created = self.created_entry_rename_for_submission(&entry, &name);
        if mode_field.as_ref() == Some(field) {
            if let Some(created) = created {
                self.pending_created_rename.replace(Some(created));
                self.suppress_created_entry_scroll();
            }
            self.mode_views.borrow().submit_rename(field);
            return;
        }
        self.cancel_rename();
        if let Some(created) = created {
            self.pending_created_rename.replace(Some(created));
            self.suppress_created_entry_scroll();
        }
        queue_rename(&self.browser, entry, name);
    }

    fn created_entry_rename_for_submission(
        &self,
        entry: &FileEntry,
        name: &str,
    ) -> Option<Rc<CreatedEntryRename>> {
        let pending = self.pending_created_rename.borrow().clone()?;
        if pending.source != entry.location {
            self.pending_created_rename.take();
            return None;
        }
        if name == entry.display_name || validate_basename(name).is_err() {
            self.pending_created_rename.take();
            return None;
        }
        let target = pending
            .parent
            .child(std::ffi::OsStr::new(name))
            .unwrap_or_else(|| entry.location.clone());
        pending.target.replace(Some(target));
        self.pending_created_rename.take();
        Some(pending)
    }

    pub(super) fn rename_created_entry(self: &Rc<Self>, location: &Location) {
        let Some(pending) = self
            .pending_new_entry
            .borrow()
            .clone()
            .filter(|pending| Some(pending.parent.clone()) == location.parent())
        else {
            return;
        };
        let location = location.clone();
        let weak = Rc::downgrade(self);
        let deadline = Instant::now() + Duration::from_secs(5);
        let selected = std::cell::Cell::new(false);
        // Wait for the refreshed listing and the virtualized row to be allocated.
        self.overlay.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if !state
                .pending_new_entry
                .borrow()
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &pending))
            {
                return gtk::glib::ControlFlow::Break;
            }
            if Instant::now() >= deadline
                || state.browser.location_at(pending.depth).as_ref() != Some(&pending.parent)
            {
                state.pending_new_entry.take();
                return gtk::glib::ControlFlow::Break;
            }
            if !state.browser.column_is_settled(pending.depth) {
                return gtk::glib::ControlFlow::Continue;
            }
            let Some(snapshot) = state.browser.column_snapshot(pending.depth) else {
                return gtk::glib::ControlFlow::Continue;
            };
            let position = state
                .browser
                .with_entries(pending.depth, 0..snapshot.count, |entries| {
                    entries.iter().position(|entry| entry.location == location)
                })
                .flatten();
            if let Some(position) = position {
                if !selected.replace(true) {
                    state.browser.select(pending.depth, position);
                }
                if !state.created_entry_is_visible(pending.depth, position) {
                    // GTK can replace a scroll request while allocating a refreshed row.
                    state.reveal_created_entry(pending.depth, position);
                    return gtk::glib::ControlFlow::Continue;
                }
                if let Some(entry) = state.browser.entry_at(pending.depth, position) {
                    if !state.begin_rename_item(pending.depth, position, entry) {
                        return gtk::glib::ControlFlow::Continue;
                    }

                    state
                        .pending_created_rename
                        .replace(Some(Rc::new(CreatedEntryRename {
                            depth: pending.depth,
                            parent: pending.parent.clone(),
                            source: location.clone(),
                            target: RefCell::new(None),
                            scroll_positions: RefCell::new(None),
                        })));
                    state.pending_new_entry.take();
                    return gtk::glib::ControlFlow::Break;
                }
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    pub(super) fn suppress_created_entry_scroll(&self) {
        self.suppress_created_focus_scroll.set(true);
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            self.mode_views.borrow().suppress_focus_scroll();
        }
    }

    pub(super) fn preserve_created_entry_scroll(&self, location: &Location) -> bool {
        let Some(pending) = self
            .pending_created_rename
            .borrow()
            .clone()
            .filter(|pending| pending.target.borrow().as_ref() == Some(location))
        else {
            return false;
        };
        pending
            .scroll_positions
            .replace(Some(capture_scroll_positions(self.overlay.upcast_ref())));
        true
    }

    pub(super) fn finish_created_entry_rename(self: &Rc<Self>, location: &Location) {
        let Some(pending) = self
            .pending_created_rename
            .borrow()
            .clone()
            .filter(|pending| pending.target.borrow().as_ref() == Some(location))
        else {
            return;
        };
        let location = location.clone();
        let weak = Rc::downgrade(self);
        let deadline = Instant::now() + Duration::from_secs(5);
        let restored_scroll = std::cell::Cell::new(false);
        let selection_applied = std::cell::Cell::new(false);
        let focus_requested = std::cell::Cell::new(false);
        self.overlay.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if !state
                .pending_created_rename
                .borrow()
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &pending))
            {
                return gtk::glib::ControlFlow::Break;
            }
            if Instant::now() >= deadline
                || state.browser.location_at(pending.depth).as_ref() != Some(&pending.parent)
            {
                state.clear_created_entry_rename();
                return gtk::glib::ControlFlow::Break;
            }
            let Some(snapshot) = state.browser.column_snapshot(pending.depth) else {
                state.clear_created_entry_rename();
                return gtk::glib::ControlFlow::Break;
            };
            if snapshot.error.is_some() {
                state.clear_created_entry_rename();
                return gtk::glib::ControlFlow::Break;
            }
            if !state.browser.column_is_settled(pending.depth) {
                return gtk::glib::ControlFlow::Continue;
            }
            let position = state
                .browser
                .with_entries(pending.depth, 0..snapshot.count, |entries| {
                    entries.iter().position(|entry| entry.location == location)
                })
                .flatten();
            if let Some(position) = position {
                if !restored_scroll.replace(true) {
                    if let Some(scroll_positions) = pending.scroll_positions.borrow().as_ref() {
                        restore_scroll_positions(scroll_positions);
                    }
                    return gtk::glib::ControlFlow::Continue;
                }
                let visible = state.created_entry_is_visible(pending.depth, position);
                if !visible {
                    state.suppress_created_focus_scroll.set(false);
                    state.mode_views.borrow().clear_focus_scroll_suppression();
                    state.reveal_created_entry(pending.depth, position);
                    return gtk::glib::ControlFlow::Continue;
                }
                if !selection_applied.replace(true) {
                    state.suppress_created_entry_scroll();
                    state.browser.select(pending.depth, position);
                    state.focus_created_entry_view(pending.depth);
                    return gtk::glib::ControlFlow::Continue;
                }
                if !focus_requested.replace(true) {
                    state.focus_created_entry_view(pending.depth);
                    return gtk::glib::ControlFlow::Continue;
                }
                let focused = state
                    .browser
                    .focused_item()
                    .is_some_and(|(depth, _, entry)| {
                        depth == pending.depth && entry.location == location
                    })
                    && state.item_view_has_focus();
                if focused && state.created_entry_is_visible(pending.depth, position) {
                    state.clear_created_entry_rename();
                    return gtk::glib::ControlFlow::Break;
                }
                if !focused {
                    state.focus_created_entry_view(pending.depth);
                }
                return gtk::glib::ControlFlow::Continue;
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    fn focus_created_entry_view(&self, depth: usize) {
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            if let Some(column) = self.columns.borrow().get(depth)
                && !column.list.grab_focus()
            {
                column.presentation.stack.grab_focus();
            }
        } else {
            self.mode_views.borrow().focus_visible_pane(depth);
        }
    }

    fn item_view_has_focus(&self) -> bool {
        let focused = self.overlay.root().and_then(|root| root.focus());
        self.mode_views.borrow().item_view_has_focus()
            || self.columns.borrow().iter().any(|column| {
                focused.as_ref().is_some_and(|focused| {
                    focused == column.presentation.stack.upcast_ref::<gtk::Widget>()
                        || focused == column.list.upcast_ref::<gtk::Widget>()
                        || column.list.is_ancestor(focused)
                        || focused.is_ancestor(&column.list)
                })
            })
    }

    fn reveal_created_entry(&self, depth: usize, source_position: usize) {
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            if let Some(column) = self.columns.borrow().get(depth)
                && let Some(position) = column.map.view_position(source_position)
            {
                super::columns::scroll_column_into_view(column, position);
            }
        } else {
            self.mode_views.borrow().reveal_item(depth, source_position);
        }
    }

    fn created_entry_is_visible(&self, depth: usize, source_position: usize) -> bool {
        match self.mode_views.borrow().mode() {
            BrowserMode::Columns => {
                let columns = self.columns.borrow();
                let Some(column) = columns.get(depth) else {
                    return false;
                };
                let Some(row) = column.bound_rows.borrow().iter().find_map(|bound| {
                    let item = bound.item.upgrade()?;
                    (column.map.source_position(item.position()) == Some(source_position))
                        .then(|| bound.row.upgrade())
                        .flatten()
                }) else {
                    return false;
                };
                let Some(bounds) = row.compute_bounds(&column.list) else {
                    return false;
                };
                row.is_mapped()
                    && bounds.y() >= 0.0
                    && bounds.y() + bounds.height() <= column.list.height() as f32
            }
            BrowserMode::Icons | BrowserMode::List => self
                .mode_views
                .borrow()
                .bound_item_is_visible(depth, source_position)
                .unwrap_or(false),
        }
    }

    pub(super) fn clear_created_entry_rename(&self) {
        self.pending_created_rename.take();
        self.suppress_created_focus_scroll.set(false);
        self.mode_views.borrow().clear_focus_scroll_suppression();
    }
}

#[cfg(test)]
mod tests;
