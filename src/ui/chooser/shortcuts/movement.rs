// SPDX-License-Identifier: MIT

use super::*;

const ALT_SUPER: ModifierType = ModifierType::ALT_MASK.union(ModifierType::SUPER_MASK);

impl ChooserShortcuts {
    pub(super) fn toggle_preview(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        let press = &context.press;
        if !press.is(PREVIEW.in_map(Map::Default).ignoring(FOCUS_MODIFIERS)) {
            return None;
        }
        if press.is(PREVIEW.in_map(Map::Default)) && context.view().activate_directory_on_space() {
            return STOP;
        }
        self.preview.toggle(
            preview_target(context.browser.focused_entry()),
            context.browser.active_depth(),
        );
        STOP
    }

    pub(super) fn navigate_up(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(BACKSPACE.ignoring(FOCUS_MODIFIERS)) {
            return None;
        }
        context.view().navigate_up();
        STOP
    }

    /// Arrows in the list and icon views keep GTK's spatial movement.
    pub(super) fn item_arrows(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if context.alt() || context.view().view_mode() == BrowserMode::Columns {
            return None;
        }
        let direction = arrow_direction(&context.press, FOCUS_MODIFIERS)?;
        let extend = context.held.contains(ModifierType::SHIFT_MASK) && context.multiple();
        if context.view().cross_type_group(direction, extend) {
            return STOP;
        }
        if up_reaches_header(context, UP.ignoring(ModifierType::SUPER_MASK)) {
            return STOP;
        }
        synchronize_selection_after_move(context.state, extend);
        if context.press.aliased()
            && let Some(arrow) = crate::ui::focus_navigation::spatial_arrow(&context.press)
        {
            crate::ui::focus_navigation::activate_native_arrow(&context.state.window, arrow.key());
            return STOP;
        }
        PROCEED
    }

    pub(super) fn extend_selection(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.multiple() {
            return None;
        }
        let step = if context.press.is(EXTEND_UP.ignoring(ALT_SUPER)) {
            -1
        } else if context.press.is(EXTEND_DOWN.ignoring(ALT_SUPER)) {
            1
        } else {
            return None;
        };
        context.browser.extend_selection(step);
        STOP
    }

    pub(super) fn type_to_search(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.view().item_view_has_focus() {
            return None;
        }
        let query = crate::ui::window::type_to_search_query(&context.press)?;
        if !context.preferences.type_to_search() {
            return None;
        }
        let shown = match query {
            crate::ui::window::TypeToSearchQuery::Empty => context.view().show_filter(),
            crate::ui::window::TypeToSearchQuery::Character(character) => context
                .view()
                .show_filter_with_query(&character.to_string()),
        };
        shown.then_some(glib::Propagation::Stop)
    }

    pub(super) fn up_to_header(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        up_reaches_header(context, UP.ignoring(ALT_SUPER).or_alternates())
            .then_some(glib::Propagation::Stop)
    }

    pub(super) fn jump(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        let press = &context.press;
        if press.is(HISTORY_BACK.in_map(Map::Default).ignoring(FOCUS_MODIFIERS)) {
            context.browser.back();
        } else if press.is(HISTORY_FORWARD
            .in_map(Map::Default)
            .ignoring(FOCUS_MODIFIERS))
        {
            context.browser.forward();
        } else if press.is(PARENT_FOLDER.ignoring(FOCUS_MODIFIERS)) {
            context.browser.parent();
        } else if press.is(HOME_FOLDER.ignoring(FOCUS_MODIFIERS)) {
            context.browser.navigate(Location::local(home_directory()));
        } else {
            return None;
        }
        STOP
    }

    pub(super) fn move_or_open(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        let press = &context.press;
        if press.is(DOWN.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            context.browser.move_selection(1);
        } else if press.is(UP.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            context.browser.move_selection(-1);
        } else if press.is(LEFT.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            self.move_left(context);
        } else if press.is(RIGHT.ignoring(FOCUS_MODIFIERS))
            && context.view().view_mode() == BrowserMode::Columns
        {
            context.browser.enter_focused_directory();
        } else if press.is_any(&[RIGHT.alternates(), ENTER.ignoring(FOCUS_MODIFIERS)]) {
            self.open_focused(context);
        } else {
            return None;
        }
        STOP
    }

    fn move_left(&self, context: &KeyContext<'_>) {
        let to_sidebar = context.view().first_column_has_focus() && self.sidebar_toggle.is_active();
        if to_sidebar && !context.arrow_scoped() {
            self.enter_sidebar(context);
        } else {
            context.view().navigate_left();
        }
    }

    fn open_focused(&self, context: &KeyContext<'_>) {
        if !self.tenxer.confirm_focused() {
            context.view().activate_focused();
        }
    }
}

fn up_reaches_header(context: &KeyContext<'_>, up: keys::Matcher) -> bool {
    if !context.press.is(up) || context.arrow_scoped() {
        return false;
    }
    context.view().focus_header_from_top_item()
}

/// Reconciles the selection in visual order once GTK has moved focus.
fn synchronize_selection_after_move(state: &Rc<ChooserState>, extend: bool) {
    let weak = Rc::downgrade(state);
    glib::idle_add_local_once(move || {
        let Some(state) = weak.upgrade() else {
            return;
        };
        if !state.view.item_view_has_focus() {
            return;
        }
        state.view.synchronize_native_selection(extend);
    });
}
