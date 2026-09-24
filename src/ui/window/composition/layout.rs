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
        browser::BrowserView, preferences::PreferenceManager, preview::PreviewDrawer,
        shortcut_footer::ShortcutFooter,
    },
};

use super::super::{
    MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH, SidebarView, animate_sidebar, build_appearance_menu,
    pin_status, preferred_sidebar_width, sidebar_rail_width,
};

pub(super) struct Header {
    widget: gtk::HeaderBar,
    pub(super) content: gtk::Box,
    pub(super) sidebar_toggle: gtk::ToggleButton,
    pub(super) search: gtk::Button,
    pub(super) close: gtk::Button,
    pub(super) settings: gtk::Button,
}

impl Header {
    pub(super) fn new(
        window: &gtk::ApplicationWindow,
        browser: &BrowserView,
        preview: &PreviewDrawer,
        preferences: &Rc<PreferenceManager>,
    ) -> Self {
        let widget = gtk::HeaderBar::new();
        widget.set_show_title_buttons(false);
        let sidebar_toggle = gtk::ToggleButton::builder()
            .active(true)
            .tooltip_text("Toggle sidebar (Ctrl+B)")
            .build();
        sidebar_toggle.set_child(Some(&assets::primary_icon(icons::PANEL_LEFT, 17)));
        sidebar_toggle.add_css_class("sidebar-toggle");
        sidebar_toggle.set_cursor_from_name(Some("pointer"));
        let location = browser.location_widget();
        location.set_hexpand(true);
        let search = header_action(icons::SEARCH, "Search (Ctrl+K)");
        let appearance =
            build_appearance_menu(browser, &browser.browser(), preferences.clone(), preview);
        let settings = header_action(icons::SETTINGS, "Settings");
        let close = header_action(icons::X, "Close window");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions.add_css_class("header-actions");
        actions.append(&search);
        actions.append(&appearance);
        actions.append(&settings);
        actions.append(&close);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.set_hexpand(true);
        content.set_valign(gtk::Align::Center);
        content.append(&sidebar_toggle);
        content.append(&location);
        content.append(&actions);
        widget.set_title_widget(Some(&content));
        let header = Self {
            widget,
            content,
            sidebar_toggle,
            search,
            close,
            settings,
        };
        // Minimal mode hides Search.
        preferences.bind_preference(
            &header.search,
            PreferenceManager::minimal_mode,
            |widget, minimal| {
                widget.set_visible(!minimal);
            },
        );
        let closing_window = window.downgrade();
        header.close.connect_clicked(move |_| {
            if let Some(window) = closing_window.upgrade() {
                window.close();
            }
        });
        header
    }
}

fn header_action(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&assets::chrome_icon(icon)));
    button.add_css_class("header-action");
    button.set_cursor_from_name(Some("pointer"));
    button
}

pub(super) fn preview(browser: &BrowserView, preferences: &Rc<PreferenceManager>) -> PreviewDrawer {
    let preferences = preferences.clone();
    let preview = PreviewDrawer::new(
        Rc::new(LocalPreviewProvider::new(Rc::new(move || {
            preferences.media_preview_backend()
        }))),
        true,
    );
    preview.observe_browser(&browser.browser());
    preview
}

pub(super) fn browser_layout(
    browser: &BrowserView,
    preview: &PreviewDrawer,
    sidebar: &SidebarView,
    header: &Header,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header.widget);
    bind_pin_handlers(browser, sidebar);
    let preview_for_print = preview.downgrade();
    browser.set_print_handler(Rc::new(move |entry| {
        if let Some(preview) = preview_for_print.upgrade() {
            preview.print_entry(entry);
        }
    }));
    let content = browser_split(browser, sidebar, &header.sidebar_toggle, preview);
    let preview_split = gtk::Paned::new(gtk::Orientation::Horizontal);
    preview_split.add_css_class("preview-split");
    preview_split.set_wide_handle(false);
    preview_split.set_resize_start_child(true);
    preview_split.set_resize_end_child(false);
    preview_split.set_shrink_start_child(false);
    preview_split.set_shrink_end_child(true);
    preview_split.set_start_child(Some(&content));
    preview_split.set_end_child(Some(&preview.widget()));
    preview_split.set_position(i32::MAX);
    preview_split.set_vexpand(true);
    preview.attach_split(&preview_split, &content, browser, Some(sidebar));
    browser.add_marquee_origin(&preview.widget(), gtk::PackType::End);
    root.append(&preview_split);
    root
}

fn bind_pin_handlers(browser: &BrowserView, sidebar: &SidebarView) {
    let weak_pin_sidebar = Rc::downgrade(&sidebar.state);
    let weak_unpin_sidebar = Rc::downgrade(&sidebar.state);
    let pinned_places = sidebar.state.pinned_places.clone();
    browser.set_pin_handlers(
        Rc::new(move |location, name| {
            if let Some(sidebar) = weak_pin_sidebar.upgrade() {
                sidebar.pin_location(location, name);
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
    toggle: &gtk::ToggleButton,
    preview: &PreviewDrawer,
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
    browser.add_marquee_origin(&sidebar.widget, gtk::PackType::Start);
    content.set_start_child(Some(&sidebar.widget));
    content.set_end_child(Some(&browser.widget()));
    bind_sidebar_toggle(&content, sidebar, toggle, preview);
    bind_sidebar_layout(&content, sidebar, toggle, preview);
    super::super::bind_sidebar_text_size(&content, sidebar);
    content
}

fn bind_sidebar_layout(
    content: &gtk::Paned,
    sidebar: &SidebarView,
    toggle: &gtk::ToggleButton,
    preview: &PreviewDrawer,
) {
    let weak_sidebar = Rc::downgrade(&sidebar.state);
    let weak_toggle = toggle.downgrade();
    let weak_preview = preview.downgrade();
    content.add_tick_callback(move |content, _| {
        let (Some(toggle), Some(sidebar), Some(preview)) = (
            weak_toggle.upgrade(),
            weak_sidebar.upgrade(),
            weak_preview.upgrade(),
        ) else {
            return glib::ControlFlow::Break;
        };
        if !toggle.is_active() || preview.is_open() {
            return glib::ControlFlow::Continue;
        }
        let available = content
            .root()
            .map_or_else(|| content.width(), |r| r.width());
        if available <= 0 {
            return glib::ControlFlow::Continue;
        }
        let needs_full = preferred_sidebar_width() + crate::ui::browser::COLUMN_WIDTH + 1;
        let is_railed = sidebar.rail.get();
        if is_railed && available >= needs_full {
            let restore = sidebar
                .saved_width
                .get()
                .unwrap_or_else(preferred_sidebar_width);
            sidebar.set_rail(false);
            content.set_position(restore);
        } else if !is_railed && available < needs_full {
            sidebar.set_rail(true);
            content.set_position(sidebar_rail_width());
        }
        glib::ControlFlow::Continue
    });
}

fn bind_sidebar_toggle(
    content: &gtk::Paned,
    sidebar: &SidebarView,
    toggle: &gtk::ToggleButton,
    preview: &PreviewDrawer,
) {
    let generation = Rc::new(Cell::new(0));
    let animating = Rc::new(Cell::new(false));
    let constrained_toggle = toggle.downgrade();
    let constrained_animation = animating.clone();
    let constrained_sidebar = Rc::downgrade(&sidebar.state);
    content.connect_position_notify(move |content| {
        if !constrained_toggle
            .upgrade()
            .is_some_and(|toggle| toggle.is_active())
            || constrained_animation.get()
        {
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
        } else {
            state.saved_width.set(Some(content.position()));
        }
    });
    let content = content.downgrade();
    let sidebar_widget = sidebar.widget.downgrade();
    let toggled_sidebar = Rc::downgrade(&sidebar.state);
    let toggled_preview = preview.downgrade();
    toggle.connect_toggled(move |toggle| {
        let (Some(content), Some(sidebar_widget), Some(state), Some(preview)) = (
            content.upgrade(),
            sidebar_widget.upgrade(),
            toggled_sidebar.upgrade(),
            toggled_preview.upgrade(),
        ) else {
            return;
        };
        let open = toggle.is_active();
        let window_width = content
            .root()
            .and_downcast::<gtk::Window>()
            .map(|w| w.default_width())
            .unwrap_or(0);
        let allocated = content
            .root()
            .map_or_else(|| content.width(), |r| r.width());
        let available = if allocated > 0 {
            allocated
        } else if window_width > 0 {
            window_width
        } else {
            content.width()
        };
        if open && !preview.is_open() {
            let needs = preferred_sidebar_width() + crate::ui::browser::COLUMN_WIDTH + 1;
            state.set_rail(available > 0 && available < needs);
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
}

pub(super) struct FooterBinding {
    pub(super) shortcuts: ShortcutFooter,
    jobs: crate::ui::jobs::JobsIndicator,
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
        shortcuts.bind_minimal_mode(preferences);
        shortcuts.observe_browser(browser);
        let jobs = crate::ui::jobs::JobsIndicator::new();
        jobs.bind_window(window);
        shortcuts.set_activity(jobs.widget());
        let clipboard = window.clipboard();
        let clipboard_handler = RefCell::new(Some(shortcuts.connect_clipboard(&clipboard)));
        root.append(shortcuts.widget());
        let updated_shortcuts = shortcuts.downgrade();
        browser.connect_view_mode_changed(move |mode| {
            if let Some(shortcuts) = updated_shortcuts.upgrade() {
                shortcuts.set_mode(mode);
            }
        });
        Self {
            shortcuts,
            jobs,
            clipboard,
            clipboard_handler,
        }
    }

    pub(super) fn disconnect_clipboard(&self) {
        if let Some(handler) = self.clipboard_handler.borrow_mut().take() {
            self.clipboard.disconnect(handler);
        }
        if let Some(popover) = self.jobs.widget().popover() {
            popover.unparent();
        }
    }
}
