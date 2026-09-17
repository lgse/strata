// SPDX-License-Identifier: MIT

//! Custom actions inside the browser context menus.
//!
//! The section is rebuilt every time a menu opens, so the buttons it shows and
//! the paths they will run on come from one snapshot of the current selection.
//! That is what keeps a parent column from acting on whatever the deepest open
//! folder happens to be, and it means a stale menu cannot run an action on files
//! the user no longer sees.

use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    assets::icons,
    model::{FileEntry, Location},
    services::{InvocationSource, MatchedAction},
    ui::actions::{action_icon, folder_input, inputs_for_entries, native_paths, run_action},
};

use super::super::ViewState;
use super::{context_menu_option, item_context_option, keyboard};

/// Which menu the section is embedded in, so rows match that menu's styling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ActionMenuStyle {
    Item,
    Folder,
}

/// The "custom actions" block of a context menu.
pub(super) struct ActionMenuSection {
    container: gtk::Box,
    separator: gtk::Separator,
    top_items: gtk::Box,
    submenu_button: gtk::Button,
    submenu_popover: gtk::Popover,
    submenu_items: gtk::Box,
    parent_popover: gtk::Popover,
    style: ActionMenuStyle,
}

impl ActionMenuSection {
    pub(super) fn new(style: ActionMenuStyle, parent_popover: &gtk::Popover) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("context-menu-actions");
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.set_visible(false);
        container.append(&separator);
        let top_items = gtk::Box::new(gtk::Orientation::Vertical, 0);
        top_items.set_visible(false);
        container.append(&top_items);

        let submenu_items = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let submenu_scroll = gtk::ScrolledWindow::builder()
            .child(&submenu_items)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(420)
            .build();
        submenu_scroll.add_css_class("context-menu-scroll");
        // Anchored to the right of its row, the way a submenu is expected to open.
        let submenu_popover = gtk::Popover::builder()
            .has_arrow(false)
            .autohide(true)
            .position(gtk::PositionType::Right)
            .child(&submenu_scroll)
            .build();
        submenu_popover.add_css_class("folder-context-popover");
        keyboard::install(&submenu_popover);

        let submenu_button = match style {
            ActionMenuStyle::Item => item_context_option(icons::PLAY, "Actions", ""),
            ActionMenuStyle::Folder => context_menu_option(icons::PLAY, "Actions ▸", ""),
        };
        submenu_button.set_visible(false);
        let weak_popover = submenu_popover.downgrade();
        submenu_button.connect_clicked(move |button| {
            let Some(popover) = weak_popover.upgrade() else {
                return;
            };
            // The popover is detached whenever the parent menu closes, so it is
            // re-attached here instead of being left parented to a hidden button.
            if popover.parent().is_none() {
                popover.set_parent(button);
            }
            if !popover.is_visible() {
                popover.popup();
            }
        });
        container.append(&submenu_button);
        submenu_popover.set_parent(&submenu_button);

        // The submenu detaches once it is closed, so the button it is anchored to
        // is never finalized with a child and the next open re-parents it.
        let weak_submenu = submenu_popover.downgrade();
        submenu_popover.connect_closed(move |_| {
            if let Some(popover) = weak_submenu.upgrade()
                && popover.parent().is_some()
            {
                popover.unparent();
            }
        });
        // Closing the parent menu must take the submenu with it.
        let weak_submenu = submenu_popover.downgrade();
        parent_popover.connect_closed(move |_| {
            if let Some(popover) = weak_submenu.upgrade()
                && popover.is_visible()
            {
                popover.popdown();
            }
        });
        let weak_submenu = submenu_popover.downgrade();
        submenu_button.connect_destroy(move |_| {
            if let Some(popover) = weak_submenu.upgrade()
                && popover.parent().is_some()
            {
                popover.unparent();
            }
        });

        Self {
            container,
            separator,
            top_items,
            submenu_button,
            submenu_popover,
            submenu_items,
            parent_popover: parent_popover.clone(),
            style,
        }
    }

    pub(super) fn widget(&self) -> &gtk::Box {
        &self.container
    }

    /// Rebuilds for the current selection.
    pub(super) fn rebuild_for_selection(
        &self,
        state: &Rc<ViewState>,
        entries: &[FileEntry],
        parent: Option<PathBuf>,
    ) {
        let (Some(inputs), Some(paths), Some(parent)) =
            (inputs_for_entries(entries), native_paths(entries), parent)
        else {
            self.clear();
            return;
        };
        let catalog = crate::ui::actions::shared().catalog();
        self.rebuild(
            state,
            &catalog.matches(&inputs),
            paths,
            parent,
            InvocationSource::Selection,
        );
    }

    /// Rebuilds for a folder the menu was opened on.
    ///
    /// The folder itself is the input, and `background` tells the script it was
    /// invoked from the folder rather than from a selection inside it.
    pub(super) fn rebuild_for_folder(&self, state: &Rc<ViewState>, location: &Location) {
        let Some(input) = folder_input(location) else {
            self.clear();
            return;
        };
        let Some(path) = location.native_path() else {
            self.clear();
            return;
        };
        let catalog = crate::ui::actions::shared().catalog();
        self.rebuild(
            state,
            &catalog.matches(std::slice::from_ref(&input)),
            vec![path.to_path_buf()],
            path.to_path_buf(),
            InvocationSource::Background,
        );
    }

    fn clear(&self) {
        remove_children(&self.top_items);
        remove_children(&self.submenu_items);
        self.separator.set_visible(false);
        self.top_items.set_visible(false);
        self.submenu_button.set_visible(false);
        self.submenu_popover.popdown();
    }

    fn rebuild(
        &self,
        state: &Rc<ViewState>,
        matched: &[MatchedAction],
        paths: Vec<PathBuf>,
        parent: PathBuf,
        source: InvocationSource,
    ) {
        remove_children(&self.top_items);
        remove_children(&self.submenu_items);
        if matched.is_empty() {
            self.clear();
            return;
        }
        let mut top_count = 0usize;
        let mut submenu_count = 0usize;
        for matched_action in matched {
            let button = self.row(matched_action);
            let action = matched_action.action.clone();
            let paths = paths.clone();
            let parent = parent.clone();
            let weak_state = Rc::downgrade(state);
            let weak_submenu = self.submenu_popover.downgrade();
            let parent_popover = self.parent_popover.clone();
            button.connect_clicked(move |_| {
                if let Some(popover) = weak_submenu.upgrade()
                    && popover.is_visible()
                {
                    popover.popdown();
                }
                parent_popover.popdown();
                let Some(state) = weak_state.upgrade() else {
                    return;
                };
                run_action(
                    &state.overlay,
                    action.clone(),
                    paths.clone(),
                    parent.clone(),
                    source,
                );
            });
            if matched_action.placement == crate::model::MenuPlacement::Top {
                self.top_items.append(&button);
                top_count += 1;
            } else {
                self.submenu_items.append(&button);
                submenu_count += 1;
            }
        }
        self.separator.set_visible(true);
        self.top_items.set_visible(top_count > 0);
        self.submenu_button.set_visible(submenu_count > 0);
        if submenu_count == 0 {
            self.submenu_popover.popdown();
        }
    }

    fn row(&self, action: &MatchedAction) -> gtk::Button {
        let label = action.action.name();
        let button = match self.style {
            ActionMenuStyle::Item => item_context_option(
                action_icon(action.action.definition.icon.as_deref()),
                label,
                "",
            ),
            ActionMenuStyle::Folder => context_menu_option(
                action_icon(action.action.definition.icon.as_deref()),
                label,
                "",
            ),
        };
        if action.action.is_available() {
            if let Some(description) = action.action.definition.description.as_deref() {
                button.set_tooltip_text(Some(description));
            }
        } else {
            // An action that cannot run stays visible but explains why, instead
            // of disappearing and looking like a bug.
            button.set_sensitive(false);
            button.set_tooltip_text(action.action.unavailable_reason());
        }
        button
    }
}

fn remove_children(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
