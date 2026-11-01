// SPDX-License-Identifier: MIT

use super::state::select_name_stem;
use super::*;

pub(super) fn tenxer_keys(
    state: &Rc<ChooserState>,
    sidebar: &SidebarView,
    sidebar_toggle: &gtk::ToggleButton,
    header: &gtk::Box,
    preview: &PreviewDrawer,
    footer: &ShortcutFooter,
) -> ChooserKeys {
    let confirming = Rc::downgrade(state);
    let cancelling = Rc::downgrade(state);
    let saving = Rc::downgrade(state);
    let naming = Rc::downgrade(state);
    let save_request = matches!(
        &state.request.kind,
        ChooserKind::SaveFile { .. } | ChooserKind::SaveFiles { .. }
    );
    crate::ui::window::chooser_keys(
        &state.window,
        &state.view,
        sidebar,
        TopBarNavigation::new(header, &sidebar.widget, sidebar_toggle),
        preview,
        footer,
        ChooserPolicy {
            multiple: matches!(
                &state.request.kind,
                ChooserKind::Open { multiple: true, .. }
            ),
            confirm: Rc::new(move |entry| {
                if let Some(state) = confirming.upgrade() {
                    state.confirm_file(&entry);
                }
            }),
            cancel: Rc::new(move || {
                if let Some(state) = cancelling.upgrade() {
                    state.cancel();
                }
            }),
            save: save_request.then(|| {
                Rc::new(move || {
                    if let Some(state) = saving.upgrade() {
                        state.accept();
                    }
                }) as Rc<dyn Fn()>
            }),
            edit_name: (save_request && state.filename.is_some()).then(|| {
                Rc::new(move || {
                    if let Some(state) = naming.upgrade() {
                        state.edit_name();
                    }
                }) as Rc<dyn Fn()>
            }),
        },
    )
}

pub(super) fn install_shortcuts(
    window: &gtk::Window,
    state: &Rc<ChooserState>,
    sidebar: &SidebarView,
    sidebar_toggle: &gtk::ToggleButton,
    header: &gtk::Widget,
    preview: &PreviewDrawer,
    tenxer: ChooserKeys,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    let sidebar_state = sidebar.state.clone();
    let sidebar_widget = sidebar.widget.clone();
    let sidebar_toggle = sidebar_toggle.clone();
    let header = header.clone();
    let preview = preview.clone();
    let focus_before_sidebar = Rc::new(RefCell::new(None::<gtk::Widget>));
    let handle = move |state: &Rc<ChooserState>, press: &KeyPress| -> glib::Propagation {
        let preferences = PreferenceManager::shared();
        if let Some(layer) = visible_modal_layer(&state.window) {
            let focused = gtk::prelude::RootExt::focus(&state.window);
            if !focused.is_some_and(|focus| focus == layer || focus.is_ancestor(&layer)) {
                layer.grab_focus();
            }
            return glib::Propagation::Proceed;
        }
        if press.is(ESCAPE_ANY) && state.view.cancel_column_drag() {
            return glib::Propagation::Stop;
        }
        let focused = gtk::prelude::RootExt::focus(&state.window);
        if preferences.tenxer_mode()
            && (state.name_has_focus(focused.as_ref()) || state.view.location_has_focus())
            && !(press.is(REFERENCE.in_map(Map::Default).ignoring(SHORTCUT_MODIFIERS))
                || press.is(ESCAPE_ANY)
                || press.is(TENXER_MODE))
        {
            return glib::Propagation::Proceed;
        }
        if preview.reset_pdf_zoom(press) {
            return glib::Propagation::Stop;
        }
        if let Some(result) = tenxer.handle(press) {
            return result;
        }
        if let Some(size) = preferences.text_size().for_shortcut(press) {
            state.view.set_text_size(size);
            return glib::Propagation::Stop;
        }
        if let Some(change) = ThumbnailResize::for_shortcut(press)
            && state.view.resize_icons_thumbnails(change)
        {
            return glib::Propagation::Stop;
        }
        if tenxer.reference_is_open() {
            return glib::Propagation::Proceed;
        }
        let browser = state.view.browser();
        let held = press.modifiers();
        let control = held.contains(ModifierType::CONTROL_MASK);
        let alt = held.contains(ModifierType::ALT_MASK);
        if focused
            .as_ref()
            .and_then(|focused| focused.ancestor(gtk::Popover::static_type()))
            .is_some_and(|popover| popover.has_css_class("folder-context-popover"))
        {
            return glib::Propagation::Proceed;
        }
        if press.is(CONTEXT_MENU)
            && !focused.as_ref().is_some_and(|widget| {
                crate::ui::focus_navigation::editable(widget)
                    || crate::ui::focus_navigation::in_popover(widget)
            })
            && state.view.open_focused_context_menu()
        {
            return glib::Propagation::Stop;
        }
        if state.view.item_view_has_focus()
            && !state.view.new_entry_is_active()
            && !state.view.rename_is_active()
            && !focused.as_ref().is_some_and(|widget| {
                crate::ui::focus_navigation::editable(widget)
                    || crate::ui::focus_navigation::in_popover(widget)
            })
            && preview.handle_video_key(press)
        {
            return glib::Propagation::Stop;
        }
        // Filtered rows own navigation, not the hidden directory selection.
        if press.untyped().is_any(&[
            UP.ignoring(SHORTCUT_MODIFIERS),
            DOWN.ignoring(SHORTCUT_MODIFIERS),
        ]) && state.view.selected_search_results().is_some()
            && !focused.as_ref().is_some_and(|widget| {
                crate::ui::focus_navigation::editable(widget)
                    || crate::ui::focus_navigation::in_popover(widget)
            })
        {
            state.window.set_focus_visible(true);
            return glib::Propagation::Proceed;
        }
        // Unlike the window, the chooser types h j k l whenever type-to-search
        // is on, 10xer mode included.
        let press = &if preferences.type_to_search() {
            press.untyped()
        } else {
            *press
        };
        let vim_navigation = press.aliased();
        if !focused
            .as_ref()
            .is_some_and(crate::ui::focus_navigation::in_popover)
            && (press.is(SIDEBAR_FOCUS)
                || (!focused
                    .as_ref()
                    .is_some_and(crate::ui::focus_navigation::editable)
                    && crate::ui::window::is_browser_navigation_key(press)))
        {
            state.view.keyboard_navigation();
        }
        if arrow_direction(press, ModifierType::empty()).is_some() {
            state.window.set_focus_visible(true);
            if focused
                .as_ref()
                .is_none_or(|widget| !widget.is_mapped() || !widget.is_sensitive())
            {
                if preferences.tenxer_mode() {
                    browser.focus_active();
                } else if let Some(filename) = state.filename.as_ref() {
                    filename.grab_focus();
                } else {
                    sidebar_toggle.grab_focus();
                }
                return glib::Propagation::Stop;
            }
        }
        let sidebar_has_focus = focused.as_ref().is_some_and(|focused| {
            focused == &sidebar_widget || focused.is_ancestor(&sidebar_widget)
        });
        if press.is(ESCAPE_ANY) {
            if let Some(popover) = focused
                .as_ref()
                .and_then(|widget| widget.ancestor(gtk::Popover::static_type()))
                .and_downcast::<gtk::Popover>()
            {
                popover.popdown();
                return glib::Propagation::Stop;
            }
            if state.dismiss_dropdown() {
                return glib::Propagation::Stop;
            }
            if preferences.tenxer_mode() && state.name_has_focus(focused.as_ref()) {
                browser.focus_active();
                return glib::Propagation::Stop;
            }
            if state.view.cancel_new_entry() || state.view.cancel_rename() {
                return glib::Propagation::Stop;
            }
            if state.view.dismiss_focused_filter() {
                return glib::Propagation::Stop;
            }
            if state.view.location_has_focus() {
                state.view.cancel_location_edit();
                return glib::Propagation::Stop;
            }
            if preview.is_enabled() {
                preview.close();
                return glib::Propagation::Stop;
            }
            if state.cancel_download() {
                return glib::Propagation::Stop;
            }
            state.cancel();
            return glib::Propagation::Stop;
        }
        if press.is(RENAME.in_map(Map::Default))
            && browser.selected_entries().len() == 1
            && !focused.as_ref().is_some_and(|widget| {
                crate::ui::focus_navigation::editable(widget)
                    || crate::ui::focus_navigation::in_popover(widget)
            })
            && state.view.begin_rename()
        {
            return glib::Propagation::Stop;
        }
        if press.is(PROPERTIES.ignoring(ModifierType::SUPER_MASK))
            && state.view.item_view_has_focus()
            && !state.view.new_entry_is_active()
            && !state.view.rename_is_active()
            && state.view.show_focused_properties()
        {
            return glib::Propagation::Stop;
        }
        if state.view.new_entry_is_active() || state.view.rename_is_active() {
            return glib::Propagation::Proceed;
        }
        if press.is(PREVIEW.in_map(Map::Default))
            && let Some(entry) = state.view.search_result_for_space()
        {
            if state.view.activate_directory_on_space() {
                return glib::Propagation::Stop;
            }
            preview.toggle(
                preview_target(Some(entry)),
                state.view.browser().active_depth(),
            );
            return glib::Propagation::Stop;
        }
        if press.is(FILTER.in_map(Map::Default)) && state.view.show_filter() {
            return glib::Propagation::Stop;
        }
        if press.is(LOCATION) {
            state.view.begin_location_edit();
            return glib::Propagation::Stop;
        }
        if press.is(SIDEBAR_FOCUS) {
            if preferences.tenxer_mode() {
                if !sidebar_toggle.is_active() {
                    return glib::Propagation::Stop;
                }
                if sidebar_has_focus {
                    let restored = focus_before_sidebar
                        .borrow_mut()
                        .take()
                        .is_some_and(|widget| widget.is_mapped() && widget.grab_focus());
                    if !restored {
                        browser.focus_active();
                    }
                } else {
                    focus_before_sidebar.replace(focused.clone());
                    sidebar_state.focus_active_place();
                    state.window.set_focus_visible(true);
                }
                return glib::Propagation::Stop;
            }
            if sidebar_has_focus {
                let restored = focus_before_sidebar
                    .borrow_mut()
                    .take()
                    .is_some_and(|widget| widget.grab_focus());
                if !restored {
                    browser.focus_active();
                }
            } else {
                focus_before_sidebar.replace(focused.clone());
                if !sidebar_toggle.is_active() {
                    sidebar_toggle.set_active(true);
                }
                let sidebar = sidebar_state.clone();
                glib::idle_add_local_once(move || {
                    sidebar.focus_active_place();
                });
            }
            return glib::Propagation::Stop;
        }
        if press.is(SIDEBAR) {
            if state
                .destination_host
                .as_ref()
                .is_none_or(|host| !host.confined)
            {
                sidebar_toggle.emit_clicked();
            }
            return glib::Propagation::Stop;
        }
        if state.view.location_has_focus() {
            return glib::Propagation::Proceed;
        }
        if press.is(CHOOSE_FOLDER.ignoring(ModifierType::SUPER_MASK))
            && state.view.item_view_has_focus()
            && matches!(
                &state.request.kind,
                ChooserKind::Open {
                    directory: true,
                    ..
                }
            )
        {
            state.accept();
            return glib::Propagation::Stop;
        }
        if press.is(NEW_FOLDER) {
            if state
                .destination_host
                .as_ref()
                .is_none_or(|host| host.allow_create)
            {
                state.view.create_new_folder();
            }
            return glib::Propagation::Stop;
        }
        if press.is(SELECT_ALL)
            && state.view.item_view_has_focus()
            && matches!(
                &state.request.kind,
                ChooserKind::Open { multiple: true, .. }
            )
        {
            state.view.select_all();
            return glib::Propagation::Stop;
        }
        if press.is(REFRESH.ignoring(SHORTCUT_MODIFIERS)) {
            if let Some(depth) = browser.active_depth() {
                crate::adapters::trash_support::forget_all();
                browser.retry_column(depth);
            }
            return glib::Propagation::Stop;
        }
        if press.is(HIDDEN_FILES.in_map(Map::Default)) {
            browser.toggle_hidden();
            return glib::Propagation::Stop;
        }
        let permanent = if press.is(MOVE_TO_TRASH
            .in_map(Map::Default)
            .ignoring(ModifierType::SUPER_MASK))
        {
            Some(false)
        } else if press.is(DELETE_PERMANENTLY
            .in_map(Map::Default)
            .ignoring(ModifierType::SUPER_MASK))
        {
            Some(true)
        } else {
            None
        };
        if let Some(permanent) = permanent
            && state.view.item_view_has_focus()
            && !state.view.filter_has_focus()
            && state.view.confirm_delete(permanent)
        {
            return glib::Propagation::Stop;
        }
        if let Some(mode) = crate::ui::window::browser_mode_for_digit(press) {
            crate::ui::window::apply_browser_mode(&state.view, &PreferenceManager::shared(), mode);
            return glib::Propagation::Stop;
        }
        // A user's binding for a folder jump may hold Ctrl, as in the window.
        let jumps = press.is_any(&[
            HISTORY_BACK.in_map(Map::Default),
            HISTORY_FORWARD.in_map(Map::Default),
            PARENT_FOLDER.into(),
            HOME_FOLDER.into(),
        ]);
        if control && !jumps {
            return glib::Propagation::Proceed;
        }
        let column_popover = focused
            .as_ref()
            .and_then(|focused| focused.ancestor(gtk::Popover::static_type()))
            .and_downcast::<gtk::Popover>()
            .filter(|popover| popover.has_css_class("column-popover"));
        if let Some(popover) = column_popover
            && let Some(direction) = vim_focus_direction(press)
        {
            popover.child_focus(direction);
            return glib::Propagation::Stop;
        }
        let tab = plain_tab_direction(press);
        if !preferences.tenxer_mode()
            && let Some(direction) = tab
            && !focused
                .as_ref()
                .is_some_and(crate::ui::focus_navigation::in_popover)
            && state.view.default_tab(direction)
        {
            return glib::Propagation::Stop;
        }
        if preferences.tenxer_mode()
            && tab.is_some()
            && let Some(filename) = state.filename.as_ref()
        {
            let in_header = focused
                .as_ref()
                .is_some_and(|focused| focused == &header || focused.is_ancestor(&header));
            if state.name_has_focus(focused.as_ref()) {
                browser.focus_active();
                return glib::Propagation::Stop;
            }
            if in_header {
                filename.grab_focus_without_selecting();
                select_name_stem(filename);
                return glib::Propagation::Stop;
            }
        }
        if !alt && let Some(focused) = focused.as_ref() {
            if crate::ui::focus_navigation::in_popover(focused) {
                return glib::Propagation::Proceed;
            }
            if press.is(ENTER.ignoring(ModifierType::SUPER_MASK))
                && crate::ui::focus_navigation::activate(state.window.upcast_ref())
            {
                return glib::Propagation::Stop;
            }
            if crate::ui::focus_navigation::editable(focused) {
                return glib::Propagation::Proceed;
            }
        }
        if preferences.tenxer_mode()
            && state.view.item_view_has_focus()
            && tab == Some(gtk::DirectionType::TabForward)
        {
            if header.child_focus(gtk::DirectionType::TabForward) {
                state.window.set_focus_visible(true);
            }
            return glib::Propagation::Stop;
        }
        if preferences.tenxer_mode() && tab.is_some() {
            if !crate::ui::focus_navigation::contains_widget(&state.view.widget(), focused.as_ref())
            {
                browser.focus_active();
            }
            return glib::Propagation::Stop;
        }
        let mut header_left_boundary = false;
        if state.view.header_actions_have_focus() {
            if press.is(LEFT.ignoring(FOCUS_MODIFIERS).or_alternates()) {
                if state.view.move_header_focus(gtk::DirectionType::Left)
                    || preferences.tenxer_mode()
                {
                    return glib::Propagation::Stop;
                }
                header_left_boundary = true;
            } else if press.is(RIGHT.ignoring(FOCUS_MODIFIERS).or_alternates()) {
                state.view.move_header_focus(gtk::DirectionType::Right);
                return glib::Propagation::Stop;
            } else if press.is(DOWN.ignoring(FOCUS_MODIFIERS).or_alternates()) {
                state.view.focus_items_from_header();
                return glib::Propagation::Stop;
            }
        }
        if sidebar_has_focus
            && !preferences.tenxer_mode()
            && let Some(direction) =
                vim_focus_direction(press).or_else(|| arrow_direction(press, FOCUS_MODIFIERS))
        {
            if direction == gtk::DirectionType::Right {
                focus_before_sidebar.borrow_mut().take();
                browser.focus_active();
            } else if !sidebar_widget.child_focus(direction) && direction == gtk::DirectionType::Up
            {
                sidebar_toggle.grab_focus();
                state.window.set_focus_visible(true);
            }
            return glib::Propagation::Stop;
        }
        if press.is(BACKSPACE.ignoring(FOCUS_MODIFIERS))
            && state.view.dismiss_empty_focused_filter()
        {
            return glib::Propagation::Stop;
        }
        if !alt && !state.view.item_view_has_focus() && !header_left_boundary {
            if preferences.tenxer_mode()
                && (arrow_direction(press, FOCUS_MODIFIERS).is_some()
                    || vim_focus_direction(press).is_some())
            {
                browser.focus_active();
                if vim_focus_direction(press).is_some() {
                    return glib::Propagation::Stop;
                }
            } else if let Some(direction) = arrow_direction(press, ModifierType::SUPER_MASK) {
                if direction == gtk::DirectionType::Up
                    && focused.as_ref().is_some_and(|focused| {
                        let mut widget = Some(focused.clone());
                        while let Some(current) = widget {
                            if current.has_css_class("chooser-options") {
                                return true;
                            }
                            widget = current.parent();
                        }
                        false
                    })
                {
                    browser.focus_active();
                    return glib::Propagation::Stop;
                }
                if crate::ui::focus_navigation::move_focus(state.window.upcast_ref(), direction) {
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            } else {
                return glib::Propagation::Proceed;
            }
        }
        if press.is(LEFT.ignoring(ModifierType::SUPER_MASK))
            && sidebar_toggle.is_active()
            && state.view.item_view_has_focus()
            && state.view.item_at_sidebar_edge()
            && !PreferenceManager::shared().arrow_navigation_scoped_active()
        {
            focus_before_sidebar.replace(focused.clone());
            sidebar_state.focus_active_place();
            state.window.set_focus_visible(true);
            return glib::Propagation::Stop;
        }
        if press.is(PREVIEW.in_map(Map::Default).ignoring(FOCUS_MODIFIERS)) {
            if press.is(PREVIEW.in_map(Map::Default)) && state.view.activate_directory_on_space() {
                return glib::Propagation::Stop;
            }
            preview.toggle(
                preview_target(browser.focused_entry()),
                browser.active_depth(),
            );
            return glib::Propagation::Stop;
        }
        if press.is(BACKSPACE.ignoring(FOCUS_MODIFIERS)) {
            state.view.navigate_up();
            return glib::Propagation::Stop;
        }
        let multiple = matches!(
            &state.request.kind,
            ChooserKind::Open { multiple: true, .. }
        );
        if !alt
            && state.view.view_mode() != crate::ui::browser_modes::BrowserMode::Columns
            && let Some(direction) = arrow_direction(press, FOCUS_MODIFIERS)
        {
            let extend = held.contains(ModifierType::SHIFT_MASK) && multiple;
            if state.view.cross_type_group(direction, extend) {
                return glib::Propagation::Stop;
            }
            if press.is(UP.ignoring(ModifierType::SUPER_MASK))
                && !PreferenceManager::shared().arrow_navigation_scoped_active()
                && state.view.focus_header_from_top_item()
            {
                return glib::Propagation::Stop;
            }
            // Keep GTK's spatial movement, then reconcile selection in visual order.
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
            if vim_navigation && let Some(arrow) = crate::ui::focus_navigation::spatial_arrow(press)
            {
                crate::ui::focus_navigation::activate_native_arrow(&state.window, arrow.key());
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        let alt_super = ModifierType::ALT_MASK.union(ModifierType::SUPER_MASK);
        if multiple && press.is(EXTEND_UP.ignoring(alt_super)) {
            browser.extend_selection(-1);
            return glib::Propagation::Stop;
        }
        if multiple && press.is(EXTEND_DOWN.ignoring(alt_super)) {
            browser.extend_selection(1);
            return glib::Propagation::Stop;
        }
        if state.view.item_view_has_focus()
            && let Some(query) = crate::ui::window::type_to_search_query(press)
            && preferences.type_to_search()
            && match query {
                crate::ui::window::TypeToSearchQuery::Empty => state.view.show_filter(),
                crate::ui::window::TypeToSearchQuery::Character(character) => {
                    state.view.show_filter_with_query(&character.to_string())
                }
            }
        {
            return glib::Propagation::Stop;
        }
        if press.is(UP.ignoring(alt_super).or_alternates())
            && !PreferenceManager::shared().arrow_navigation_scoped_active()
            && state.view.focus_header_from_top_item()
        {
            return glib::Propagation::Stop;
        }
        if press.is(HISTORY_BACK.in_map(Map::Default).ignoring(FOCUS_MODIFIERS)) {
            browser.back();
        } else if press.is(HISTORY_FORWARD
            .in_map(Map::Default)
            .ignoring(FOCUS_MODIFIERS))
        {
            browser.forward();
        } else if press.is(PARENT_FOLDER.ignoring(FOCUS_MODIFIERS)) {
            browser.parent();
        } else if press.is(HOME_FOLDER.ignoring(FOCUS_MODIFIERS)) {
            browser.navigate(Location::local(home_directory()));
        } else if press.is(DOWN.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            browser.move_selection(1);
        } else if press.is(UP.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            browser.move_selection(-1);
        } else if press.is(LEFT.ignoring(FOCUS_MODIFIERS).or_alternates()) {
            if state.view.first_column_has_focus()
                && sidebar_toggle.is_active()
                && !PreferenceManager::shared().arrow_navigation_scoped_active()
            {
                focus_before_sidebar.replace(focused.clone());
                sidebar_state.focus_active_place();
            } else {
                state.view.navigate_left();
            }
        } else if press.is(RIGHT.ignoring(FOCUS_MODIFIERS))
            && state.view.view_mode() == BrowserMode::Columns
        {
            browser.enter_focused_directory();
        } else if press.is_any(&[RIGHT.alternates(), ENTER.ignoring(FOCUS_MODIFIERS)]) {
            if !tenxer.confirm_focused() {
                state.view.activate_focused();
            }
        } else {
            return glib::Propagation::Proceed;
        }
        glib::Propagation::Stop
    };
    keys::on_press(&keys, move |_, press| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match handle(&state, press) {
            glib::Propagation::Proceed => edit_focused_text(&state.window, press),
            handled => handled,
        }
    });
    window.add_controller(keys.clone());
    let keys = keys.downgrade();
    window.connect_unrealize(move |window| {
        if let Some(keys) = keys.upgrade() {
            window.remove_controller(&keys);
        }
    });
}
