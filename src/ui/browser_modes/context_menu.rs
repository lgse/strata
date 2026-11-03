// SPDX-License-Identifier: MIT

//! Context menus for a pane's items and for the folder it shows.

use std::rc::{Rc, Weak};

use gtk::prelude::*;

use super::{
    BrowserMode, ModeViews,
    pane::{Pane, PaneSection},
    selection::SectionOwner,
    source_index::{source_position_for_view, view_position_for_source},
};
use crate::ui::browser::{self, ContextMenuTarget, ContextMenuTrigger, ViewState};

impl ModeViews {
    pub(super) fn install_context_menu(&self, pane: &Pane) -> ContextMenuTrigger {
        let Some(state) = self.context_state.borrow().as_ref().and_then(Weak::upgrade) else {
            return Rc::new(|_, _| {});
        };
        let Some(location) = self.browser.location_at(pane.depth) else {
            return Rc::new(|_, _| {});
        };
        pane.search.install_context_menu(|view, resolve| {
            browser::install_resolved_item_context_menu(&state, view, resolve, pane.depth)
        });
        let search = pane.search.clone();
        let sections = Rc::downgrade(&pane.sections);
        let entries = pane.model.downgrade();
        browser::install_folder_context_menu(
            &state,
            pane.stack.upcast_ref(),
            Rc::new(move || {
                entries
                    .upgrade()
                    .is_some_and(|entries| entries.n_items() > 0)
            }),
            Rc::new(move |picked| {
                search.is_item_target(picked)
                    || sections.upgrade().is_some_and(|sections| {
                        sections
                            .borrow()
                            .iter()
                            .any(|section| section_item_position(section, picked).is_some())
                    })
            }),
            pane.depth,
            location,
        )
    }

    pub(in crate::ui) fn context_menu_target(
        &self,
        depth: usize,
        position: Option<usize>,
    ) -> Option<ContextMenuTarget> {
        let pane = self.mode_pane(depth)?;
        if pane.search.selected_entries().is_some() {
            if let Some(target) = pane.search.context_menu_target() {
                return Some(target);
            }
        } else if let Some(position) = position {
            return self.item_menu_target(pane, position);
        }
        folder_menu_target(pane)
    }

    /// Opens beside the rendered item of the entry at `position`.
    fn item_menu_target(&self, pane: &Pane, position: usize) -> Option<ContextMenuTarget> {
        pane.item_sections().into_iter().find_map(|section| {
            let view_position =
                view_position_for_source(&pane.model, Some(&section.view_model), position)?;
            let widget = section.item_widget(view_position)?;
            let bounds = widget.compute_bounds(&section.view)?;
            let x = if self.mode == BrowserMode::List {
                bounds.center().x()
            } else {
                bounds.x() + bounds.width()
            };
            Some((
                section.item_context_trigger.clone(),
                f64::from(x),
                f64::from(bounds.center().y()),
            ))
        })
    }
}

/// Opens at the middle of the folder's listing.
fn folder_menu_target(pane: &Pane) -> Option<ContextMenuTarget> {
    let width = f64::from(pane.stack.width());
    let height = f64::from(pane.stack.height());
    (width > 0.0 && height > 0.0).then(|| {
        (
            pane.folder_context_trigger.clone(),
            width / 2.0,
            height / 2.0,
        )
    })
}

/// The view position of the item `picked` belongs to, when it is one of the section's
/// rendered items.
fn section_item_position(section: &PaneSection, picked: &gtk::Widget) -> Option<u32> {
    let mut candidate = Some(picked.clone());
    while let Some(widget) = candidate {
        let position = section.bound_items.borrow().iter().find_map(|bound| {
            let bound_widget = bound.widget.upgrade()?;
            let item = bound.item.upgrade()?;
            (bound_widget == widget).then_some(item.position())
        });
        if position.is_some() {
            return position;
        }
        candidate = widget.parent();
    }
    None
}

pub(super) fn install_section_context_menu(
    state: &Rc<ViewState>,
    section: &PaneSection,
    owner: &SectionOwner,
) -> ContextMenuTrigger {
    let items = section.bound_items.clone();
    let pick_position = Rc::new(move |picked: &gtk::Widget| {
        items.borrow().iter().find_map(|bound| {
            let widget = bound.widget.upgrade()?;
            (widget == *picked || picked.is_ancestor(&widget))
                .then(|| bound.item.upgrade().map(|item| item.position()))?
        })
    });
    let source_index = owner.source_index.clone();
    let view_model = section.view_model.clone();
    let source_position = Rc::new(move |position| {
        source_position_for_view(&source_index, Some(&view_model), position)
    });
    let owner_view = section.view.downgrade();
    let sections = owner.sections.clone();
    let clear_other_selections = Rc::new(move || {
        let Some(sections) = sections.upgrade() else {
            return;
        };
        for other in sections.borrow().iter() {
            if Some(&other.view) == owner_view.upgrade().as_ref() {
                continue;
            }
            other.syncing.set(true);
            other.selection.unselect_all();
            other.syncing.set(false);
        }
    });
    browser::install_item_context_menu(
        state,
        &section.view,
        &section.selection,
        pick_position,
        source_position,
        clear_other_selections,
        owner.depth,
    )
}
