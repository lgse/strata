// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{gio, prelude::*};

use crate::ui::{
    blur::BlurBin, browser::BrowserView, preferences::PreferenceManager, preview::PreviewDrawer,
    settings::UpdateNoticeHandler,
};

use super::{SidebarView, TypeToSearch, keyboard};

mod input;
mod layout;
mod search;
mod settings;

pub(super) struct WindowContent {
    pub(super) browser: BrowserView,
    pub(super) sidebar: SidebarView,
    preview: PreviewDrawer,
    header: layout::Header,
    overlay: gtk::Overlay,
    blurred_root: BlurBin,
    footer: layout::FooterBinding,
}

impl WindowContent {
    pub(super) fn new(
        window: &gtk::ApplicationWindow,
        preferences: &Rc<PreferenceManager>,
    ) -> Self {
        let browser = super::browser_for_window();
        let preview = layout::preview(&browser, preferences);
        let header = layout::Header::new(window, &browser, &preview, preferences);
        let sidebar = super::build_sidebar(browser.clone(), preferences.clone(), false);
        let root = layout::browser_layout(&browser, &preview, &sidebar, &header);
        let footer = layout::FooterBinding::new(window, &root, &browser, preferences);
        input::install_mouse_history(&root, &browser);
        crate::ui::scrolling::install_autoscroll_stop(&root);
        let overlay = gtk::Overlay::new();
        let blurred_root = BlurBin::new(&root);
        overlay.set_child(Some(&blurred_root));
        Self {
            browser,
            sidebar,
            preview,
            header,
            overlay,
            blurred_root,
            footer,
        }
    }

    pub(super) fn bind(
        &self,
        window: &gtk::ApplicationWindow,
        preferences: &Rc<PreferenceManager>,
    ) -> UpdateNoticeHandler {
        search::install(window, self, preferences);
        install_browser_actions(window, &self.browser, preferences);
        let (notice, open_settings) = settings::install(window, self, preferences);
        window.set_child(Some(&self.overlay));
        let click_browser = self.browser.downgrade();
        let click_window = window.downgrade();
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(move |_, _, x, y| {
            if let Some(browser) = click_browser.upgrade()
                && let Some(window) = click_window.upgrade()
            {
                browser.dismiss_filter_on_outside_click(window.upcast_ref(), x, y);
            }
        });
        window.add_controller(click);
        input::install_edit_cancellation(window, &self.browser);
        super::install_modal_focus_trap(window);
        let top_bar = crate::ui::top_bar_navigation::TopBarNavigation::new(
            &self.header.content,
            &self.sidebar.widget,
            &self.header.sidebar_toggle,
        );
        keyboard::install(
            window,
            &self.sidebar,
            keyboard::Bindings {
                view: self.browser.clone(),
                top_bar,
                preview: self.preview.clone(),
                type_to_search: TypeToSearch {
                    view: self.browser.clone(),
                    preferences: preferences.clone(),
                },
                shortcuts: self.footer.shortcuts.clone(),
                open_settings,
            },
        );
        notice
    }

    pub(super) fn connect_cleanup(self, window: &gtk::ApplicationWindow) {
        let browser = self.browser.browser();
        let sidebar = self.sidebar;
        let footer = self.footer;
        window.connect_destroy(move |_| {
            footer.disconnect_clipboard();
            browser.bump_navigation_generation();
            browser.clear_observer();
            sidebar.disconnect();
        });
    }
}

fn install_browser_actions(
    window: &gtk::ApplicationWindow,
    browser: &BrowserView,
    preferences: &Rc<PreferenceManager>,
) {
    let terminal_view = browser.downgrade();
    let terminal_action = gio::SimpleAction::new("open-terminal", None);
    terminal_action.connect_activate(move |_, _| {
        if let Some(view) = terminal_view.upgrade() {
            view.open_terminal();
        }
    });
    window.add_action(&terminal_action);

    let refresh_view = browser.downgrade();
    let refresh_action = gio::SimpleAction::new("refresh", None);
    refresh_action.connect_activate(move |_, _| {
        if let Some(view) = refresh_view.upgrade() {
            view.refresh();
        }
    });
    window.add_action(&refresh_action);

    let toggle_preferences = preferences.clone();
    let toggle_action = gio::SimpleAction::new("toggle-arrow-scope", None);
    toggle_action.connect_activate(move |_, _| {
        let next = !toggle_preferences.arrow_navigation_scoped();
        toggle_preferences.set_arrow_navigation_scoped(next);
    });
    window.add_action(&toggle_action);
    if let Some(application) = window.application() {
        for (action, accels) in super::DEFAULT_ACCELS {
            application.set_accels_for_action(action, accels);
        }
        // Accelerators are application-wide; closing one window must not change other windows' map.
        let accels_application = application.clone();
        preferences.bind_preference(
            window,
            PreferenceManager::minimal_mode,
            move |_, enabled| {
                if enabled {
                    for (action, _) in super::DEFAULT_ACCELS {
                        accels_application.set_accels_for_action(action, &[]);
                    }
                    tracing::debug!("minimal mode accels cleared");
                } else {
                    for (action, accels) in super::DEFAULT_ACCELS {
                        accels_application.set_accels_for_action(action, accels);
                    }
                    tracing::debug!("minimal mode accels restored");
                }
            },
        );
    }
}

#[cfg(test)]
mod tests;
