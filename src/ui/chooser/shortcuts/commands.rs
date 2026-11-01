// SPDX-License-Identifier: MIT

use super::*;

impl ChooserShortcuts {
    pub(super) fn mark_keyboard_navigation(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        if context.focus_in_popover() {
            return None;
        }
        if context.press.is(SIDEBAR_FOCUS) || browser_key_outside_text(context) {
            context.view().keyboard_navigation();
        }
        None
    }

    /// An arrow with focus on nothing usable puts it back on a chooser control.
    pub(super) fn recover_lost_focus(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        arrow_direction(&context.press, ModifierType::empty())?;
        context.state.window.set_focus_visible(true);
        let lost = context
            .focused
            .as_ref()
            .is_none_or(|widget| !widget.is_mapped() || !widget.is_sensitive());
        if !lost {
            return None;
        }
        if context.tenxer() {
            context.browser.focus_active();
        } else if let Some(filename) = context.state.filename.as_ref() {
            filename.grab_focus();
        } else {
            self.sidebar_toggle.grab_focus();
        }
        STOP
    }

    pub(super) fn rename(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(RENAME.in_map(Map::Default))
            || context.browser.selected_entries().len() != 1
        {
            return None;
        }
        if context.focus_in_text_or_popover() {
            return None;
        }
        context
            .view()
            .begin_rename()
            .then_some(glib::Propagation::Stop)
    }

    pub(super) fn properties(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context
            .press
            .is(PROPERTIES.ignoring(ModifierType::SUPER_MASK))
            || !context.item_view_idle()
        {
            return None;
        }
        context
            .view()
            .show_focused_properties()
            .then_some(glib::Propagation::Stop)
    }

    pub(super) fn editing_keeps_key(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        context
            .item_view_editing()
            .then_some(glib::Propagation::Proceed)
    }

    pub(super) fn preview_search_result(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        if !context.press.is(PREVIEW.in_map(Map::Default)) {
            return None;
        }
        let entry = context.view().search_result_for_space()?;
        if !context.view().activate_directory_on_space() {
            self.preview
                .toggle(preview_target(Some(entry)), context.browser.active_depth());
        }
        STOP
    }

    pub(super) fn filter(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        (context.press.is(FILTER.in_map(Map::Default)) && context.view().show_filter())
            .then_some(glib::Propagation::Stop)
    }

    pub(super) fn location(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(LOCATION) {
            return None;
        }
        context.view().begin_location_edit();
        STOP
    }

    pub(super) fn sidebar_focus(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(SIDEBAR_FOCUS) {
            return None;
        }
        if context.tenxer() {
            self.tenxer_sidebar_focus(context);
        } else {
            self.toggle_sidebar_focus(context);
        }
        STOP
    }

    /// 10xer mode only moves focus into a sidebar that is already shown.
    fn tenxer_sidebar_focus(&self, context: &KeyContext<'_>) {
        if !self.sidebar_toggle.is_active() {
            return;
        }
        if !self.sidebar_has_focus(context) {
            self.enter_sidebar(context);
            context.state.window.set_focus_visible(true);
            return;
        }
        let restored = self
            .focus_before_sidebar
            .borrow_mut()
            .take()
            .is_some_and(|widget| widget.is_mapped() && widget.grab_focus());
        if !restored {
            context.browser.focus_active();
        }
    }

    fn toggle_sidebar_focus(&self, context: &KeyContext<'_>) {
        if self.sidebar_has_focus(context) {
            let restored = self
                .focus_before_sidebar
                .borrow_mut()
                .take()
                .is_some_and(|widget| widget.grab_focus());
            if !restored {
                context.browser.focus_active();
            }
            return;
        }
        self.focus_before_sidebar.replace(context.focused.clone());
        if !self.sidebar_toggle.is_active() {
            self.sidebar_toggle.set_active(true);
        }
        let sidebar = self.sidebar_state.clone();
        glib::idle_add_local_once(move || {
            sidebar.focus_active_place();
        });
    }

    pub(super) fn sidebar_toggle(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(SIDEBAR) {
            return None;
        }
        if context
            .state
            .destination_host
            .as_ref()
            .is_none_or(|host| !host.confined)
        {
            self.sidebar_toggle.emit_clicked();
        }
        STOP
    }

    pub(super) fn location_keeps_key(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        context
            .view()
            .location_has_focus()
            .then_some(glib::Propagation::Proceed)
    }

    pub(super) fn choose_folder(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        let chooses_folders = matches!(
            &context.state.request.kind,
            ChooserKind::Open {
                directory: true,
                ..
            }
        );
        if !chooses_folders || !item_view_command(context, CHOOSE_FOLDER) {
            return None;
        }
        context.state.accept();
        STOP
    }

    pub(super) fn new_folder(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(NEW_FOLDER) {
            return None;
        }
        if context
            .state
            .destination_host
            .as_ref()
            .is_none_or(|host| host.allow_create)
        {
            context.view().create_new_folder();
        }
        STOP
    }

    pub(super) fn select_all(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(SELECT_ALL) || !context.view().item_view_has_focus() {
            return None;
        }
        if !context.multiple() {
            return None;
        }
        context.view().select_all();
        STOP
    }

    pub(super) fn refresh(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(REFRESH.ignoring(SHORTCUT_MODIFIERS)) {
            return None;
        }
        if let Some(depth) = context.browser.active_depth() {
            crate::adapters::trash_support::forget_all();
            context.browser.retry_column(depth);
        }
        STOP
    }

    pub(super) fn hidden_files(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(HIDDEN_FILES.in_map(Map::Default)) {
            return None;
        }
        context.browser.toggle_hidden();
        STOP
    }

    pub(super) fn delete(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        let permanent = delete_permanence(&context.press)?;
        if !context.view().item_view_has_focus() || context.view().filter_has_focus() {
            return None;
        }
        context
            .view()
            .confirm_delete(permanent)
            .then_some(glib::Propagation::Stop)
    }

    pub(super) fn browser_mode(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        let mode = crate::ui::window::browser_mode_for_digit(&context.press)?;
        crate::ui::window::apply_browser_mode(context.view(), &context.preferences, mode);
        STOP
    }

    pub(super) fn control_keeps_key(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        // A user's binding for a folder jump may hold Ctrl, as in the window.
        let jumps = context.press.is_any(&[
            HISTORY_BACK.in_map(Map::Default),
            HISTORY_FORWARD.in_map(Map::Default),
            PARENT_FOLDER.into(),
            HOME_FOLDER.into(),
        ]);
        (context.control() && !jumps).then_some(glib::Propagation::Proceed)
    }
}

fn browser_key_outside_text(context: &KeyContext<'_>) -> bool {
    !context.focus_editable() && crate::ui::window::is_browser_navigation_key(&context.press)
}

/// The command, tolerating Super, while the item view has focus.
fn item_view_command(context: &KeyContext<'_>, command: keys::Command) -> bool {
    context.press.is(command.ignoring(ModifierType::SUPER_MASK))
        && context.view().item_view_has_focus()
}

fn delete_permanence(press: &KeyPress) -> Option<bool> {
    let tolerated = ModifierType::SUPER_MASK;
    if press.is(MOVE_TO_TRASH.in_map(Map::Default).ignoring(tolerated)) {
        return Some(false);
    }
    press
        .is(DELETE_PERMANENTLY.in_map(Map::Default).ignoring(tolerated))
        .then_some(true)
}
