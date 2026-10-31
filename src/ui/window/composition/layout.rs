// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use crate::{
    adapters::LocalPreviewProvider,
    assets::{self, icons},
    ui::{
        browser::BrowserView,
        marquee::OriginClick,
        preferences::PreferenceManager,
        preview::PreviewDrawer,
        shortcut_footer::ShortcutFooter,
        sidebar_toggle::{SidebarMode, SidebarToggle},
    },
};

use super::super::{
    MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH, SidebarView, animate_sidebar, build_appearance_menu,
    pin_status, sidebar_rail_width,
};

pub(super) struct Header {
    widget: gtk::HeaderBar,
    pub(super) actions: gtk::Box,
    pub(super) new_tab: gtk::Button,
    pub(super) content: gtk::Box,
    pub(super) sidebar_toggle: SidebarToggle,
    pub(super) search: gtk::Button,
    pub(super) settings: gtk::Button,
    #[cfg(test)]
    pub(super) minimize: gtk::Button,
    #[cfg(test)]
    pub(super) maximize: gtk::Button,
    pub(super) close: gtk::Button,
}

impl Header {
    pub(super) fn new(
        window: &gtk::ApplicationWindow,
        browser: &BrowserView,
        preview: &PreviewDrawer,
        preferences: &Rc<PreferenceManager>,
    ) -> Self {
        let widget = gtk::HeaderBar::new();
        widget.add_css_class("browser-header");
        widget.set_show_title_buttons(false);
        let sidebar_toggle = SidebarToggle::new();
        {
            let toggle = sidebar_toggle.downgrade();
            preferences.bind_preference(
                &sidebar_toggle,
                PreferenceManager::sidebar_expanded,
                move |_, expanded| {
                    if let Some(toggle) = toggle.upgrade() {
                        toggle.set_active(expanded);
                    }
                },
            );
        }
        {
            let preferences = preferences.clone();
            sidebar_toggle.connect_toggled(move |toggle| {
                preferences.set_sidebar_expanded(toggle.is_active());
            });
        }
        let location = browser.location_widget();
        location.set_hexpand(true);
        let search = header_action(icons::SEARCH, "");
        let search_keys = crate::ui::keys::Keys::One(crate::ui::keys::bindings::SEARCH.shown());
        crate::ui::shortcut_reference::follow_tooltip(&search, "Search (%{keys})", search_keys);
        crate::ui::shortcut_reference::follow_label(&search, "Search (%{keys})", search_keys);
        crate::ui::tenxer_mode::hide_while_enabled(&search);
        let appearance =
            build_appearance_menu(browser, &browser.browser(), preferences.clone(), preview);
        let settings_label = crate::i18n::tr("Settings");
        let settings = header_action(icons::SETTINGS, &settings_label);
        crate::ui::accessibility::set_label(&settings, &settings_label);
        let minimize_label = crate::i18n::tr("Minimize window");
        let minimize = header_action(icons::MINUS, &minimize_label);
        crate::ui::accessibility::set_label(&minimize, &minimize_label);
        let minimizing_window = window.downgrade();
        minimize.connect_clicked(move |_| {
            if let Some(window) = minimizing_window.upgrade() {
                window.minimize();
            }
        });
        let maximize = header_action(icons::MAXIMIZE, &crate::i18n::tr("Maximize window"));
        let maximizing_window = window.downgrade();
        maximize.connect_clicked(move |_| {
            let Some(window) = maximizing_window.upgrade() else {
                return;
            };
            if window.is_maximized() {
                window.unmaximize();
            } else {
                window.maximize();
            }
        });
        bind_maximize_icon(window, &maximize);
        let close_label = crate::i18n::tr("Close window");
        let close = header_action(icons::X, &close_label);
        crate::ui::accessibility::set_label(&close, &close_label);
        let closing_window = window.downgrade();
        close.connect_clicked(move |_| {
            if let Some(window) = closing_window.upgrade() {
                window.close();
            }
        });
        for (button, read) in [
            (
                &minimize,
                PreferenceManager::window_show_minimize as fn(&PreferenceManager) -> bool,
            ),
            (&maximize, PreferenceManager::window_show_maximize),
            (&close, PreferenceManager::window_show_close),
        ] {
            preferences
                .bind_preference(button, read, |widget, visible| widget.set_visible(visible));
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions.add_css_class("header-actions");
        let new_tab = header_action(icons::PLUS, "");
        crate::ui::shortcut_reference::follow_tooltip(
            &new_tab,
            "New tab (%{keys})",
            crate::ui::keys::Keys::One(crate::ui::keys::bindings::TAB_NEW.shown()),
        );
        crate::ui::accessibility::set_label(&new_tab, &crate::i18n::tr("New tab"));
        actions.append(&new_tab);
        actions.append(&search);
        actions.append(&appearance);
        actions.append(&settings);
        actions.append(&minimize);
        actions.append(&maximize);
        actions.append(&close);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.set_hexpand(true);
        content.set_valign(gtk::Align::Center);
        content.append(&sidebar_toggle);
        content.append(&location);
        content.append(&actions);
        widget.set_title_widget(Some(&content));
        Self {
            widget,
            content,
            sidebar_toggle,
            search,
            settings,
            #[cfg(test)]
            minimize,
            #[cfg(test)]
            maximize,
            close,
            actions,
            new_tab,
        }
    }
}

fn bind_maximize_icon(window: &gtk::ApplicationWindow, button: &gtk::Button) {
    let weak_button = button.downgrade();
    let update = move |window: &gtk::ApplicationWindow| {
        let Some(image) = weak_button
            .upgrade()
            .and_then(|button| button.child())
            .and_downcast::<gtk::Image>()
        else {
            return;
        };
        let (icon, tooltip) = if window.is_maximized() {
            (icons::MINIMIZE, crate::i18n::tr("Restore window"))
        } else {
            (icons::MAXIMIZE, crate::i18n::tr("Maximize window"))
        };
        assets::set_primary_icon(&image, icon);
        if let Some(button) = weak_button.upgrade() {
            button.set_tooltip_text(Some(&tooltip));
            crate::ui::accessibility::set_label(&button, &tooltip);
        }
    };
    update(window);
    window.connect_maximized_notify(move |window| update(window));
}

pub(super) fn header_action(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&assets::chrome_icon(icon)));
    button.add_css_class("header-action");
    button.set_cursor_from_name(Some("pointer"));
    button
}

pub(super) fn preview(browser: &BrowserView, preferences: &Rc<PreferenceManager>) -> PreviewDrawer {
    let backend = preferences.clone();
    let preview = PreviewDrawer::new(
        Rc::new(LocalPreviewProvider::new(Rc::new(move || {
            backend.media_preview_backend()
        }))),
        true,
    );
    preview.bind_find_shortcut(preferences);
    preview.observe_browser(&browser.browser());
    preview
}

pub(super) fn browser_layout(
    browser: &BrowserView,
    preview: &PreviewDrawer,
    sidebar: &SidebarView,
    header: &Header,
    preferences: &Rc<PreferenceManager>,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header.widget);
    bind_pin_handlers(browser, sidebar);
    let content = browser_split(
        browser,
        sidebar,
        &header.sidebar_toggle,
        preview,
        preferences,
    );
    let preview_split = gtk::Paned::new(gtk::Orientation::Horizontal);
    preview_split.add_css_class("preview-split");
    // The column beside the divider keeps its resize edge; the preview grip moves it.
    preview_split.set_wide_handle(true);
    preview_split.set_resize_start_child(true);
    preview_split.set_resize_end_child(false);
    preview_split.set_shrink_start_child(false);
    preview_split.set_shrink_end_child(true);
    preview_split.set_start_child(Some(&content));
    preview_split.set_end_child(Some(&preview.widget()));
    preview_split.set_position(i32::MAX);
    preview_split.set_vexpand(true);
    preview.attach_split(&preview_split, &content, browser, Some(sidebar));
    browser.add_marquee_origin(
        &preview.widget(),
        gtk::PackType::End,
        OriginClick::keeps_selection_within(&preview.pane_widget()),
    );
    root.append(&preview_split);
    root
}

fn bind_pin_handlers(browser: &BrowserView, sidebar: &SidebarView) {
    let weak_pin_sidebar = Rc::downgrade(&sidebar.state);
    let weak_unpin_sidebar = Rc::downgrade(&sidebar.state);
    let pinned_places = sidebar.state.pinned_places.clone();
    browser.set_pin_handlers(
        Rc::new(move |location, name, done| {
            if let Some(sidebar) = weak_pin_sidebar.upgrade() {
                sidebar.pin_location_with(location, name, done);
            } else {
                done(false);
            }
        }),
        Rc::new(move |location| {
            if let Some(sidebar) = weak_unpin_sidebar.upgrade() {
                sidebar.unpin_location(location);
            }
        }),
        Rc::new(move |location| pin_status(&pinned_places.borrow(), location)),
    );
}

fn browser_split(
    browser: &BrowserView,
    sidebar: &SidebarView,
    toggle: &SidebarToggle,
    preview: &PreviewDrawer,
    preferences: &Rc<PreferenceManager>,
) -> gtk::Paned {
    let content = gtk::Paned::new(gtk::Orientation::Horizontal);
    content.add_css_class("sidebar-split");
    // Wide handles keep GTK's mouse hit area inside the divider allocation.
    content.set_wide_handle(true);
    content.set_shrink_start_child(false);
    content.set_resize_start_child(false);
    content.set_position(SIDEBAR_WIDTH);
    content.set_vexpand(true);
    sidebar.widget.set_size_request(MIN_SIDEBAR_WIDTH, -1);
    browser.add_marquee_origin(
        &sidebar.widget,
        gtk::PackType::Start,
        OriginClick::ClearsSelection,
    );
    content.set_start_child(Some(&sidebar.widget));
    content.set_end_child(Some(&browser.widget()));
    bind_sidebar_toggle(&content, sidebar, toggle, preview);
    bind_sidebar_modes(sidebar, toggle, preferences);
    super::super::bind_sidebar_text_size(&content, sidebar);
    content
}

/// The toggle steps this window's sidebar through full, rail, and hidden and
/// saves each step, so every window follows. Showing it asks for the full
/// sidebar even where space railed it automatically.
fn bind_sidebar_modes(
    sidebar: &SidebarView,
    toggle: &SidebarToggle,
    preferences: &Rc<PreferenceManager>,
) {
    let state = Rc::downgrade(&sidebar.state);
    preferences.bind_preference(
        &sidebar.widget,
        PreferenceManager::sidebar_compact,
        move |_, compact| {
            if let Some(state) = state.upgrade() {
                state.set_compact(compact);
            }
        },
    );
    let state = Rc::downgrade(&sidebar.state);
    let stepping_toggle = toggle.clone();
    let stepping_preferences = preferences.clone();
    toggle.set_step(move || {
        let Some(state) = state.upgrade() else {
            return;
        };
        let mode = SidebarMode::of(stepping_toggle.is_active(), state.rail.get());
        match mode.next() {
            SidebarMode::Expanded => {
                state.set_compact(false);
                stepping_preferences.set_sidebar_compact(false);
                stepping_preferences.set_sidebar_expanded(true);
            }
            SidebarMode::Compact => stepping_preferences.set_sidebar_compact(true),
            SidebarMode::Hidden => stepping_preferences.set_sidebar_expanded(false),
        }
    });
    let describe = {
        let toggle = toggle.clone();
        let state = Rc::downgrade(&sidebar.state);
        Rc::new(move || {
            if let Some(state) = state.upgrade() {
                toggle.describe(SidebarMode::of(toggle.is_active(), state.rail.get()));
            }
        })
    };
    let on_toggled = describe.clone();
    toggle.connect_toggled(move |_| on_toggled());
    let on_rail = describe.clone();
    sidebar.state.connect_rail_changed(move |_| on_rail());
    preferences.bind_preference(toggle, PreferenceManager::shown_keys, move |_, _| {
        describe();
    });
}

fn bind_sidebar_toggle(
    content: &gtk::Paned,
    sidebar: &SidebarView,
    toggle: &SidebarToggle,
    preview: &PreviewDrawer,
) {
    let generation = Rc::new(Cell::new(0));
    let animating = Rc::new(Cell::new(false));
    let constrained_toggle = toggle.clone();
    let constrained_animation = animating.clone();
    let constrained_sidebar = Rc::downgrade(&sidebar.state);
    content.connect_position_notify(move |content| {
        if !constrained_toggle.is_active() || constrained_animation.get() {
            return;
        }
        let Some(state) = constrained_sidebar.upgrade() else {
            return;
        };
        let railed = state.rail.get();
        if railed {
            if content.position() != sidebar_rail_width() {
                content.set_position(sidebar_rail_width());
            }
        } else if content.position() < MIN_SIDEBAR_WIDTH {
            content.set_position(MIN_SIDEBAR_WIDTH);
        } else if content.position()
            + crate::ui::preview::separator_width(content)
            + crate::ui::browser::COLUMN_WIDTH
            < content.width()
        {
            // A divider pinned by a narrow window is not the user's choice.
            state.saved_width.set(Some(content.position()));
        }
    });
    let content = content.clone();
    let sidebar_widget = sidebar.widget.clone();
    let toggled_sidebar = Rc::downgrade(&sidebar.state);
    let initial_content = content.clone();
    let initial_sidebar = sidebar_widget.clone();
    let toggled_preview = preview.clone();
    toggle.connect_toggled(move |toggle| {
        let open = toggle.is_active();
        let Some(state) = toggled_sidebar.upgrade() else {
            return;
        };
        // The same rule the preview applies each frame, so the sidebar opens
        // straight into the rail instead of expanding and snapping back.
        if open
            && let Some(fit) = toggled_preview.sidebar_fit(&content)
            && let Some(rail) = state.observe_fit(fit)
        {
            state.set_rail(rail);
        }
        animate_sidebar(
            &content,
            &sidebar_widget,
            &state,
            &generation,
            &animating,
            open,
        );
    });
    if !toggle.is_active() {
        initial_content.set_position(0);
        initial_sidebar.set_visible(false);
        initial_content.set_shrink_start_child(true);
    }
}

pub(super) struct FooterBinding {
    pub(super) shortcuts: ShortcutFooter,
    clipboard: gdk::Clipboard,
    clipboard_handler: RefCell<Option<glib::SignalHandlerId>>,
}

impl FooterBinding {
    pub(super) fn new(
        window: &gtk::ApplicationWindow,
        root: &gtk::Box,
        browser: &BrowserView,
        preferences: &PreferenceManager,
    ) -> Self {
        let shortcuts = ShortcutFooter::new(browser.view_mode());
        shortcuts.bind_preferences(preferences);
        shortcuts.observe_browser(&browser.browser());
        let jobs = crate::ui::jobs::JobsIndicator::new();
        jobs.bind_window(window);
        shortcuts.set_activity(jobs.widget());
        let clipboard = window.clipboard();
        let clipboard_handler = RefCell::new(Some(shortcuts.connect_clipboard(&clipboard)));
        root.append(shortcuts.widget());
        let updated_shortcuts = shortcuts.clone();
        browser.connect_view_mode_changed(move |mode| updated_shortcuts.set_mode(mode));
        Self {
            shortcuts,
            clipboard,
            clipboard_handler,
        }
    }

    pub(super) fn disconnect_clipboard(&self) {
        if let Some(handler) = self.clipboard_handler.borrow_mut().take() {
            self.clipboard.disconnect(handler);
        }
    }
}
