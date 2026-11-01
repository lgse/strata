// SPDX-License-Identifier: MIT

use super::super::state::select_name_stem;
use super::*;

impl ChooserShortcuts {
    pub(super) fn column_popover_focus(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        let popover = context
            .focused_popover()
            .filter(|popover| popover.has_css_class("column-popover"))?;
        let direction = vim_focus_direction(&context.press)?;
        popover.child_focus(direction);
        STOP
    }

    pub(super) fn default_tab(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if context.tenxer() {
            return None;
        }
        let direction = context.tab()?;
        if context.focus_in_popover() {
            return None;
        }
        context
            .view()
            .default_tab(direction)
            .then_some(glib::Propagation::Stop)
    }

    /// In 10xer mode Tab moves between the name field and the items or header.
    pub(super) fn tenxer_name_tab(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.tenxer() || context.tab().is_none() {
            return None;
        }
        let filename = context.state.filename.as_ref()?;
        if context.name_has_focus() {
            context.browser.focus_active();
            return STOP;
        }
        if !context.focus_within(&self.header) {
            return None;
        }
        filename.grab_focus_without_selecting();
        select_name_stem(filename);
        STOP
    }

    pub(super) fn focused_widget_key(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if context.alt() {
            return None;
        }
        let focused = context.focused.as_ref()?;
        if crate::ui::focus_navigation::in_popover(focused) {
            return PROCEED;
        }
        if context.press.is(ENTER.ignoring(ModifierType::SUPER_MASK))
            && crate::ui::focus_navigation::activate(context.state.window.upcast_ref())
        {
            return STOP;
        }
        crate::ui::focus_navigation::editable(focused).then_some(glib::Propagation::Proceed)
    }

    pub(super) fn tenxer_tab_to_header(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        if !context.tenxer() || !context.view().item_view_has_focus() {
            return None;
        }
        if context.tab() != Some(gtk::DirectionType::TabForward) {
            return None;
        }
        if self.header.child_focus(gtk::DirectionType::TabForward) {
            context.state.window.set_focus_visible(true);
        }
        STOP
    }

    pub(super) fn tenxer_tab(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.tenxer() || context.tab().is_none() {
            return None;
        }
        if !crate::ui::focus_navigation::contains_widget(
            &context.view().widget(),
            context.focused.as_ref(),
        ) {
            context.browser.focus_active();
        }
        STOP
    }

    pub(super) fn header_actions(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.view().header_actions_have_focus() {
            return None;
        }
        let press = &context.press;
        if press.is(LEFT.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            if context.view().move_header_focus(gtk::DirectionType::Left) || context.tenxer() {
                return STOP;
            }
            context.header_left_boundary.set(true);
        } else if press.is(RIGHT.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            context.view().move_header_focus(gtk::DirectionType::Right);
            return STOP;
        } else if press.is(DOWN.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            context.view().focus_items_from_header();
            return STOP;
        }
        None
    }

    pub(super) fn sidebar_arrows(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !self.sidebar_has_focus(context) || context.tenxer() {
            return None;
        }
        let direction = vim_focus_direction(&context.press)
            .or_else(|| arrow_direction(&context.press, FOCUS_MODIFIERS))?;
        if direction == gtk::DirectionType::Right {
            self.focus_before_sidebar.borrow_mut().take();
            context.browser.focus_active();
        } else if !self.sidebar_widget.child_focus(direction)
            && direction == gtk::DirectionType::Up
        {
            self.sidebar_toggle.grab_focus();
            context.state.window.set_focus_visible(true);
        }
        STOP
    }

    pub(super) fn dismiss_empty_filter(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        (context.press.is(BACKSPACE.ignoring(FOCUS_MODIFIERS))
            && context.view().dismiss_empty_focused_filter())
        .then_some(glib::Propagation::Stop)
    }

    /// Keys outside the item view move focus between the chooser's controls.
    pub(super) fn outside_item_view(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if context.alt() || context.view().item_view_has_focus() {
            return None;
        }
        if context.header_left_boundary.get() {
            return None;
        }
        if context.tenxer() && focus_direction_key(&context.press) {
            context.browser.focus_active();
            return vim_focus_direction(&context.press)
                .is_some()
                .then_some(glib::Propagation::Stop);
        }
        Some(move_between_controls(context))
    }

    pub(super) fn left_to_sidebar(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(LEFT.ignoring(ModifierType::SUPER_MASK))
            || !self.sidebar_toggle.is_active()
        {
            return None;
        }
        if !context.view().item_view_has_focus() || !context.view().item_at_sidebar_edge() {
            return None;
        }
        if context.arrow_scoped() {
            return None;
        }
        self.enter_sidebar(context);
        context.state.window.set_focus_visible(true);
        STOP
    }
}

fn move_between_controls(context: &KeyContext<'_>) -> glib::Propagation {
    let Some(direction) = arrow_direction(&context.press, ModifierType::SUPER_MASK) else {
        return glib::Propagation::Proceed;
    };
    if direction == gtk::DirectionType::Up && within_chooser_options(context.focused.as_ref()) {
        context.browser.focus_active();
        return glib::Propagation::Stop;
    }
    if crate::ui::focus_navigation::move_focus(context.state.window.upcast_ref(), direction) {
        return glib::Propagation::Stop;
    }
    glib::Propagation::Proceed
}

fn focus_direction_key(press: &KeyPress) -> bool {
    arrow_direction(press, FOCUS_MODIFIERS).is_some() || vim_focus_direction(press).is_some()
}

fn within_chooser_options(focused: Option<&gtk::Widget>) -> bool {
    std::iter::successors(focused.cloned(), gtk::Widget::parent)
        .any(|widget| widget.has_css_class("chooser-options"))
}
