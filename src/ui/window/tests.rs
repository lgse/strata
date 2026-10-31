// SPDX-License-Identifier: MIT

mod appearance_density;
mod bookmarks;
mod keyboard_dispatch;
mod keyboard_policy;
mod preferences;
mod rail;
mod sidebar;

use crate::test_support::gtk_test;

use super::{
    BrowserMode, MouseHistoryAction, SinglePaneArrow, TypeToSearchQuery, browser_mode_for_digit,
    is_browser_navigation_key, mouse_history_action, sidebar_focus_direction,
    single_pane_arrow_action, type_to_search_query,
};
use crate::ui::focus_navigation::vim_focus_direction;
