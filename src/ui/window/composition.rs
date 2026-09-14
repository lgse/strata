// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{gio, prelude::*};

use crate::ui::{
    blur::BlurBin, browser::BrowserView, preview::PreviewDrawer, settings::UpdateNoticeHandler,
    theme::ThemeManager,
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
    pub(super) fn new(window: &gtk::ApplicationWindow, preferences: &Rc<ThemeManager>) -> Self {
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
        preferences: &Rc<ThemeManager>,
    ) -> UpdateNoticeHandler {
        search::install(window, self, preferences);
        install_browser_actions(window, &self.browser);
        let notice = settings::install(window, self, preferences);
        window.set_child(Some(&self.overlay));
        let click_browser = self.browser.clone();
        let click_window = window.clone();
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(move |_, _, x, y| {
            click_browser.dismiss_filter_on_outside_click(
                click_window.upcast_ref::<gtk::Widget>(),
                x,
                y,
            );
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

fn install_browser_actions(window: &gtk::ApplicationWindow, browser: &BrowserView) {
    let terminal_view = browser.clone();
    let terminal_action = gio::SimpleAction::new("open-terminal", None);
    terminal_action.connect_activate(move |_, _| {
        terminal_view.open_terminal();
    });
    window.add_action(&terminal_action);

    let refresh_view = browser.clone();
    let refresh_action = gio::SimpleAction::new("refresh", None);
    refresh_action.connect_activate(move |_, _| {
        refresh_view.refresh();
    });
    window.add_action(&refresh_action);
    if let Some(application) = window.application() {
        for (action, accels) in super::DEFAULT_ACCELS {
            application.set_accels_for_action(action, accels);
        }
    }
}

#[cfg(test)]
mod tests;
