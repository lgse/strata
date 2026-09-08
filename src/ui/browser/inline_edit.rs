// SPDX-License-Identifier: MIT

use crate::model::{FileEntry, Location};
use crate::services::validate_basename;
use crate::ui::browser::ViewState;
use crate::ui::browser::paths::is_trash_location;
use crate::ui::browser_modes::BrowserMode;
use gtk::prelude::*;
use std::rc::Rc;

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
            if let Some(field) = field
                && !target
                    .as_ref()
                    .is_some_and(|target| target == &field || target.is_ancestor(&field))
            {
                state.submit_rename(&field);
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
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
            if std::time::Instant::now() >= deadline
                || state.browser.location_at(pending.depth).as_ref() != Some(&pending.parent)
            {
                state.pending_new_entry.take();
                return gtk::glib::ControlFlow::Break;
            }
            let Some(snapshot) = state
                .browser
                .column_snapshot(pending.depth)
                .filter(|snapshot| !snapshot.loading)
            else {
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
                } else if let Some(entry) = state.browser.entry_at(pending.depth, position)
                    && state.begin_rename_item(pending.depth, position, entry)
                {
                    state.pending_new_entry.take();
                    return gtk::glib::ControlFlow::Break;
                }
            }
            gtk::glib::ControlFlow::Continue
        });
    }

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
        // GTK 4.14 can bind an inserted row without allocating it after the first
        // scroll request. Let the creation tick reveal it before requiring a field.
        super::prepare_collection_inline_edit(column.list.upcast_ref(), filtered_position);
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

    pub(super) fn submit_rename(self: &Rc<Self>, field: &gtk::Entry) {
        if self.mode_views.borrow().active_rename_field().as_ref() == Some(field) {
            self.mode_views.borrow().submit_rename(field);
            return;
        }
        let entry = self
            .active_rename
            .borrow()
            .as_ref()
            .filter(|active| active.field == *field)
            .map(|active| active.entry.clone());
        let Some(entry) = entry else { return };
        let name = field.text().to_string();
        self.cancel_rename();
        queue_rename(&self.browser, entry, name);
    }
}

#[cfg(test)]
mod tests;
