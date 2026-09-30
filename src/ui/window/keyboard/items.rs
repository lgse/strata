// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
    prelude::*,
};

use super::{Dispatcher, KeyEvent, KeyResult};
use crate::{
    app::{Browser, VisualKind},
    model::Location,
    ui::{
        browser_modes::BrowserMode,
        preview::preview_target,
        tenxer_mode::Chord,
        window::{
            SinglePaneArrow, home_directory, jump_direction, page_direction,
            sidebar_focus_direction, single_pane_arrow_action,
        },
    },
};

impl Dispatcher {
    pub(super) fn dismissal(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        if event.key == Key::BackSpace
            && event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK)
            && self.view.dismiss_empty_focused_filter()
        {
            return Some(Propagation::Stop);
        }
        if event.key == Key::Delete
            && !self.view.filter_has_focus()
            && !event.text_has_focus()
            && self.view.confirm_delete(event.shift())
        {
            return Some(Propagation::Stop);
        }
        if event.key == Key::Escape
            && event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            && !event.text_has_focus()
        {
            return self.dismiss_preview_or_selection(browser);
        }
        None
    }

    pub(super) fn dismiss_preview_or_selection(&self, browser: &Browser) -> KeyResult {
        if self.preview.is_enabled() {
            self.preview.close();
            browser.focus_active();
            return Some(Propagation::Stop);
        }
        // Transient surfaces may return focus to pane chrome rather than an item.
        (browser.close_peek() || browser.clear_active_selection()).then_some(Propagation::Stop)
    }

    pub(super) fn archive_navigation(&self, event: &KeyEvent) -> KeyResult {
        if !event.without(
            Modifiers::CONTROL_MASK
                | Modifiers::ALT_MASK
                | Modifiers::SUPER_MASK
                | Modifiers::SHIFT_MASK,
        ) || event.text_has_focus()
            || (!self.view.item_view_has_focus()
                && !self.preview.archive_list_has_focus(event.focused.as_ref()))
        {
            return None;
        }
        match event.key {
            Key::Up | Key::Down | Key::Left | Key::Right | Key::Return | Key::KP_Enter => self
                .preview
                .archive_key(event.key)
                .then_some(Propagation::Stop),
            Key::space => self.preview.close_archive().then_some(Propagation::Stop),
            _ => None,
        }
    }

    pub(super) fn item_navigation(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK)
            && !self.view.item_view_has_focus()
            && !event.header_left_boundary
        {
            return Some(Propagation::Proceed);
        }
        if let Some(direction) = jump_direction(event.key, event.modifiers)
            && self.view.jump_selection(direction)
        {
            return Some(Propagation::Stop);
        }
        self.single_pane_navigation(event)
            .or_else(|| self.item_commands(browser, event))
            .or_else(|| self.selection_navigation(browser, event))
            .or_else(|| self.directory_navigation(browser, event))
    }

    fn single_pane_navigation(&self, event: &KeyEvent) -> KeyResult {
        if !self.view.item_view_has_focus() {
            return None;
        }
        let action = single_pane_arrow_action(
            self.view.view_mode(),
            event.key,
            event.modifiers,
            self.view.at_left_edge(),
            self.top_bar.sidebar_toggle().is_active(),
        )?;
        let action = match action {
            SinglePaneArrow::Sidebar if self.arrows_scoped_to_content() => SinglePaneArrow::Stay,
            other => other,
        };
        Some(match action {
            SinglePaneArrow::Native => self.native_selection(event),
            SinglePaneArrow::Stay => Propagation::Stop,
            SinglePaneArrow::Sidebar => {
                self.enter_sidebar(event);
                Propagation::Stop
            }
        })
    }

    fn native_selection(&self, event: &KeyEvent) -> Propagation {
        if event.without(Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK)
            && event.key == Key::Up
            && !self.arrows_scoped_to_content()
            && self.view.focus_header_from_top_item()
        {
            return Propagation::Stop;
        }
        self.view.commit_selection();
        let started_from_empty = !event.control() && self.view.resume_native_selection();
        if event.shift() && started_from_empty {
            return Propagation::Stop;
        }
        if event.without(Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK)
            && let Some(direction) = sidebar_focus_direction(event.key)
            && self.view.cross_type_group(direction, false)
        {
            Propagation::Stop
        } else if event.vim_navigation {
            crate::ui::focus_navigation::activate_native_arrow(&self.window, event.key);
            Propagation::Stop
        } else {
            Propagation::Proceed
        }
    }

    fn item_commands(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK) {
            return None;
        }
        if self.view.item_view_has_focus() && matches!(event.key, Key::Home | Key::End) {
            self.view.commit_selection();
            if !event.shift()
                && self
                    .view
                    .jump_parked_selection(if event.key == Key::Home { -1 } else { 1 })
            {
                return Some(Propagation::Stop);
            }
        }
        if let Some(direction) = page_direction(event.key)
            && self.view.page_selection(direction, event.shift())
        {
            return Some(Propagation::Stop);
        }
        match event.key {
            Key::y | Key::Y => {
                self.view.copy_path();
            }
            Key::p | Key::P => self.view.pin_focused(),
            Key::space => {
                self.view.cancel_pending_click_rename();
                let activated_directory = event
                    .without(Modifiers::SHIFT_MASK | Modifiers::SUPER_MASK)
                    && self.view.activate_directory_on_space();
                if !activated_directory {
                    self.preview.toggle(
                        preview_target(browser.focused_entry()),
                        browser.active_depth(),
                    );
                }
            }
            Key::BackSpace => self.view.navigate_up(),
            _ => return None,
        }
        Some(Propagation::Stop)
    }

    fn selection_navigation(&self, browser: &Browser, event: &KeyEvent) -> KeyResult {
        if event.shift() {
            match event.key {
                Key::Up => browser.extend_selection(-1),
                Key::Down => browser.extend_selection(1),
                _ => return None,
            }
            return Some(Propagation::Stop);
        }
        if !event.alt()
            && matches!(event.key, Key::k | Key::Up)
            && !self.arrows_scoped_to_content()
            && self.view.focus_header_from_top_item()
        {
            return Some(Propagation::Stop);
        }
        None
    }

    fn directory_navigation(&self, browser: &Rc<Browser>, event: &KeyEvent) -> KeyResult {
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::SUPER_MASK) {
            return Some(Propagation::Proceed);
        }
        match (event.key, event.alt()) {
            (Key::Left, true) => browser.back(),
            (Key::Right, true) => browser.forward(),
            (Key::Up, true) => browser.parent(),
            (Key::Home, true) => browser.navigate(Location::local(home_directory())),
            (Key::j | Key::Down, false) => browser.move_selection(1),
            (Key::k | Key::Up, false) => browser.move_selection(-1),
            (Key::h | Key::Left, false) => self.navigate_left(event),
            (Key::Right, false) if self.view.view_mode() == BrowserMode::Columns => {
                browser.enter_focused_directory();
            }
            (Key::l | Key::Return | Key::KP_Enter, false) => self.view.activate_focused(),
            (Key::Escape, false) => browser.escape(),
            _ => return None,
        }
        Some(Propagation::Stop)
    }

    fn navigate_left(&self, event: &KeyEvent) {
        if self.view.first_column_has_focus()
            && self.top_bar.sidebar_toggle().is_active()
            && !self.arrows_scoped_to_content()
        {
            self.enter_sidebar(event);
        } else {
            self.view.navigate_left();
        }
    }

    pub(super) fn tenxer_icons(
        &self,
        browser: &Rc<Browser>,
        key: Key,
        modifiers: Modifiers,
    ) -> bool {
        if self.view.view_mode() != BrowserMode::Icons || !self.view.item_view_has_focus() {
            return false;
        }
        let search = self.view.selected_search_results().is_some();
        let mods = super::command_modifiers(modifiers);
        if search && !mods.is_empty() {
            return match mods {
                Modifiers::CONTROL_MASK => {
                    (matches!(key, Key::r | Key::R) && self.view.invert_filter_results())
                        || (self.view.listing_search_active() && self.tenxer_control_page(key))
                }
                Modifiers::SHIFT_MASK => {
                    (matches!(key, Key::G | Key::V)
                        || is_arrow(key)
                        || page_direction(key).is_some())
                        && self.tenxer_shifted(browser, key)
                }
                _ => false,
            };
        }
        if mods == Modifiers::CONTROL_MASK {
            return self.tenxer_control_page(key)
                || self.tenxer_first_or_last(key)
                || self.tenxer_selection_command(key);
        }
        if mods == Modifiers::ALT_MASK {
            return self.tenxer_alt_navigation(browser, key);
        }
        if mods == Modifiers::SHIFT_MASK {
            return self.tenxer_shifted(browser, key);
        }
        if !mods.is_empty() {
            return false;
        }
        if let Some(arrow) = crate::ui::focus_navigation::spatial_arrow(key) {
            return self.move_icon_cursor(browser, arrow, search);
        }
        match key {
            key if swallowed_on_hits(key) && self.view.listing_search_active() => {}
            Key::Home | Key::KP_Home if !search => self.jump_displayed(-1),
            Key::End | Key::KP_End | Key::G if !search => self.jump_displayed(1),
            Key::H if !search => self.go_back(browser),
            Key::L if !search => self.go_forward(browser),
            Key::BackSpace if !search => self.go_parent(),
            Key::o | Key::Return | Key::KP_Enter => self.activate_focused(browser),
            Key::i => {
                if !self.toggle_file_preview(browser) {
                    self.view.toggle_folder_peek();
                }
            }
            Key::Page_Up | Key::KP_Page_Up if !search => self.view.page_displayed_cursor(-1, false),
            Key::Page_Down | Key::KP_Page_Down if !search => {
                self.view.page_displayed_cursor(1, false)
            }
            Key::space if !self.selection_keys_blocked() => self.toggle_tenxer_cursor(),
            Key::v if !self.selection_keys_blocked() => self.toggle_visual(VisualKind::Select),
            Key::g => self.shortcuts.arm_chord(Chord::Go),
            _ => return false,
        }
        true
    }

    /// Grid motion keeps every committed fill, including a singleton and an empty
    /// inverted fill. A load cursor still follows GTK's spatial move.
    fn move_icon_cursor(&self, browser: &Rc<Browser>, arrow: Key, search: bool) -> bool {
        if !search && browser.visual_kind().is_some() {
            self.move_icon_range(browser, arrow);
            return true;
        }
        let preserved = (!search).then_some(()).and_then(|_| {
            let depth = browser.active_depth()?;
            if browser.selection_is_load_cursor() {
                return None;
            }
            let positions = browser.selected_positions(depth);
            let cursor = browser
                .focused_item()
                .filter(|(item_depth, _, _)| *item_depth == depth)
                .map(|(_, position, _)| position)?;
            Some((depth, positions, cursor))
        });
        if let Some((depth, _, cursor)) = preserved.clone() {
            browser.install_pane_fill(depth, &[cursor], cursor);
        }
        self.view.keyboard_navigation();
        if search {
            self.view.keep_result_fill(|| {
                self.view.focus_search_results();
                crate::ui::focus_navigation::activate_native_arrow(&self.window, arrow);
            });
            return true;
        }
        crate::ui::focus_navigation::activate_native_arrow(&self.window, arrow);
        if let Some((depth, positions, _)) = preserved {
            let cursor = browser
                .focused_item()
                .filter(|(item_depth, _, _)| *item_depth == depth)
                .map(|(_, position, _)| position)
                .unwrap_or(0);
            browser.install_pane_fill(depth, &positions, cursor);
        }
        true
    }

    /// GTK moves the grid cursor over a single-item selection; the walked range
    /// is then recomputed from the anchor in displayed order.
    fn move_icon_range(&self, browser: &Rc<Browser>, arrow: Key) {
        let Some((depth, cursor, _)) = browser.focused_item() else {
            return;
        };
        self.view.keyboard_navigation();
        // GTK's selection echo would otherwise end the range as a pointer change.
        let range = browser.take_visual();
        browser.install_pane_fill(depth, &[cursor], cursor);
        crate::ui::focus_navigation::activate_native_arrow(&self.window, arrow);
        browser.restore_visual(range);
        self.view.refresh_visual();
    }

    pub(super) fn tenxer_listing(
        &self,
        browser: &Rc<Browser>,
        key: Key,
        modifiers: Modifiers,
    ) -> bool {
        if self.view.view_mode() == BrowserMode::Icons || !self.view.item_view_has_focus() {
            return false;
        }
        let mods = super::command_modifiers(modifiers);
        if mods == Modifiers::CONTROL_MASK {
            return self.tenxer_control_page(key)
                || self.tenxer_first_or_last(key)
                || self.tenxer_selection_command(key);
        }
        if mods == Modifiers::ALT_MASK {
            return self.tenxer_alt_navigation(browser, key);
        }
        if mods == Modifiers::SHIFT_MASK {
            return self.tenxer_shifted(browser, key);
        }
        if !mods.is_empty() {
            return false;
        }
        self.tenxer_plain(browser, key)
    }

    fn tenxer_selection_command(&self, key: Key) -> bool {
        if matches!(key, Key::r | Key::R) && self.view.invert_filter_results() {
            return true;
        }
        if self.view.selected_search_results().is_some() {
            return false;
        }
        let filled = match key {
            Key::a | Key::A => self.view.select_focused_pane(),
            Key::r | Key::R => self.view.invert_focused_pane(),
            _ => return false,
        };
        if !filled {
            self.shortcuts.show_feedback("Nothing to select");
        }
        true
    }

    fn toggle_visual(&self, kind: VisualKind) {
        if !self.view.toggle_visual(kind) {
            self.shortcuts.show_feedback("Nothing to select");
        }
    }

    fn toggle_tenxer_cursor(&self) {
        if !self.view.toggle_cursor_and_advance() {
            self.shortcuts.show_feedback("Nothing to select");
        }
    }

    fn tenxer_first_or_last(&self, key: Key) -> bool {
        let direction = match key {
            Key::Up | Key::KP_Up => -1,
            Key::Down | Key::KP_Down => 1,
            _ => return false,
        };
        self.jump_displayed(direction);
        true
    }

    fn tenxer_control_page(&self, key: Key) -> bool {
        let (direction, half) = match key {
            Key::u | Key::U => (-1, true),
            Key::d | Key::D => (1, true),
            Key::b | Key::B | Key::Page_Up | Key::KP_Page_Up => (-1, false),
            Key::f | Key::F | Key::Page_Down | Key::KP_Page_Down => (1, false),
            _ => return false,
        };
        if !self.view.listing_search_active() {
            self.view.page_displayed_cursor(direction, half);
        }
        true
    }

    fn tenxer_alt_navigation(&self, browser: &Rc<Browser>, key: Key) -> bool {
        match key {
            Key::Left | Key::KP_Left => self.go_back(browser),
            Key::Right | Key::KP_Right => self.go_forward(browser),
            Key::Up | Key::KP_Up => self.go_parent(),
            _ => return false,
        }
        true
    }

    fn tenxer_shifted(&self, browser: &Rc<Browser>, key: Key) -> bool {
        match key {
            // Ranges come from v / V; the default map's Shift+arrow and
            // Shift+Page selection (and GTK's native one) would overlap them.
            key if is_arrow(key) || page_direction(key).is_some() => {}
            Key::V if !self.selection_keys_blocked() => {
                self.toggle_visual(VisualKind::Unset);
            }
            Key::G => self.jump_displayed(1),
            Key::H => self.go_back(browser),
            Key::L => self.go_forward(browser),
            Key::J | Key::K => self.scroll_open_preview(key),
            _ => return false,
        }
        true
    }

    fn tenxer_plain(&self, browser: &Rc<Browser>, key: Key) -> bool {
        match key {
            key if swallowed_on_hits(key) && self.view.listing_search_active() => {}
            Key::j | Key::Down | Key::KP_Down => self.view.move_displayed_cursor(1, 1),
            Key::k | Key::Up | Key::KP_Up => self.view.move_displayed_cursor(-1, 1),
            Key::Home | Key::KP_Home => self.jump_displayed(-1),
            Key::End | Key::KP_End | Key::G => self.jump_displayed(1),
            Key::H => self.go_back(browser),
            Key::L => self.go_forward(browser),
            Key::h | Key::Left | Key::KP_Left if self.view.listing_search_active() => {
                self.view.dismiss_listing_search();
            }
            Key::h | Key::Left | Key::KP_Left | Key::BackSpace => self.go_parent(),
            Key::l | Key::Right | Key::KP_Right => self.enter_preview(browser),
            Key::o | Key::Return | Key::KP_Enter => self.activate_focused(browser),
            Key::g => self.shortcuts.arm_chord(Chord::Go),
            Key::i if self.toggle_file_preview(browser) => {}
            Key::i if self.view.view_mode() == BrowserMode::Columns => {
                self.open_miller_child(browser);
            }
            Key::i => self.view.toggle_folder_peek(),
            Key::Page_Up | Key::KP_Page_Up => self.view.page_displayed_cursor(-1, false),
            Key::Page_Down | Key::KP_Page_Down => self.view.page_displayed_cursor(1, false),
            Key::space if !self.selection_keys_blocked() => self.toggle_tenxer_cursor(),
            Key::v if !self.selection_keys_blocked() => self.toggle_visual(VisualKind::Select),
            _ => return false,
        }
        true
    }

    fn jump_displayed(&self, direction: i32) {
        self.view.move_displayed_cursor(direction, usize::MAX);
    }

    fn go_parent(&self) {
        self.view.keyboard_navigation();
        self.view.navigate_up();
    }

    fn go_back(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        browser.back();
    }

    fn go_forward(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        browser.forward();
    }

    /// Opens the focused result while results replace the directory, and
    /// nothing when there is none rather than the hidden directory's cursor.
    fn activate_focused(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        if self.chooser_confirm(browser) {
            return;
        }
        if let Some(entry) = self.view.selected_search_result() {
            if entry.is_directory() {
                browser.navigate(entry.location);
            } else {
                browser.open_location(entry.location);
            }
            return;
        }
        if self.view.results_replace_listing() {
            return;
        }
        self.view.activate_focused();
    }

    fn selection_keys_blocked(&self) -> bool {
        self.view.selected_search_results().is_some() && !self.view.results_replace_listing()
    }

    /// Opens the directory under the cursor, or the directory hit, in the
    /// next column without moving focus.
    fn open_miller_child(&self, browser: &Rc<Browser>) {
        self.view.keyboard_navigation();
        if self.view.results_replace_listing() {
            if let (Some(depth), Some(entry)) =
                (browser.active_depth(), self.view.selected_search_result())
                && entry.is_directory()
            {
                self.view.open_hit_column(depth, entry.location);
            }
            return;
        }
        let Some((depth, _, entry)) = browser.focused_item() else {
            return;
        };
        if entry.is_directory() {
            browser.show_child(depth, entry.location);
        }
    }
}

fn swallowed_on_hits(key: Key) -> bool {
    matches!(
        key,
        Key::Home
            | Key::KP_Home
            | Key::End
            | Key::KP_End
            | Key::Page_Up
            | Key::KP_Page_Up
            | Key::Page_Down
            | Key::KP_Page_Down
    )
}

fn is_arrow(key: Key) -> bool {
    matches!(
        key,
        Key::Up
            | Key::KP_Up
            | Key::Down
            | Key::KP_Down
            | Key::Left
            | Key::KP_Left
            | Key::Right
            | Key::KP_Right
    )
}

pub(super) fn is_modifier_key(key: Key) -> bool {
    matches!(
        key,
        Key::Shift_L
            | Key::Shift_R
            | Key::Control_L
            | Key::Control_R
            | Key::Alt_L
            | Key::Alt_R
            | Key::Super_L
            | Key::Super_R
            | Key::Meta_L
            | Key::Meta_R
            | Key::ISO_Level3_Shift
            | Key::Caps_Lock
    )
}
