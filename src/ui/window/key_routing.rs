// SPDX-License-Identifier: MIT

//! Key and mouse-button decisions shared by the window and chooser dispatchers.

use std::rc::Rc;

use gtk::gdk::ModifierType;

use crate::ui::{
    browser::BrowserView,
    browser_modes::BrowserMode,
    keys::{
        KeyPress,
        bindings::{
            BACKSPACE, DOWN, END_KEY, ENTER, FILTER_SLASH, FOCUS_NEXT, HOME_KEY, LEFT,
            PAGE_DOWN_KEY, PAGE_UP_KEY, RIGHT, SPACE, UP, VIEW_COLUMNS, VIEW_COMPACT, VIEW_ICONS,
            VIEW_LIST,
        },
    },
    preferences::PreferenceManager,
};

use super::{ChooserKeys, ChooserPolicy, SidebarView, keyboard};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MouseHistoryAction {
    Back,
    Forward,
}

#[derive(Clone)]
pub(super) struct TypeToSearch {
    pub(super) view: BrowserView,
    pub(super) preferences: Rc<PreferenceManager>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::ui) enum TypeToSearchQuery {
    Empty,
    Character(char),
}

impl TypeToSearch {
    pub(in crate::ui) fn show(&self, query: TypeToSearchQuery) -> bool {
        self.preferences.type_to_search_active()
            && match query {
                TypeToSearchQuery::Empty => self.view.show_filter(),
                TypeToSearchQuery::Character(character) => {
                    self.view.show_filter_with_query(&character.to_string())
                }
            }
    }
}

pub(in crate::ui) fn chooser_keys(
    window: &gtk::Window,
    view: &BrowserView,
    sidebar: &SidebarView,
    top_bar: crate::ui::top_bar_navigation::TopBarNavigation,
    preview: &crate::ui::preview::PreviewDrawer,
    shortcuts: &crate::ui::shortcut_footer::ShortcutFooter,
    policy: ChooserPolicy,
) -> ChooserKeys {
    ChooserKeys::new(
        window,
        sidebar,
        keyboard::DispatcherParts {
            view: view.clone(),
            top_bar,
            preview: preview.clone(),
            type_to_search: TypeToSearch {
                view: view.clone(),
                preferences: PreferenceManager::shared(),
            },
            shortcuts: shortcuts.clone(),
            history: crate::services::NavigationHistory::shared(),
        },
        policy,
    )
}

pub(super) fn mouse_history_action(button: u32) -> Option<MouseHistoryAction> {
    match button {
        8 => Some(MouseHistoryAction::Back),
        9 => Some(MouseHistoryAction::Forward),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SinglePaneArrow {
    Native,
    Stay,
    Sidebar,
}

pub(super) fn single_pane_arrow_action(
    mode: BrowserMode,
    press: &KeyPress,
    at_left_edge: bool,
    sidebar_visible: bool,
) -> Option<SinglePaneArrow> {
    // GTK moves or extends the selection with Ctrl and Shift arrows.
    const NATIVE: ModifierType = ModifierType::CONTROL_MASK.union(ModifierType::SHIFT_MASK);
    if mode == BrowserMode::Columns {
        return None;
    }
    let sidebar_reachable = at_left_edge && sidebar_visible;
    let horizontal = press.is_any(&[LEFT.ignoring(NATIVE), RIGHT.ignoring(NATIVE)]);
    let vertical = press.is_any(&[UP.ignoring(NATIVE), DOWN.ignoring(NATIVE)]);
    if sidebar_reachable && press.is(LEFT) {
        Some(SinglePaneArrow::Sidebar)
    } else if horizontal {
        Some(horizontal_arrow_action(mode))
    } else {
        vertical.then_some(SinglePaneArrow::Native)
    }
}

/// List rows have no horizontal neighbours, so sideways arrows stay put there.
fn horizontal_arrow_action(mode: BrowserMode) -> SinglePaneArrow {
    if mode == BrowserMode::List {
        SinglePaneArrow::Stay
    } else {
        SinglePaneArrow::Native
    }
}

pub(in crate::ui) fn is_browser_navigation_key(press: &KeyPress) -> bool {
    const TOLERATED: ModifierType = ModifierType::SHIFT_MASK.union(ModifierType::ALT_MASK);
    [UP, DOWN, LEFT, RIGHT, FOCUS_NEXT, ENTER]
        .into_iter()
        .any(|key| press.is(key.ignoring(TOLERATED).or_alternates()))
        || [HOME_KEY, END_KEY, PAGE_UP_KEY, PAGE_DOWN_KEY, BACKSPACE]
            .into_iter()
            .any(|key| press.is(key.ignoring(TOLERATED)))
}

pub(in crate::ui) fn type_to_search_query(press: &KeyPress) -> Option<TypeToSearchQuery> {
    // Space belongs to quick preview; focused text fields handle their own spaces.
    if press.is(SPACE.ignoring(ModifierType::SHIFT_MASK)) {
        return None;
    }
    // A letter that reads as `/` only through the layout fallback is typed.
    if press.is(FILTER_SLASH) && !press.used_fallback() {
        return Some(TypeToSearchQuery::Empty);
    }
    press.text().map(TypeToSearchQuery::Character)
}

pub(super) fn sidebar_focus_direction(
    press: &KeyPress,
    tolerated: ModifierType,
) -> Option<gtk::DirectionType> {
    crate::ui::focus_navigation::arrow_direction(press, tolerated)
        .or_else(|| crate::ui::focus_navigation::vim_focus_direction(press))
}

pub(in crate::ui) fn apply_browser_mode(
    view: &BrowserView,
    preferences: &PreferenceManager,
    mode: BrowserMode,
) {
    view.set_view_mode(mode);
    preferences.set_browser_mode(mode);
}

pub(in crate::ui) fn browser_mode_for_digit(press: &KeyPress) -> Option<BrowserMode> {
    [
        (VIEW_COLUMNS, BrowserMode::Columns),
        (VIEW_ICONS, BrowserMode::Icons),
        (VIEW_LIST, BrowserMode::List),
        (VIEW_COMPACT, BrowserMode::Compact),
    ]
    .into_iter()
    .find(|(view, _)| press.is(*view))
    .map(|(_, mode)| mode)
}
