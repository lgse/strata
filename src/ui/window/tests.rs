// SPDX-License-Identifier: MIT

mod bookmarks;
mod keyboard_policy;
mod preferences;

use crate::test_support::gtk_test;

use super::{
    BrowserMode, DEFAULT_ACCELS, MouseHistoryAction, SinglePaneArrow, TypeToSearchQuery,
    browser_mode_for_digit, is_browser_navigation_key, is_context_menu_shortcut,
    is_native_editing_shortcut, is_open_terminal_shortcut, is_redo_shortcut, is_refresh_shortcut,
    is_rename_shortcut, is_sidebar_focus_shortcut, is_toggle_hidden_shortcut, is_undo_shortcut,
    jump_direction, mouse_history_action, page_direction, sidebar_focus_direction,
    single_pane_arrow_action, type_to_search_query, vim_focus_direction,
};
