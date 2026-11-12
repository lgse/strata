// SPDX-License-Identifier: MIT

//! Renaming an item in place in an Icons or List pane.

use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};

use super::{
    BrowserMode, ModeViews,
    pane::{Pane, PaneSection},
    source_index::view_position_for_source,
};
use crate::{
    app::Browser,
    model::{FileEntry, Location},
    ui::{
        browser::{self, ViewState},
        collection_edit::{self, ActiveEdit, EditTarget, EditWidgets, Naming, Reveal, Submission},
    },
};

/// The rename field and label of a rendered item, its widget, and its collection view.
type RenameTarget = (EditWidgets, gtk::Widget, gtk::Widget);

/// The entry a rename starts or ends on, before and after it moves.
struct RenamedLocations<'a> {
    old: &'a Location,
    new: Option<&'a Location>,
}

impl RenamedLocations<'_> {
    fn matches(&self, location: &Location) -> bool {
        location == self.old || self.new.is_some_and(|new| new == location)
    }
}

impl ModeViews {
    pub fn rename_is_active(&self) -> bool {
        self.active_rename.borrow().is_some()
    }

    pub(in crate::ui) fn active_rename_field(&self) -> Option<gtk::Entry> {
        self.active_rename
            .borrow()
            .as_ref()
            .map(|rename| rename.field.clone())
    }

    pub(in crate::ui) fn take_active_rename(&self, field: &gtk::Entry) -> Option<Submission> {
        collection_edit::take_submission(&self.active_rename, field)
    }

    pub(in crate::ui) fn take_rename(&self) -> Option<ActiveEdit> {
        self.active_rename.take()
    }

    pub(in crate::ui) fn rename_label_widgets(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
    ) -> Vec<gtk::Widget> {
        let renamed = RenamedLocations {
            old: old_location,
            new: new_location,
        };
        let mut labels = Vec::new();
        for pane in self.all_panes() {
            labels.extend(pane.search.rename_label_widgets(old_location, new_location));
            for section in pane.item_sections() {
                self.collect_rename_labels(pane, &section, &renamed, &mut labels);
            }
        }
        labels
    }

    fn collect_rename_labels(
        &self,
        pane: &Pane,
        section: &PaneSection,
        renamed: &RenamedLocations,
        labels: &mut Vec<gtk::Widget>,
    ) {
        section.bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(_widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            let Some(entry) = pane.bound_entry(&self.browser, &item) else {
                return true;
            };
            if renamed.matches(&entry.location)
                && let Some(label) = bound.rename_label.upgrade()
            {
                labels.push(label);
            }
            true
        });
    }

    pub fn cancel_rename(&self) -> bool {
        collection_edit::cancel(&self.active_rename)
    }

    pub(in crate::ui) fn follow_renamed_result(
        &self,
        from: &std::path::Path,
        to: Option<&std::path::Path>,
    ) {
        for pane in self.all_panes() {
            pane.search.follow_rename(from, to);
        }
    }

    pub(in crate::ui) fn forget_renamed_result(
        &self,
        from: &std::path::Path,
        to: Option<&std::path::Path>,
    ) {
        for pane in self.all_panes() {
            pane.search.forget_rename(from, to);
        }
    }

    fn rename_submit(&self) -> collection_edit::Submit {
        let state = self.context_state.borrow().clone().unwrap_or_default();
        let active = Rc::downgrade(&self.active_rename);
        let browser = Rc::downgrade(&self.browser);
        Rc::new(move |field| {
            if let Some(state) = state.upgrade() {
                state.submit_mode_rename(field);
            } else {
                queue_detached_rename(&active, &browser, field);
            }
        })
    }

    pub(in crate::ui) fn icons_rename_view(&self, depth: usize) -> Option<gtk::Widget> {
        self.visible_panes()
            .into_iter()
            .find(|pane| self.mode.is_grid() && pane.depth == depth)
            .map(Pane::focus_view)
    }

    pub(in crate::ui) fn icons_rename_row(
        &self,
        depth: usize,
        source_position: usize,
    ) -> Option<(gtk::Widget, u32, Option<gtk::Widget>)> {
        let pane = self
            .visible_panes()
            .into_iter()
            .find(|pane| self.mode.is_grid() && pane.depth == depth)?;
        pane.item_sections().iter().find_map(|section| {
            let (position, row) = rename_row_in(pane, section, source_position)?;
            Some((section.view.clone(), position, row))
        })
    }

    pub(in crate::ui) fn list_rename_view(&self, depth: usize) -> Option<gtk::ListView> {
        self.list_pane
            .as_ref()
            .filter(|pane| self.mode == BrowserMode::List && pane.depth == depth)?
            .section
            .view
            .clone()
            .downcast()
            .ok()
    }

    pub(in crate::ui) fn list_rename_row(
        &self,
        depth: usize,
        source_position: usize,
    ) -> Option<(u32, Option<gtk::Widget>)> {
        let pane = self.list_pane.as_ref().filter(|pane| pane.depth == depth)?;
        rename_row_in(pane, &pane.section, source_position)
    }

    pub fn begin_search_rename(&self, depth: usize, entry: &FileEntry) -> bool {
        let Some(pane) = self.mode_pane(depth) else {
            return false;
        };
        self.cancel_rename();
        let Some(mut target) = pane.search.edit_target(entry) else {
            return false;
        };
        self.bind_rename_cancellation(&mut target, &entry.location);
        collection_edit::begin(
            &self.active_rename,
            entry.clone(),
            target,
            self.rename_submit(),
        )
    }

    pub fn begin_rename(
        &self,
        depth: usize,
        source_position: usize,
        entry: &FileEntry,
        naming: Naming,
    ) -> bool {
        self.cancel_rename();
        let Some(pane) = self.mode_pane(depth) else {
            return false;
        };
        let Some((widgets, row, collection)) = rename_target(pane, source_position) else {
            return false;
        };
        let mut target = EditTarget::from(widgets);
        target.naming = naming;
        self.bind_rename_cancellation(&mut target, &entry.location);
        if self.mode == BrowserMode::List {
            target.reveal = Some(self.rename_reveal(&row, &collection));
        }
        collection_edit::begin(
            &self.active_rename,
            entry.clone(),
            target,
            self.rename_submit(),
        )
    }

    /// Scrolls a List rename row back into view, until another rename starts.
    fn rename_reveal(&self, row: &gtk::Widget, collection: &gtk::Widget) -> Reveal {
        let state = self.context_state.borrow().clone().unwrap_or_default();
        let generation = reveal_generation(&state);
        let row = row.downgrade();
        let scroll = collection
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .map(|scroll| scroll.downgrade());
        Rc::new(move |_| {
            if reveal_generation(&state) != generation {
                return;
            }
            if let (Some(row), Some(scroll)) = (
                row.upgrade(),
                scroll.as_ref().and_then(glib::WeakRef::upgrade),
            ) {
                browser::reveal_rename_row(&row, &scroll, None);
            }
        })
    }

    fn bind_rename_cancellation(&self, target: &mut EditTarget, location: &Location) {
        let state = self.context_state.borrow().clone().unwrap_or_default();
        let location = location.clone();
        target.cancelled = Some(Rc::new(move || {
            if let Some(state) = state.upgrade() {
                state.cancel_group_naming(&location);
            }
        }));
    }

    pub(in crate::ui) fn refresh_name_highlights(&self, find: Option<&str>) {
        for pane in self.all_panes() {
            for section in pane.item_sections() {
                highlight_section_names(&section, find, &pane.filter_query.borrow());
            }
            pane.search.refresh_result_highlights(find);
        }
    }
}

fn reveal_generation(state: &Weak<ViewState>) -> Option<u64> {
    state
        .upgrade()
        .map(|state| state.rename_reveal_generation())
}

/// Without a view state, the submitted name is renamed directly.
fn queue_detached_rename(
    active: &Weak<std::cell::RefCell<Option<ActiveEdit>>>,
    browser: &Weak<Browser>,
    field: &gtk::Entry,
) {
    let Some(active) = active.upgrade() else {
        return;
    };
    let Some(submission) = collection_edit::take_submission(&active, field) else {
        return;
    };
    if let Some(browser) = browser.upgrade() {
        browser::queue_rename(&browser, submission.entry, submission.name);
    }
}

fn highlight_section_names(section: &PaneSection, find: Option<&str>, filter_query: &str) {
    for label in section
        .bound_items
        .borrow()
        .iter()
        .filter_map(|bound| bound.rename_label.upgrade())
    {
        browser::find::highlight_listing_name(&label, find, filter_query);
    }
}

/// The view position of a directory entry in `section`, and its row while it is mapped.
fn rename_row_in(
    pane: &Pane,
    section: &PaneSection,
    source_position: usize,
) -> Option<(u32, Option<gtk::Widget>)> {
    let position =
        view_position_for_source(&pane.model, Some(&section.view_model), source_position)?;
    Some((position, section.mapped_item_widget(position)))
}

fn rename_target(pane: &Pane, source_position: usize) -> Option<RenameTarget> {
    pane.item_sections().iter().find_map(|section| {
        let position =
            view_position_for_source(&pane.model, Some(&section.view_model), source_position)?;
        browser::prepare_collection_inline_edit(&section.view, position);
        let (widgets, row) = editable_item(pane, section, position)?;
        Some((widgets, row, section.view.clone()))
    })
}

/// The rendered item at `position`, once it is laid out where an editor can open.
fn editable_item(
    pane: &Pane,
    section: &PaneSection,
    position: u32,
) -> Option<(EditWidgets, gtk::Widget)> {
    section.bound_items.borrow().iter().find_map(|bound| {
        if bound.item.upgrade()?.position() != position {
            return None;
        }
        let widget = bound.widget.upgrade()?;
        ready_for_edit(&widget, &section.view, &pane.stack).then(|| (bound.edit.clone(), widget))
    })
}

fn ready_for_edit(widget: &gtk::Widget, view: &gtk::Widget, stack: &gtk::Stack) -> bool {
    let laid_out = widget.is_mapped() && widget.width() > 0;
    laid_out && widget.is_ancestor(view) && !stack.is_transition_running()
}
