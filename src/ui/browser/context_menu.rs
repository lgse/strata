// SPDX-License-Identifier: MIT

use crate::model::FileEntry;
use crate::ui::browser::ViewState;
use std::cell::RefCell;
use std::rc::Rc;

mod actions;
mod commands;
mod focus;
mod folder_menu;
mod item_menu;
mod item_options;
mod keyboard;
mod open_with;
mod options;
mod popover;
mod presentation;
pub(in crate::ui::browser) use focus::{
    bind_column_context_owner, context_search_active, focus_context_column, focus_context_entry,
    preview_context_entry, rename_context_entry,
};
use focus::{context_filter_or_search_active, current_context_position};
pub(in crate::ui) use folder_menu::install_folder_context_menu;
pub(in crate::ui) use item_menu::{install_item_context_menu, install_resolved_item_context_menu};
pub(in crate::ui::browser) use options::context_menu_danger_option;
pub(in crate::ui) use options::context_menu_option;
pub(in crate::ui) use popover::{context_menu_popover, show_context_popover};
use popover::{install_secondary_release_retarget, show_model_context_popover};
#[cfg(test)]
use {
    crate::adapters::gio_file_for_location, crate::model::Location,
    crate::ui::browser::paths::is_trash_location, crate::ui::browser_modes::BrowserMode,
    gtk::prelude::*, open_with::OpenWithSelection,
};

/// `commands::collect` turns a box with this class into a native submenu
/// instead of flattening its buttons into top-level rows.
pub(super) const SUBMENU_ROW_CLASS: &str = "item-context-submenu";
pub(super) const SUBMENU_ENTRIES_CLASS: &str = "item-context-submenu-entries";

pub(in crate::ui) type ContextPickPosition = Rc<dyn Fn(&gtk::Widget) -> Option<u32>>;

pub(in crate::ui) type ContextSourcePosition = Rc<dyn Fn(u32) -> Option<usize>>;

pub(in crate::ui) type ContextMenuTrigger = Rc<dyn Fn(f64, f64)>;
type ResolvedContextTrigger = Rc<dyn Fn(f64, f64, Option<&gtk::GestureClick>) -> bool>;
pub(in crate::ui) type ContextMenuTarget = (ContextMenuTrigger, f64, f64);
pub(in crate::ui) type ContextTarget = (Option<usize>, FileEntry);
pub(in crate::ui) type ContextResolver = Rc<dyn Fn(&gtk::Widget) -> Option<ContextTarget>>;

const ITEM_CONTEXT_SUMMARY_MAX_CHARS: i32 = 30;

fn selected_items_summary(entries: &[FileEntry]) -> String {
    let mut names = entries
        .iter()
        .take(3)
        .map(|entry| entry.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if entries.len() > 3 {
        names.push_str(", …");
    }
    let max_chars = ITEM_CONTEXT_SUMMARY_MAX_CHARS as usize;
    if names.chars().count() > max_chars {
        names = names.chars().take(max_chars - 1).collect();
        names.push('…');
    }
    names
}

pub(super) fn context_entries(
    state: &Rc<ViewState>,
    target: &RefCell<Option<ContextTarget>>,
) -> Vec<FileEntry> {
    if let Some((_, entry)) = target.borrow().as_ref()
        && let Some(entries) = (super::BrowserView {
            state: state.clone(),
        })
        .selected_search_results()
    {
        return if entries
            .iter()
            .any(|selected| selected.location == entry.location)
        {
            entries
        } else {
            vec![entry.clone()]
        };
    }
    if let Some((None, entry)) = target.borrow().as_ref() {
        return vec![entry.clone()];
    }
    state.sync_mode_selection();
    let entries = state.browser.selected_entries();
    let target = target.borrow();
    let Some((_, target)) = target.as_ref() else {
        return entries;
    };
    if entries
        .iter()
        .any(|entry| entry.location == target.location)
    {
        entries
    } else {
        vec![target.clone()]
    }
}

/// In Trash this shared action deletes permanently, so `can_trash` is irrelevant.
/// Unknown capabilities retain the delete fallback (#179); callers exclude
/// nested Trash children, which GVfs cannot remove independently.
pub(super) fn move_to_trash_is_visible(in_trash: bool, can_trash: Option<bool>) -> bool {
    in_trash || can_trash.unwrap_or(true)
}

/// Hidden in Trash, where the shared delete action is already permanent, or
/// when GIO confirms deletion is unsupported.
pub(super) fn permanently_delete_is_visible(in_trash: bool, can_delete: Option<bool>) -> bool {
    !in_trash && can_delete.unwrap_or(true)
}

#[cfg(test)]
mod tests;
