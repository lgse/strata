// SPDX-License-Identifier: MIT

use super::*;

impl ChooserShortcuts {
    /// In 10xer mode a focused name or location field types every key but these.
    /// Tab and Shift+Tab leave the name for the options or the files, never Cancel.
    pub(super) fn tenxer_text_keeps_key(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        if !context.tenxer() {
            return None;
        }
        let typing = context.name_has_focus() || context.view().location_has_focus();
        let reaches_chooser = context.press.is_any::<keys::Matcher>(&[
            REFERENCE.in_map(Map::Default).ignoring(SHORTCUT_MODIFIERS),
            ESCAPE_ANY,
            TENXER_MODE.into(),
        ]);
        let name_tab = context.name_has_focus() && context.tab().is_some();
        (typing && !reaches_chooser && !name_tab).then_some(glib::Propagation::Proceed)
    }

    pub(super) fn reset_preview_zoom(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        self.preview
            .reset_preview_zoom(&context.press)
            .then_some(glib::Propagation::Stop)
    }

    pub(super) fn tenxer_key(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        self.tenxer.handle(&context.press)
    }

    pub(super) fn text_size(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        let size = context
            .preferences
            .text_size()
            .for_shortcut(&context.press)?;
        context.view().set_text_size(size);
        STOP
    }

    pub(super) fn resize_thumbnails(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        let change = ThumbnailResize::for_shortcut(&context.press)?;
        context
            .view()
            .resize_icons_thumbnails(change)
            .then_some(glib::Propagation::Stop)
    }

    pub(super) fn reference_keeps_key(
        &self,
        _context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        self.tenxer
            .reference_is_open()
            .then_some(glib::Propagation::Proceed)
    }

    pub(super) fn folder_menu_keeps_key(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        context
            .focused
            .as_ref()
            .and_then(|focused| focused.ancestor(gtk::Popover::static_type()))
            .is_some_and(|popover| popover.has_css_class("folder-context-popover"))
            .then_some(glib::Propagation::Proceed)
    }

    /// A focused Back button lists its history for the context-menu key itself.
    pub(super) fn back_history_keeps_key(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        (context.press.is(CONTEXT_MENU)
            && context
                .focused
                .as_ref()
                .is_some_and(crate::ui::browser::lists_back_history))
        .then_some(glib::Propagation::Proceed)
    }

    pub(super) fn context_menu(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(CONTEXT_MENU) || context.focus_in_text_or_popover() {
            return None;
        }
        context
            .view()
            .open_focused_context_menu()
            .then_some(glib::Propagation::Stop)
    }

    pub(super) fn video_key(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.item_view_idle() || context.focus_in_text_or_popover() {
            return None;
        }
        self.preview
            .handle_video_key(&context.press)
            .then_some(glib::Propagation::Stop)
    }

    /// Filtered rows own navigation, not the hidden directory selection.
    pub(super) fn filtered_rows_navigate(
        &self,
        context: &KeyContext<'_>,
    ) -> Option<glib::Propagation> {
        let vertical = context.press.untyped().is_any(&[
            UP.ignoring(SHORTCUT_MODIFIERS),
            DOWN.ignoring(SHORTCUT_MODIFIERS),
        ]);
        if !vertical || context.focus_in_text_or_popover() {
            return None;
        }
        context.view().selected_search_results()?;
        context.state.window.set_focus_visible(true);
        PROCEED
    }

    pub(super) fn escape(&self, context: &KeyContext<'_>) -> Option<glib::Propagation> {
        if !context.press.is(ESCAPE_ANY) {
            return None;
        }
        if let Some(popover) = context.focused_popover() {
            popover.popdown();
            return STOP;
        }
        if !self.escape_ends_editing(context) && !self.escape_closes_surface(context) {
            context.state.cancel();
        }
        STOP
    }

    fn escape_ends_editing(&self, context: &KeyContext<'_>) -> bool {
        if context.state.dismiss_dropdown() {
            return true;
        }
        if context.tenxer() && context.name_has_focus() {
            context.browser.focus_active();
            return true;
        }
        if context.view().cancel_new_entry() || context.view().cancel_rename() {
            return true;
        }
        context.view().dismiss_focused_filter()
    }

    fn escape_closes_surface(&self, context: &KeyContext<'_>) -> bool {
        if context.view().location_has_focus() {
            context.view().cancel_location_edit();
            return true;
        }
        if self.preview.is_enabled() {
            self.preview.close();
            return true;
        }
        context.state.cancel_download()
    }
}
