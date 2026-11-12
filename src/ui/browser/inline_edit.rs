// SPDX-License-Identifier: MIT

use crate::model::{FileEntry, Location};
use crate::services::validate_basename;
use crate::ui::browser::ViewState;
use crate::ui::browser::paths::is_trash_location;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::collection_edit::Naming;
use gtk::prelude::*;
use std::rc::Rc;

mod new_entry;
mod pending;
mod reveal;
pub(super) use new_entry::PendingEntryRename;
pub(super) use pending::PendingRename;
pub(in crate::ui) use reveal::reveal_rename_row;

struct ColumnsRenameTarget {
    row: gtk::Box,
    edit: crate::ui::collection_edit::EditWidgets,
    spacer: gtk::Box,
    size: gtk::Label,
    scroll: gtk::ScrolledWindow,
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

fn is_within(target: &gtk::Widget, field: &gtk::Entry) -> bool {
    target == field.upcast_ref::<gtk::Widget>() || target.is_ancestor(field)
}

pub(in crate::ui) fn set_rename_label(label: &gtk::Widget, name: &str) {
    if let Some(label) = label.downcast_ref::<gtk::Inscription>() {
        label.set_text(Some(name));
    } else if let Some(label) = label.downcast_ref::<gtk::Label>() {
        label.set_label(name);
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
            state.cancel_new_entry();
            let target = gesture
                .widget()
                .and_then(|root| root.pick(x, y, gtk::PickFlags::DEFAULT));
            if let Some(field) = state.active_rename_field()
                && !target
                    .as_ref()
                    .is_some_and(|target| is_within(target, &field))
            {
                state.submit_rename(&field);
            }
            state.yield_rename_reveal();
        });
        root.add_controller(click);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.state);
        scroll.connect_scroll(move |_, _, _| {
            if let Some(state) = weak.upgrade() {
                state.cancel_new_entry();
                state.yield_rename_reveal();
            }
            gtk::glib::Propagation::Proceed
        });
        root.add_controller(scroll);
    }
}

/// The native paths of a rename, `to` being `None` for a move to another folder.
fn native_rename<'a>(
    old_location: &'a Location,
    new_location: Option<&'a Location>,
) -> Option<(&'a std::path::Path, Option<&'a std::path::Path>)> {
    let to = match new_location {
        Some(location) => Some(location.native_path()?),
        None => None,
    };
    Some((old_location.native_path()?, to))
}

fn names_location(
    location: &Location,
    old_location: &Location,
    new_location: Option<&Location>,
) -> bool {
    location == old_location || new_location.is_some_and(|new_location| new_location == location)
}

pub(in crate::ui) fn queue_rename(
    browser: &Rc<crate::app::Browser>,
    entry: FileEntry,
    name: String,
) {
    if name == entry.display_name || validate_basename(&name).is_err() {
        browser.clear_group_folder();
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
    pub(in crate::ui) fn rename_label_widgets(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
    ) -> Vec<gtk::Widget> {
        let mut labels = Vec::new();
        {
            let columns = self.columns.borrow();
            for (depth, column) in columns.iter().enumerate() {
                column.bound_rows.borrow_mut().retain(|bound| {
                    let (Some(item), Some(_row)) = (bound.item.upgrade(), bound.row.upgrade())
                    else {
                        return false;
                    };
                    let Some(location) = self.column_row_location(depth, column, item.position())
                    else {
                        return true;
                    };
                    if names_location(&location, old_location, new_location)
                        && let Some(label) = bound.rename_label.upgrade()
                    {
                        labels.push(label.upcast());
                    }
                    true
                });
            }
        }
        labels.extend(
            self.mode_views
                .borrow()
                .rename_label_widgets(old_location, new_location),
        );
        labels
    }

    /// The item a Columns row at `position` shows. Filter results replace the
    /// listing's rows, so there a row's position indexes the results, not the directory.
    fn column_row_location(
        &self,
        depth: usize,
        column: &super::columns::ColumnView,
        position: u32,
    ) -> Option<Location> {
        if column.recursive_search_active.get() {
            return column
                .search_results
                .borrow()
                .get(position as usize)
                .map(|result| Location::local(&result.path));
        }
        let position = column.map.source_position(position)?;
        Some(self.browser.entry_at(depth, position)?.location)
    }

    fn update_rename_labels(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
        name: &str,
    ) {
        for label in self.rename_label_widgets(old_location, new_location) {
            set_rename_label(&label, name);
        }
    }

    pub(in crate::ui) fn schedule_click_rename(
        self: &Rc<Self>,
        depth: usize,
        source_position: usize,
    ) {
        self.cancel_click_rename();
        let Some(entry) = self.browser.entry_at(depth, source_position) else {
            return;
        };
        let generation = self.click_rename_generation.get() + 1;
        self.click_rename_generation.set(generation);
        let interval = self.scroller.settings().gtk_double_click_time().max(1) as u64;
        let weak = Rc::downgrade(self);
        let id = gtk::glib::timeout_add_local_once(
            std::time::Duration::from_millis(interval),
            move || {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                if state.click_rename_generation.get() != generation {
                    return;
                }
                state.pending_click_rename.take();
                if state.click_rename_still_wanted((depth, source_position), &entry) {
                    state.begin_rename();
                }
            },
        );
        self.pending_click_rename.replace(Some(id));
    }

    fn editing_elsewhere(&self) -> bool {
        self.rename_operation_pending()
            || self.active_rename.borrow().is_some()
            || self
                .scroller
                .root()
                .and_then(|root| root.focus())
                .as_ref()
                .is_some_and(crate::ui::focus_navigation::editable)
    }

    /// A delayed click rename only opens on the same, solely selected and focused item.
    fn click_rename_still_wanted(
        &self,
        (depth, source_position): (usize, usize),
        entry: &FileEntry,
    ) -> bool {
        if self.editing_elsewhere() || self.browser.selected_entries().len() != 1 {
            return false;
        }
        self.browser
            .focused_item()
            .is_some_and(|(current_depth, position, current)| {
                (current_depth, position) == (depth, source_position)
                    && current.location == entry.location
            })
    }

    pub(in crate::ui) fn cancel_click_rename(&self) {
        if let Some(id) = self.pending_click_rename.take() {
            id.remove();
        }
        self.click_rename_generation
            .set(self.click_rename_generation.get() + 1);
    }

    pub(in crate::ui) fn begin_entry_rename(
        self: &Rc<Self>,
        depth: usize,
        entry: &FileEntry,
    ) -> bool {
        if self.begin_search_result_rename(depth, entry) {
            return true;
        }
        self.sync_mode_selection();
        if !self
            .browser
            .rename_item()
            .is_some_and(|(_, _, selected)| selected.location == entry.location)
        {
            return false;
        }
        self.begin_rename()
    }

    pub(in crate::ui) fn begin_search_result_rename(
        self: &Rc<Self>,
        depth: usize,
        entry: &FileEntry,
    ) -> bool {
        self.cancel_click_rename();
        if self.rename_operation_pending()
            || is_trash_location(&entry.location)
            || entry.is_computer_item()
        {
            return false;
        }
        self.cancel_new_entry();
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return self.mode_views.borrow().begin_search_rename(depth, entry);
        }
        let Some(position) = self.search_result_view_position(depth, entry) else {
            return false;
        };
        self.cancel_rename();
        let Some(target) = self.resolve_columns_rename_target_at_view_position(depth, position)
        else {
            return false;
        };
        self.activate_columns_rename(target, entry.clone(), Naming::Path);
        true
    }

    fn search_result_view_position(&self, depth: usize, entry: &FileEntry) -> Option<u32> {
        let path = entry.location.native_path()?;
        let columns = self.columns.borrow();
        let column = columns.get(depth)?;
        if !column.recursive_search_active.get() {
            return None;
        }
        let position = column
            .search_results
            .borrow()
            .iter()
            .position(|item| item.path == path)?;
        u32::try_from(position).ok()
    }

    pub(super) fn begin_rename(self: &Rc<Self>) -> bool {
        self.cancel_click_rename();
        if self.rename_operation_pending() {
            return false;
        }
        self.sync_mode_selection();
        let Some((depth, source_position, entry)) = self.browser.rename_item() else {
            self.cancel_new_entry();
            return false;
        };
        let taking_over = self
            .pending_new_entry
            .borrow()
            .as_ref()
            .filter(|pending| pending.created.borrow().as_ref() == Some(&entry.location))
            .map(|pending| pending.naming);
        if taking_over.is_none() {
            self.cancel_new_entry();
        }
        let naming = taking_over.unwrap_or_default();
        let started = self.begin_rename_item((depth, source_position), entry, naming);
        if started && taking_over.is_some() {
            self.pending_new_entry.take();
        }
        started
    }

    fn begin_rename_item(
        self: &Rc<Self>,
        (depth, source_position): (usize, usize),
        entry: FileEntry,
        naming: Naming,
    ) -> bool {
        if is_trash_location(&entry.location) || entry.is_computer_item() {
            return false;
        }
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return self
                .mode_views
                .borrow()
                .begin_rename(depth, source_position, &entry, naming);
        }
        self.cancel_rename();
        let Some(target) = self.resolve_columns_rename_target(depth, source_position) else {
            return false;
        };
        self.activate_columns_rename(target, entry, naming);
        true
    }

    fn resolve_columns_rename_target(
        &self,
        depth: usize,
        source_position: usize,
    ) -> Option<ColumnsRenameTarget> {
        let filtered_position = self
            .columns
            .borrow()
            .get(depth)?
            .map
            .view_position(source_position)?;
        self.resolve_columns_rename_target_at_view_position(depth, filtered_position)
    }

    fn resolve_columns_rename_target_at_view_position(
        &self,
        depth: usize,
        filtered_position: u32,
    ) -> Option<ColumnsRenameTarget> {
        let columns = self.columns.borrow();
        let column = columns.get(depth)?;
        // Prepare before checking allocation: it cancels deferred scrolling and lets GTK bind
        // the row needed by the editor.
        super::prepare_collection_inline_edit(column.list.upcast_ref(), filtered_position);
        let (row, bound) = column.shown_row_at(filtered_position)?;
        if row.width() <= 0 || column.presentation.stack.is_transition_running() {
            return None;
        }
        Some(ColumnsRenameTarget {
            row,
            edit: bound.edit,
            spacer: bound.spacer,
            size: bound.size,
            scroll: column.listing_scroll.clone(),
        })
    }

    fn activate_columns_rename(
        self: &Rc<Self>,
        target: ColumnsRenameTarget,
        entry: FileEntry,
        naming: Naming,
    ) {
        let ColumnsRenameTarget {
            row,
            edit,
            spacer,
            size,
            scroll,
        } = target;
        spacer.set_visible(false);
        size.set_visible(false);
        constrain_rename_to_viewport(&edit.field, &self.scroller);
        let viewport = self.scroller.downgrade();
        let row = row.downgrade();
        let scroll = scroll.downgrade();
        let weak = Rc::downgrade(self);
        let reveal_generation = self.rename_reveal_generation.get();
        let mut target = crate::ui::collection_edit::EditTarget::from(edit.clone());
        target.naming = naming;
        let browser = Rc::downgrade(&self.browser);
        let location = entry.location.clone();
        target.cancelled = Some(Rc::new(move || {
            if let Some(browser) = browser.upgrade() {
                browser.clear_group_folder_for(&location);
            }
        }));
        target.reveal = Some(Rc::new(move |field| {
            if let Some(viewport) = viewport.upgrade() {
                constrain_rename_to_viewport(field, &viewport);
            }
            if !weak
                .upgrade()
                .is_some_and(|state| state.rename_reveal_generation.get() == reveal_generation)
            {
                return;
            }
            if let (Some(row), Some(scroll)) = (row.upgrade(), scroll.upgrade()) {
                reveal_rename_row(&row, &scroll, None);
            }
        }));
        let field = edit.field.downgrade();
        target.finish = Some(Rc::new(move || {
            if let Some(field) = field.upgrade() {
                field.set_margin_start(0);
                field.set_margin_end(0);
            }
            spacer.set_visible(true);
            size.set_visible(
                crate::ui::preferences::PreferenceManager::shared().columns_show_sizes()
                    && !size.label().is_empty(),
            );
        }));
        let state = Rc::downgrade(self);
        crate::ui::collection_edit::begin(
            &self.active_rename,
            entry,
            target,
            Rc::new(move |field| {
                if let Some(state) = state.upgrade() {
                    state.submit_rename(field);
                }
            }),
        );
    }

    fn active_rename_field(&self) -> Option<gtk::Entry> {
        self.active_rename
            .borrow()
            .as_ref()
            .map(|active| active.field.clone())
            .or_else(|| self.mode_views.borrow().active_rename_field())
    }

    pub(in crate::ui) fn cancel_group_naming(&self, location: &Location) {
        self.browser.clear_group_folder_for(location);
    }

    pub(super) fn cancel_rename(&self) -> bool {
        self.cancel_click_rename();
        let mode_rename = self.mode_views.borrow().take_rename();
        let cancelled = mode_rename.is_some();
        drop(mode_rename);
        let cancelled = crate::ui::collection_edit::cancel(&self.active_rename) || cancelled;
        if cancelled {
            self.browser.clear_group_folder();
        }
        cancelled
    }

    /// Tab in a rename: commits the name, then opens the editor on the next item
    /// (`direction` 1) or the previous one (-1). At the listing's edge it only commits.
    pub(in crate::ui) fn advance_rename(self: &Rc<Self>, direction: i32) -> bool {
        let Some(field) = self.active_rename_field() else {
            return false;
        };
        let before = self
            .browser
            .rename_item()
            .map(|(depth, position, _)| (depth, position));
        let generation = self.rename_generation.get();
        self.submit_rename(&field);
        self.note_rename_advanced(generation);
        // The committed row must not pull the cursor back once its rename lands.
        self.yield_rename_reveal();
        self.browser.move_selection(direction);
        if let Some((depth, position, entry)) = self.browser.rename_item()
            && before != Some((depth, position))
        {
            self.mirror_rename_target();
            self.rename_when_shown(depth, position, entry);
        }
        true
    }

    fn rename_when_shown(self: &Rc<Self>, depth: usize, position: usize, entry: FileEntry) {
        if self.begin_rename_item((depth, position), entry.clone(), Naming::Path) {
            return;
        }
        let weak = Rc::downgrade(self);
        let reveal_generation = self.rename_reveal_generation.get();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        // The neighbouring row is allocated a frame after the scroll that showed it.
        self.overlay.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            let current = state.browser.rename_item().is_some_and(
                |(current_depth, current_position, current)| {
                    (current_depth, current_position) == (depth, position)
                        && current.location == entry.location
                },
            );
            let expired = std::time::Instant::now() >= deadline
                || state.rename_reveal_generation.get() != reveal_generation;
            if !current || expired {
                return gtk::glib::ControlFlow::Break;
            }
            if state.begin_rename_item((depth, position), entry.clone(), Naming::Path) {
                gtk::glib::ControlFlow::Break
            } else {
                gtk::glib::ControlFlow::Continue
            }
        });
    }

    fn submit_rename_entry(self: &Rc<Self>, submission: crate::ui::collection_edit::Submission) {
        let path = submission.path();
        let new_item = submission.naming == Naming::NewItem;
        let crate::ui::collection_edit::Submission { entry, name, .. } = submission;
        let name = match path {
            Some(path) if !path.is_rename_of(entry.is_directory()) => {
                let first = path.first().to_owned();
                let replacement = entry
                    .location
                    .parent()
                    .and_then(|parent| path.start(&parent))
                    .and_then(|start| start.child(std::ffi::OsStr::new(&first)));
                let generation =
                    self.start_pending_rename(&entry, first.clone(), Some(replacement));
                self.follow_pending_rename();
                // A row leaving for a folder above keeps its name until it goes.
                if path.up == 0 {
                    self.update_rename_labels(&entry.location, None, &first);
                }
                self.queue_pending_rename(generation, move |browser| {
                    browser.place_entry(entry, path, new_item)
                });
                return;
            }
            // A folder typed with a trailing slash is still just named.
            Some(path) => path.name,
            None => name,
        };
        let valid_change = name != entry.display_name && validate_basename(&name).is_ok();
        if !valid_change {
            self.browser.clear_group_folder();
            return;
        }
        let generation = self.start_pending_rename(&entry, name.clone(), None);
        self.follow_pending_rename();
        self.update_rename_labels(&entry.location, None, &name);
        self.queue_pending_rename(generation, move |browser| {
            if new_item {
                browser.name_new_item(entry, name)
            } else {
                browser.rename(entry, name)
            }
        });
    }

    /// Filter results keep their selection and focus by path. Tells those listing
    /// `old_location` that it becomes `new_location`, which they list only once their
    /// search index catches up, or with `None` that it moves to another folder.
    pub(super) fn follow_renamed_result(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
    ) {
        let Some((from, to)) = native_rename(old_location, new_location) else {
            return;
        };
        for column in self.columns.borrow().iter() {
            let listed = || {
                column
                    .search_results
                    .borrow()
                    .iter()
                    .any(|result| result.path == from)
            };
            if column.recursive_search_active.get() && listed() {
                column.renamed_result.expect(from, to);
            }
        }
        self.mode_views.borrow().follow_renamed_result(from, to);
    }

    /// Undoes [`Self::follow_renamed_result`] for this rename only.
    pub(super) fn forget_renamed_result(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
    ) {
        let Some((from, to)) = native_rename(old_location, new_location) else {
            return;
        };
        for column in self.columns.borrow().iter() {
            column.renamed_result.clear_if(from, to);
        }
        self.mode_views.borrow().forget_renamed_result(from, to);
    }

    pub(in crate::ui) fn submit_mode_rename(self: &Rc<Self>, field: &gtk::Entry) {
        let active_rename = self
            .mode_views
            .try_borrow_mut()
            .ok()
            .and_then(|mode_views| mode_views.take_active_rename(field));
        let Some(submission) = active_rename else {
            return;
        };
        self.submit_rename_entry(submission);
    }

    pub(super) fn submit_rename(self: &Rc<Self>, field: &gtk::Entry) {
        let mode_field_active = self
            .mode_views
            .try_borrow()
            .ok()
            .and_then(|mode_views| mode_views.active_rename_field())
            .as_ref()
            == Some(field);
        if mode_field_active {
            self.submit_mode_rename(field);
            return;
        }
        let Some(submission) =
            crate::ui::collection_edit::take_submission(&self.active_rename, field)
        else {
            return;
        };
        self.submit_rename_entry(submission);
    }
}
