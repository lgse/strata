// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};

use crate::ui::{
    blur::BlurBin,
    browser::{BrowserView, PaneTarget},
    preferences::PreferenceManager,
    preview::PreviewDrawer,
    settings::UpdateNoticeHandler,
};

use super::{SidebarView, TypeToSearch, keyboard};

mod input;
mod layout;
mod pane_controls;
mod panes;
mod search;
mod settings;
mod tabs;
mod tenxer_splash;

pub(super) use tabs::{TabWindow, is_tab_shortcut, tab_index};

/// What the tab-level pane shortcuts ask of a tab.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PaneCommand {
    Toggle,
    FocusOther,
    CopyToOther,
    MoveToOther,
}

pub(super) struct WindowContent {
    /// The main pane's view, which every tab keeps.
    pub(super) browser: BrowserView,
    pub(super) sidebar: SidebarView,
    preview: PreviewDrawer,
    header: layout::Header,
    overlay: gtk::Overlay,
    blurred_root: BlurBin,
    panes: Rc<panes::Panes>,
    target: PaneTarget,
    actions: gio::SimpleActionGroup,
    key_controllers: RefCell<Vec<gtk::EventController>>,
    window: glib::WeakRef<gtk::ApplicationWindow>,
    preferences: Rc<PreferenceManager>,
}

impl WindowContent {
    pub(super) fn new(
        window: &gtk::ApplicationWindow,
        preferences: &Rc<PreferenceManager>,
    ) -> Self {
        let browser = super::browser_for_window();
        let target = PaneTarget::new(&browser);
        let preview = layout::preview(&browser, preferences);
        let header = layout::Header::new(window, &target, &preview, preferences);
        let sidebar = super::build_pane_sidebar(target.clone(), preferences.clone(), false);
        let footers = footer_stack();
        let footer = layout::FooterBinding::new(window, &footers, &browser, preferences);
        let panes = panes::Panes::new(
            panes::Pane::new(browser.clone(), footer),
            target.clone(),
            &header.locations,
            &footers,
        );
        let root = layout::browser_layout(
            &layout::PaneArea {
                target: &target,
                panes: &panes.widget(),
                footers: &footers,
            },
            &preview,
            &sidebar,
            &header,
            preferences,
        );
        input::install_mouse_history(&root, &target);
        crate::ui::scrolling::install_autoscroll_stop(&root);
        let overlay = gtk::Overlay::new();
        overlay.add_css_class("tab-context");
        let blurred_root = BlurBin::new(&root);
        overlay.set_child(Some(&blurred_root));
        let retargeted = preview.clone();
        target.connect_retargeted(move |view| retargeted.retarget(view));
        Self {
            browser,
            sidebar,
            preview,
            header,
            overlay,
            blurred_root,
            panes,
            target,
            actions: gio::SimpleActionGroup::new(),
            key_controllers: RefCell::new(Vec::new()),
            window: window.downgrade(),
            preferences: preferences.clone(),
        }
    }

    /// The view of the pane the tab's chrome acts on.
    pub(super) fn active_view(&self) -> BrowserView {
        self.panes.active_view()
    }

    pub(super) fn target(&self) -> &PaneTarget {
        &self.target
    }

    #[cfg(test)]
    pub(super) fn is_split(&self) -> bool {
        self.panes.layout().split()
    }

    pub(super) fn cancel_chords(&self) {
        self.panes.cancel_chords();
    }

    pub(super) fn has_background_operations(&self) -> bool {
        self.panes.has_background_operations()
    }

    pub(super) fn set_presence(&self, presence: crate::app::Presence, power_saver: bool) {
        self.panes.set_presence(presence, power_saver);
    }

    pub(super) fn observe_location(
        &self,
        listener: impl Fn(Option<&crate::model::Location>) + 'static,
    ) {
        self.panes.observe_location(listener);
    }

    /// The widget that holds both panes, for saving focus inside either.
    pub(super) fn panes_widget(&self) -> gtk::Widget {
        self.panes.widget()
    }

    pub(super) fn run_pane_command(&self, command: PaneCommand) {
        match command {
            PaneCommand::Toggle => {
                if !self.panes.has_extra() {
                    self.build_extra_pane();
                }
                self.panes.toggle();
                self.header.extra_pane.set_active(self.panes.is_split());
            }
            PaneCommand::FocusOther => self.panes.focus_other(),
            PaneCommand::CopyToOther => self.panes.transfer_to_other(false),
            PaneCommand::MoveToOther => self.panes.transfer_to_other(true),
        }
    }

    /// Builds the extra pane with the same wiring the main pane got.
    fn build_extra_pane(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let view = super::browser_for_window();
        let footer =
            layout::FooterBinding::new(&window, &self.panes.footers(), &view, &self.preferences);
        layout::bind_pin_handlers(&view, &self.sidebar);
        self.preview.follow_pane(&view);
        search::record_history(&view);
        self.bind_pane_input(&view);
        let pane = panes::Pane::new(view, footer);
        let controllers = self.install_pane_keys(&window, &pane);
        self.key_controllers.borrow_mut().extend(controllers);
        self.panes.attach_extra(pane);
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
        crate::ui::keys::install_native_aliases(window);
        self.browser
            .set_as_modal_focus_fallback(window.upcast_ref());
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
        search::install(window, self, preferences);
        install_browser_actions(window, &self.actions, &self.target, preferences);
        let notice = settings::install(window, self, preferences);
        self.bind_pane_input(&self.browser);
        let controllers = self.install_pane_keys(window, self.panes.main());
        self.key_controllers.borrow_mut().extend(controllers);
        notice
    }

    /// Clicks outside a pane's filter or location edit end them.
    fn bind_pane_input(&self, view: &BrowserView) {
        let click_browser = view.clone();
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(move |gesture, _, x, y| {
            if let Some(window) = gesture.widget() {
                click_browser.dismiss_filter_on_outside_click(&window, x, y);
            }
        });
        self.overlay.add_controller(click);
        input::install_edit_cancellation(&self.overlay, view);
    }

    fn install_pane_keys(
        &self,
        window: &gtk::ApplicationWindow,
        pane: &panes::Pane,
    ) -> Vec<gtk::EventController> {
        let top_bar = crate::ui::top_bar_navigation::TopBarNavigation::new(
            &self.header.content,
            &self.sidebar.widget,
            self.header.sidebar_toggle.upcast_ref(),
        );
        keyboard::install_on(
            window,
            &self.overlay,
            &self.sidebar,
            keyboard::DispatcherParts {
                view: pane.view.clone(),
                top_bar,
                preview: self.preview.clone(),
                type_to_search: TypeToSearch {
                    view: pane.view.clone(),
                    preferences: self.preferences.clone(),
                },
                shortcuts: pane.footer.shortcuts.clone(),
                history: crate::services::NavigationHistory::shared(),
            },
            Some(pane.input.clone()),
        )
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
        self.header.sidebar_toggle.upcast_ref()
    }

    #[cfg(test)]
    pub(super) fn sidebar_visible(&self) -> bool {
        self.sidebar.widget.is_visible()
    }

    #[cfg(test)]
    pub(super) fn footer(&self) -> &crate::ui::shortcut_footer::ShortcutFooter {
        &self.panes.main().footer.shortcuts
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

    /// Frees a closed tab. The tab's widgets and the closures that capture
    /// them form reference cycles that nothing else breaks, so every widget is
    /// disposed as `gtk_window_destroy` disposes a window.
    pub(super) fn destroy(&self) {
        self.dispose();
        dispose_widget_tree(self.overlay.upcast_ref());
    }

    fn dispose(&self) {
        for controller in self.key_controllers.take() {
            if let Some(widget) = controller.widget() {
                widget.remove_controller(&controller);
            }
        }
        self.preview.clear_target();
        for pane in self.panes.all() {
            let footer = &pane.footer;
            footer.shortcuts.cancel_chord();
            footer.shortcuts.dismiss_prompt();
            footer.disconnect_clipboard();
            let view = &pane.view;
            let browser = view.browser();
            browser.bump_navigation_generation();
            browser.clear_observer();
            view.dispose_file_progress();
            browser.cancel_background_operations();
            browser.cancel_file_operation();
            view.finish_navigation_cleanup();
        }
        self.sidebar.disconnect();
        PreferenceManager::shared().release_bindings_within(&self.overlay);
        let controllers = self.overlay.observe_controllers();
        while let Some(controller) = controllers.item(0).and_downcast::<gtk::EventController>() {
            self.overlay.remove_controller(&controller);
        }
    }
}

/// Adds a `win.<name>` action that does nothing while a modal layer other than
/// `own_layer` is on top or an inline edit owns the active pane's listing. The
/// key dispatcher runs window actions after yielding to those owners, so the
/// check lives in the action rather than in the dispatcher.
fn add_guarded_window_action(
    window: &gtk::ApplicationWindow,
    actions: &gio::SimpleActionGroup,
    name: &str,
    target: &PaneTarget,
    own_layer: Option<gtk::Widget>,
    run: impl Fn() + 'static,
) {
    let action = gio::SimpleAction::new(name, None);
    let window = window.downgrade();
    let target = target.downgrade();
    action.connect_activate(move |_, _| {
        let (Some(window), Some(target)) = (window.upgrade(), target.upgrade()) else {
            return;
        };
        if super::foreign_modal_visible(&window, own_layer.as_ref())
            || target.view().inline_edit_is_open()
        {
            return;
        }
        run();
    });
    actions.add_action(&action);
}

fn install_browser_actions(
    window: &gtk::ApplicationWindow,
    actions: &gio::SimpleActionGroup,
    target: &PaneTarget,
    preferences: &Rc<PreferenceManager>,
) {
    let terminal_target = target.downgrade();
    add_guarded_window_action(window, actions, "open-terminal", target, None, move || {
        if let Some(target) = terminal_target.upgrade() {
            target.view().open_terminal();
        }
    });
    let refresh_target = target.downgrade();
    add_guarded_window_action(window, actions, "refresh", target, None, move || {
        if let Some(target) = refresh_target.upgrade() {
            target.view().refresh();
        }
    });
    let toggle_preferences = preferences.clone();
    add_guarded_window_action(
        window,
        actions,
        "toggle-arrow-scope",
        target,
        None,
        move || {
            let next = !toggle_preferences.arrow_navigation_scoped();
            toggle_preferences.set_arrow_navigation_scoped(next);
        },
    );
}

/// The pane footers, of which the active pane's shows.
fn footer_stack() -> gtk::Stack {
    gtk::Stack::builder()
        .hhomogeneous(false)
        .vhomogeneous(false)
        .transition_type(gtk::StackTransitionType::None)
        .build()
}

/// Disposes `root`, which must no longer have a parent, and whatever of its tree
/// survives that. Dispose drops a widget's signal handlers and controllers, so
/// a cycle through closures capturing the widget is broken while references
/// to it remain. Widgets nothing else holds are freed by their parent's
/// dispose, as on window destruction; a widget its parent keeps, such as a
/// range's slider, is left to that parent.
#[expect(
    unsafe_code,
    reason = "glib-rs marks g_object_run_dispose unsafe because later use of the object is undefined"
)]
fn dispose_widget_tree(root: &gtk::Widget) {
    let mut tree = vec![root.downgrade()];
    let mut index = 0;
    while let Some(widget) = tree.get(index).and_then(glib::WeakRef::upgrade) {
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            tree.push(current.downgrade());
        }
        index += 1;
    }
    for widget in tree.iter().filter_map(glib::WeakRef::upgrade) {
        if widget.parent().is_some() {
            continue;
        }
        let controllers = widget.observe_controllers();
        while let Some(controller) = controllers.item(0).and_downcast::<gtk::EventController>() {
            widget.remove_controller(&controller);
        }
        // SAFETY: the tab is closed and nothing presents these widgets again;
        // they stay valid objects until their last reference is dropped.
        unsafe { widget.run_dispose() };
    }
}
