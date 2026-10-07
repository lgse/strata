// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

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
mod tabs;
mod tenxer_splash;

pub(super) use tabs::{TabWindow, is_tab_shortcut, tab_index};

pub(super) struct WindowContent {
    pub(super) browser: BrowserView,
    pub(super) sidebar: SidebarView,
    preview: PreviewDrawer,
    header: layout::Header,
    overlay: gtk::Overlay,
    blurred_root: BlurBin,
    footer: layout::FooterBinding,
    actions: gio::SimpleActionGroup,
    key_controllers: RefCell<Vec<gtk::EventController>>,
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
        overlay.add_css_class("tab-context");
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
            actions: gio::SimpleActionGroup::new(),
            key_controllers: RefCell::new(Vec::new()),
        }
    }

    #[cfg(test)]
    pub(super) fn bind(
        &self,
        window: &gtk::ApplicationWindow,
        preferences: &Rc<PreferenceManager>,
    ) -> UpdateNoticeHandler {
        let notice = self.bind_context(window, preferences);
        self.activate_actions(window);
        window.set_child(Some(&self.overlay));
        tenxer_splash::install(window, &self.overlay, preferences);
        super::install_modal_focus_trap(window);
        notice
    }

    fn activate_actions(&self, window: &gtk::ApplicationWindow) {
        for name in self.actions.list_actions() {
            if let Some(action) = self.actions.lookup_action(&name) {
                window.add_action(&action);
            }
        }
    }

    fn bind_context(
        &self,
        window: &gtk::ApplicationWindow,
        preferences: &Rc<PreferenceManager>,
    ) -> UpdateNoticeHandler {
        search::install(self, preferences);
        install_browser_actions(window, &self.actions, &self.browser, preferences);
        let notice = settings::install(window, self, preferences);
        let click_browser = self.browser.clone();
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(move |gesture, _, x, y| {
            if let Some(window) = gesture.widget() {
                click_browser.dismiss_filter_on_outside_click(&window, x, y);
            }
        });
        self.overlay.add_controller(click);
        input::install_edit_cancellation(&self.overlay, &self.browser);
        let top_bar = crate::ui::top_bar_navigation::TopBarNavigation::new(
            &self.header.content,
            &self.sidebar.widget,
            &self.header.sidebar_toggle,
        );
        let controllers = keyboard::install_on(
            window,
            &self.overlay,
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
                history: crate::services::NavigationHistory::shared(),
            },
        );
        self.key_controllers.borrow_mut().extend(controllers);
        notice
    }

    #[cfg(test)]
    pub(super) fn search_button(&self) -> &gtk::Button {
        &self.header.search
    }

    #[cfg(test)]
    pub(super) fn settings_button(&self) -> &gtk::Button {
        &self.header.settings
    }

    #[cfg(test)]
    pub(super) fn minimize_button(&self) -> &gtk::Button {
        &self.header.minimize
    }

    #[cfg(test)]
    pub(super) fn maximize_button(&self) -> &gtk::Button {
        &self.header.maximize
    }

    #[cfg(test)]
    pub(super) fn close_button(&self) -> &gtk::Button {
        &self.header.close
    }

    #[cfg(test)]
    pub(super) fn sidebar_toggle(&self) -> &gtk::ToggleButton {
        &self.header.sidebar_toggle
    }

    #[cfg(test)]
    pub(super) fn sidebar_visible(&self) -> bool {
        self.sidebar.widget.is_visible()
    }

    #[cfg(test)]
    pub(super) fn footer(&self) -> &crate::ui::shortcut_footer::ShortcutFooter {
        &self.footer.shortcuts
    }

    #[cfg(test)]
    pub(super) fn overlay(&self) -> &gtk::Overlay {
        &self.overlay
    }

    /// gtk_window_destroy() unrealizes a window but frees it only with its last
    /// reference, which its own closures can hold, so cleanup cannot wait for
    /// the destroy signal.
    #[cfg(test)]
    pub(super) fn connect_cleanup(self, window: &gtk::ApplicationWindow) {
        window.connect_unrealize(move |_| self.dispose());
    }

    fn dispose(&self) {
        for controller in self.key_controllers.take() {
            if let Some(widget) = controller.widget() {
                widget.remove_controller(&controller);
            }
        }
        self.footer.shortcuts.cancel_chord();
        self.footer.shortcuts.dismiss_prompt();
        self.footer.disconnect_clipboard();
        self.preview.clear_target();
        let browser = self.browser.browser();
        browser.bump_navigation_generation();
        browser.clear_observer();
        self.browser.dispose_file_progress();
        browser.cancel_background_operations();
        browser.cancel_file_operation();
        self.browser.finish_navigation_cleanup();
        self.sidebar.disconnect();
        PreferenceManager::shared().release_bindings_within(&self.overlay);
        let controllers = self.overlay.observe_controllers();
        while let Some(controller) = controllers.item(0).and_downcast::<gtk::EventController>() {
            self.overlay.remove_controller(&controller);
        }
    }
}

fn install_browser_actions(
    window: &gtk::ApplicationWindow,
    actions: &gio::SimpleActionGroup,
    browser: &BrowserView,
    preferences: &Rc<PreferenceManager>,
) {
    let terminal_view = browser.clone();
    let terminal_action = gio::SimpleAction::new("open-terminal", None);
    terminal_action.connect_activate(move |_, _| {
        terminal_view.open_terminal();
    });
    actions.add_action(&terminal_action);

    let refresh_view = browser.clone();
    let refresh_action = gio::SimpleAction::new("refresh", None);
    refresh_action.connect_activate(move |_, _| {
        refresh_view.refresh();
    });
    actions.add_action(&refresh_action);

    let toggle_preferences = preferences.clone();
    let toggle_action = gio::SimpleAction::new("toggle-arrow-scope", None);
    toggle_action.connect_activate(move |_, _| {
        let next = !toggle_preferences.arrow_navigation_scoped();
        toggle_preferences.set_arrow_navigation_scoped(next);
    });
    actions.add_action(&toggle_action);
    // The set lives on the application, so it follows the saved mode rather
    // than whichever window was constructed or destroyed last.
    if let Some(application) = window.application() {
        let application = application.clone();
        preferences.bind_preference(
            &browser.widget(),
            PreferenceManager::tenxer_mode,
            move |_window, enabled| {
                super::install_mode_accelerators(&application, enabled);
            },
        );
    }
}
