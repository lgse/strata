// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::location_for_file,
    app::Presence,
    model::Location,
    ui::{
        blur::BlurBin,
        browser::BrowserView,
        keys::{self, KeyPress, bindings::*},
        preferences::PreferenceManager,
        tabs_session,
    },
};

use super::{WindowContent, layout, tenxer_splash};

mod strip;
use strip::TabStrip;

#[cfg(test)]
mod tests;

struct ClosedTab {
    location: Location,
    index: usize,
}

struct Tab {
    id: u64,
    content: Rc<WindowContent>,
    focus: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    checking: RefCell<Option<Location>>,
    folder: RefCell<Option<String>>,
}

impl Tab {
    fn location(&self) -> Option<Location> {
        self.content
            .browser
            .browser()
            .active_location()
            .or_else(|| self.checking.borrow().clone())
    }
}

pub(in crate::ui::window) struct TabWindow {
    window: glib::WeakRef<gtk::ApplicationWindow>,
    preferences: Rc<PreferenceManager>,
    stack: gtk::Stack,
    drag_token: String,
    strip: TabStrip,
    tabs: RefCell<Vec<Rc<Tab>>>,
    closed_tabs: RefCell<Vec<ClosedTab>>,
    active: Cell<u64>,
    next_id: Cell<u64>,
    hints: Cell<bool>,
    presence: Cell<(Presence, bool)>,
    restoring: Cell<bool>,
    persist: bool,
}

impl TabWindow {
    pub(in crate::ui::window) fn new(
        window: &gtk::ApplicationWindow,
        preferences: &Rc<PreferenceManager>,
        persist: bool,
    ) -> Rc<Self> {
        crate::ui::decoration::claim_titlebar(window);
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::None);
        let strip = TabStrip::new();
        let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        shell.append(strip.widget());
        shell.append(&stack);
        let overlay = gtk::Overlay::new();
        overlay.add_css_class("tab-window");
        overlay.set_child(Some(&BlurBin::new(&shell)));
        window.set_child(Some(&overlay));
        let state = Rc::new(Self {
            window: window.downgrade(),
            preferences: preferences.clone(),
            stack,
            strip,
            tabs: RefCell::new(Vec::new()),
            closed_tabs: RefCell::new(Vec::new()),
            active: Cell::new(0),
            next_id: Cell::new(1),
            hints: Cell::new(false),
            presence: Cell::new((Presence::default(), false)),
            restoring: Cell::new(false),
            persist,
            drag_token: glib::uuid_string_random().to_string(),
        });
        state.strip.install_drop(&state);
        state.add(None);
        state.install_actions(window);
        state.install_keys(window);
        super::super::install_modal_focus_trap(window);
        keys::install_native_aliases(window);
        let weak = Rc::downgrade(&state);
        crate::ui::modal::set_modal_focus_fallback(
            window.upcast_ref(),
            Rc::new(move || {
                let browser = weak.upgrade().and_then(|state| {
                    let active = state.active.get();
                    state
                        .tabs
                        .borrow()
                        .iter()
                        .find(|tab| tab.id == active)
                        .map(|tab| tab.content.browser.clone())
                });
                if let Some(browser) = browser {
                    browser.restore_listing_focus();
                }
            }),
        );
        tenxer_splash::install(window, &overlay, preferences);
        preferences.register_save_notice_window(window.upcast_ref());
        let weak = Rc::downgrade(&state);
        crate::ui::close_guard::install(window, move |_| {
            let state = weak.upgrade()?;
            state
                .tabs
                .borrow()
                .iter()
                .any(|tab| tab.content.browser.browser().has_background_operations())
                .then(operations_active)
        });
        let retained = state.clone();
        window.connect_unrealize(move |window| {
            PreferenceManager::shared().release_bindings_within(window);
            for tab in retained.tabs.take() {
                tab.content.dispose();
            }
            for name in [
                "search",
                "jump-folder",
                "refresh",
                "open-terminal",
                "toggle-arrow-scope",
                "new-tab",
                "close-tab",
                "reopen-tab",
                "previous-tab",
                "select-tab",
                "open-tab-at",
                "open-window-at",
                "sidebar-tab-drag",
            ] {
                window.remove_action(name);
            }
        });
        let weak = Rc::downgrade(&state);
        window.connect_is_active_notify(move |window| {
            if !window.is_active()
                && let Some(state) = weak.upgrade()
            {
                state.show_hints(false);
            }
        });
        let weak = Rc::downgrade(&state);
        crate::ui::presence::follow(window, move |presence, power_saver| {
            if let Some(state) = weak.upgrade() {
                state.set_presence(presence, power_saver);
            }
        });
        state
    }

    fn set_presence(&self, presence: Presence, power_saver: bool) {
        self.presence.set((presence, power_saver));
        self.share_presence();
    }

    fn share_presence(&self) {
        let (presence, power_saver) = self.presence.get();
        let browsers: Vec<_> = self
            .tabs
            .borrow()
            .iter()
            .map(|tab| (tab.id == self.active.get(), tab.content.browser.clone()))
            .collect();
        for (active, browser) in browsers {
            let presence = if active { presence } else { Presence::Hidden };
            browser.set_presence(presence, power_saver);
        }
    }

    fn active_tab(&self) -> Rc<Tab> {
        self.tabs
            .borrow()
            .iter()
            .find(|tab| tab.id == self.active.get())
            .expect("a window always has an active tab")
            .clone()
    }

    pub(in crate::ui::window) fn active_browser(&self) -> BrowserView {
        self.active_tab().content.browser.clone()
    }

    fn blocked(&self) -> bool {
        self.window
            .upgrade()
            .is_none_or(|window| super::super::visible_modal_layer(&window).is_some())
    }

    fn new_tab(self: &Rc<Self>) {
        if self.blocked() {
            return;
        }
        match self.active_browser().browser().active_location() {
            Some(location) => self.add(Some(location)),
            None => {
                self.add(None);
                super::super::open_default_folder(
                    &self.active_browser().browser(),
                    &self.preferences,
                );
            }
        }
    }

    fn reopen_closed_tab(self: &Rc<Self>) {
        if self.blocked() {
            return;
        }
        let closed = self.closed_tabs.borrow_mut().pop();
        let Some(closed) = closed else {
            return;
        };
        self.open_in_new_tab(closed.location);
        let target = self.tabs.borrow().get(closed.index).map(|tab| tab.id);
        if let Some(target) = target {
            self.reorder(self.active.get(), target);
        }
    }

    fn open_in_new_tab(self: &Rc<Self>, location: Location) {
        if self.blocked() || self.window.upgrade().is_none() {
            return;
        }
        tracing::debug!(location = %location.diagnostic_path(), "opening location in new tab");
        self.add(None);
        super::super::open_argument::route_location(self.active_browser(), location);
    }

    fn open_in_new_window(self: &Rc<Self>, location: Location) -> Option<BrowserView> {
        let application = self.window.upgrade()?.application()?;
        tracing::debug!(location = %location.diagnostic_path(), "opening location in new window");
        Some(super::super::open_argument::present_location_window(
            &application,
            location,
        ))
    }

    fn add(self: &Rc<Self>, location: Option<Location>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let content = Rc::new(WindowContent::new(&window, &self.preferences));
        content.bind_context(&window, &self.preferences);
        let tab = Rc::new(Tab {
            id,
            content: content.clone(),
            focus: RefCell::new(None),
            checking: RefCell::new(None),
            folder: RefCell::new(None),
        });
        self.stack
            .add_named(&content.overlay, Some(&id.to_string()));
        let weak = Rc::downgrade(self);
        content.header.new_tab.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.new_tab();
            }
        });
        self.tabs.borrow_mut().push(tab);
        self.strip.add(self, id, &content.browser);
        strip::install_new_tab_drop(self, &content.header.new_tab);
        let weak = Rc::downgrade(self);
        content.browser.observe_tab_location(move |location| {
            if let Some(state) = weak.upgrade() {
                let label = tab_label(location);
                state.strip.label(id, &label);
                state.set_folder(id, location.map(|_| label));
                state.persist_session();
            }
        });
        if id == 1 {
            super::super::schedule_after_first_paint(&window, &content.sidebar, &self.preferences);
        } else {
            content.sidebar.schedule_after_first_paint(&content.overlay);
        }
        self.select(id);
        self.share_presence();
        self.refresh_chrome();
        if let Some(location) = location {
            content.browser.navigate_location(location);
        }
    }

    fn select(&self, id: u64) {
        if self.active.get() == id || self.blocked() {
            return;
        }
        let Some(tab) = self.tabs.borrow().iter().find(|tab| tab.id == id).cloned() else {
            return;
        };
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if self.active.get() != 0 {
            let previous = self.active_tab();
            // Drop focus on the strip, header or footer: a click focuses a tab
            // button before `clicked` runs, and a restored tab is left before
            // its listing loads, with GTK's default focus on the footer.
            let content = &previous.content;
            let panes = [
                content.browser.widget(),
                content.sidebar.widget.clone(),
                content.preview.widget(),
            ];
            *previous.focus.borrow_mut() = gtk::prelude::RootExt::focus(&window)
                .filter(|widget| panes.iter().any(|pane| widget.is_ancestor(pane)))
                .map(|widget| widget.downgrade());
            previous.content.footer.shortcuts.cancel_chord();
            previous.content.browser.cancel_location_edit();
        }
        self.active.set(id);
        self.share_presence();
        self.stack.set_visible_child(&tab.content.overlay);
        tab.content.activate_actions(&window);
        self.refresh_chrome();
        self.strip.select(id);
        self.refresh_title();
        let saved = tab
            .focus
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
            .filter(|widget| widget.is_mapped());
        // Saved focus may be a pane surface that is no longer focusable.
        if !saved.is_some_and(|focus| focus.grab_focus()) {
            tab.content.browser.browser().focus_active();
        }
        self.persist_session();
    }

    fn close(&self, id: u64) {
        if self.blocked() {
            return;
        }
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let tabs = self.tabs.borrow();
        let Some(index) = tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        if tabs[index]
            .content
            .browser
            .browser()
            .has_background_operations()
        {
            operations_active().show(window.upcast_ref());
            return;
        }
        if tabs.len() == 1 {
            drop(tabs);
            window.close();
            return;
        }
        if let Some(location) = tabs[index].location() {
            let mut closed = self.closed_tabs.borrow_mut();
            if closed.len() == 20 {
                closed.remove(0);
            }
            closed.push(ClosedTab { location, index });
        }
        drop(tabs);
        self.discard(id);
    }

    fn discard(&self, id: u64) {
        let tabs = self.tabs.borrow();
        let Some(index) = tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let tab = tabs[index].clone();
        let next = tabs[if index + 1 < tabs.len() {
            index + 1
        } else {
            index - 1
        }]
        .id;
        drop(tabs);
        if self.active.get() == id {
            self.select(next);
        }
        self.tabs.borrow_mut().retain(|tab| tab.id != id);
        tab.content.dispose();
        self.stack.remove(&tab.content.overlay);
        self.strip.remove(id);
        self.refresh_chrome();
        self.persist_session();
    }

    fn set_folder(&self, id: u64, folder: Option<String>) {
        let Some(tab) = self.tabs.borrow().iter().find(|tab| tab.id == id).cloned() else {
            return;
        };
        tab.folder.replace(folder);
        if id == self.active.get() {
            self.refresh_title();
        }
    }

    fn refresh_title(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let title = self
            .tabs
            .borrow()
            .iter()
            .find(|tab| tab.id == self.active.get())
            .and_then(|tab| tab.folder.borrow().clone())
            .map_or_else(
                || "Strata".to_owned(),
                |folder| format!("{folder} — Strata"),
            );
        window.set_title(Some(&title));
    }

    fn refresh_chrome(&self) {
        let multiple = self.tabs.borrow().len() > 1;
        self.strip.refresh_visibility();
        for tab in self.tabs.borrow().iter() {
            let header = &tab.content.header;
            if let Some(parent) = header.new_tab.parent().and_downcast::<gtk::Box>() {
                parent.remove(&header.new_tab);
            }
            if let Some(parent) = header.close.parent().and_downcast::<gtk::Box>() {
                parent.remove(&header.close);
            }
            if multiple && tab.id == self.active.get() {
                self.strip.actions.append(&header.new_tab);
                self.strip.end.append(&header.close);
            } else {
                header.actions.prepend(&header.new_tab);
                header.actions.append(&header.close);
                header.new_tab.set_visible(!multiple);
                header
                    .close
                    .set_visible(!multiple && self.preferences.window_show_close());
            }
            if tab.id == self.active.get() {
                header.new_tab.set_visible(true);
                header
                    .close
                    .set_visible(self.preferences.window_show_close());
            }
        }
        self.strip.hints(self.hints.get());
    }

    fn select_index(&self, index: usize) {
        let id = self.tabs.borrow().get(index).map(|tab| tab.id);
        if let Some(id) = id {
            self.select(id);
        }
    }

    fn install_actions(self: &Rc<Self>, window: &gtk::ApplicationWindow) {
        let sidebar_drag =
            gio::SimpleAction::new("sidebar-tab-drag", Some(&bool::static_variant_type()));
        let weak = Rc::downgrade(self);
        sidebar_drag.connect_activate(move |_, parameter| {
            if let Some(state) = weak.upgrade() {
                let active = parameter
                    .and_then(|value| value.get::<bool>())
                    .unwrap_or(false);
                state.strip.show_open_drop(active && !state.blocked());
            }
        });
        window.add_action(&sidebar_drag);

        let create = gio::SimpleAction::new("new-tab", None);
        let weak = Rc::downgrade(self);
        create.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.new_tab();
            }
        });
        window.add_action(&create);

        let close = gio::SimpleAction::new("close-tab", None);
        let weak = Rc::downgrade(self);
        close.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.close(state.active.get());
            }
        });
        window.add_action(&close);

        let reopen = gio::SimpleAction::new("reopen-tab", None);
        let weak = Rc::downgrade(self);
        reopen.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.reopen_closed_tab();
            }
        });
        window.add_action(&reopen);

        let previous = gio::SimpleAction::new("previous-tab", None);
        let weak = Rc::downgrade(self);
        previous.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.cycle(-1);
            }
        });
        window.add_action(&previous);

        let select = gio::SimpleAction::new("select-tab", Some(&u32::static_variant_type()));
        let weak = Rc::downgrade(self);
        select.connect_activate(move |_, parameter| {
            if let Some(state) = weak.upgrade()
                && let Some(index) = parameter.and_then(|value| value.get::<u32>())
            {
                state.select_index(index as usize);
            }
        });
        window.add_action(&select);

        // Window actions let menu rows and gestures reach this `TabWindow`.
        let open_tab = gio::SimpleAction::new("open-tab-at", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        open_tab.connect_activate(move |_, parameter| {
            if let (Some(state), Some(location)) = (weak.upgrade(), action_location(parameter)) {
                state.open_in_new_tab(location);
            }
        });
        window.add_action(&open_tab);

        let open_window =
            gio::SimpleAction::new("open-window-at", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        open_window.connect_activate(move |_, parameter| {
            if let (Some(state), Some(location)) = (weak.upgrade(), action_location(parameter)) {
                state.open_in_new_window(location);
            }
        });
        window.add_action(&open_window);
    }

    fn cycle(&self, delta: i32) {
        let tabs = self.tabs.borrow();
        let current = tabs
            .iter()
            .position(|tab| tab.id == self.active.get())
            .unwrap_or(0);
        let index = (current as i32 + delta).rem_euclid(tabs.len() as i32) as usize;
        let id = tabs[index].id;
        drop(tabs);
        self.select(id);
    }

    fn reorder(&self, source: u64, target: u64) {
        let tabs = self.tabs.borrow();
        let (Some(from), Some(to)) = (
            tabs.iter().position(|tab| tab.id == source),
            tabs.iter().position(|tab| tab.id == target),
        ) else {
            return;
        };
        drop(tabs);
        self.reorder_at(source, to + usize::from(from < to));
    }

    fn reorder_at(&self, source: u64, insertion: usize) {
        if self.blocked() {
            return;
        }
        let mut tabs = self.tabs.borrow_mut();
        let Some(from) = tabs.iter().position(|tab| tab.id == source) else {
            return;
        };
        if insertion > tabs.len() {
            return;
        }
        let to = insertion - usize::from(from < insertion);
        if from == to {
            return;
        }
        let tab = tabs.remove(from);
        tabs.insert(to, tab);
        self.strip
            .reorder(&tabs.iter().map(|tab| tab.id).collect::<Vec<_>>());
        self.strip.hints(self.hints.get());
        drop(tabs);
        self.persist_session();
    }

    fn move_active_tab(&self, delta: i32) {
        let tabs = self.tabs.borrow();
        let Some(current) = tabs.iter().position(|tab| tab.id == self.active.get()) else {
            return;
        };
        let target = current
            .checked_add_signed(delta as isize)
            .and_then(|index| tabs.get(index))
            .map(|tab| tab.id);
        drop(tabs);
        if let Some(target) = target {
            self.reorder(self.active.get(), target);
        }
    }

    fn show_hints(&self, show: bool) {
        if self.hints.replace(show) != show {
            self.strip.hints(show);
        }
    }

    fn persist_session(&self) {
        if !self.persist || self.restoring.get() {
            return;
        }
        if !self.preferences.restore_tabs() {
            tabs_session::remove();
            return;
        }
        let tabs = self.tabs.borrow();
        let mut locations = Vec::with_capacity(tabs.len());
        let mut active = 0;
        for tab in tabs.iter() {
            let Some(location) = tab.location() else {
                continue;
            };
            if tab.id == self.active.get() {
                active = locations.len();
            }
            locations.push(location);
        }
        drop(tabs);
        if locations.is_empty() {
            return;
        }
        tabs_session::save(&locations, active);
    }

    pub(in crate::ui::window) fn try_restore(self: &Rc<Self>) -> bool {
        if !self.preferences.restore_tabs() {
            return false;
        }
        let Some(session) = tabs_session::load_restorable() else {
            return false;
        };
        if session.tabs.is_empty() {
            return false;
        }
        self.restore_session(session);
        true
    }

    fn restore_session(self: &Rc<Self>, session: tabs_session::RestoredSession) {
        if session.tabs.is_empty() || self.blocked() {
            return;
        }
        self.restoring.set(true);
        let active = session.active.min(session.tabs.len() - 1);
        for (index, location) in session.tabs.into_iter().enumerate() {
            if index > 0 {
                self.add(None);
            }
            self.reopen(&self.active_tab(), location);
        }
        self.select_index(active);
        self.restoring.set(false);
    }

    fn reopen(self: &Rc<Self>, tab: &Tab, location: Location) {
        if location.native_path().is_none() {
            tab.content.browser.navigate_location(location);
            return;
        }
        self.strip.label(tab.id, &tab_label(Some(&location)));
        tab.checking.replace(Some(location.clone()));
        let weak = Rc::downgrade(self);
        let id = tab.id;
        super::super::open_checked(&tab.content.browser.browser(), location, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let tab = state.tabs.borrow().iter().find(|tab| tab.id == id).cloned();
            let Some(tab) = tab else {
                return;
            };
            tab.checking.take();
            if state.tabs.borrow().len() > 1 && !state.blocked() {
                state.discard(id);
            } else {
                super::super::open_default_folder(
                    &tab.content.browser.browser(),
                    &state.preferences,
                );
            }
        });
    }

    fn handle_key(self: &Rc<Self>, press: &KeyPress) -> glib::Propagation {
        if press.is(ESCAPE_ANY) && self.strip.cancel_drag() {
            return glib::Propagation::Stop;
        }
        // GDK reports a modifier key's state from before its own press.
        self.show_hints((press.modifiers() | press.modifier_key()) == TAB_NUMBERS.modifiers());
        if self.blocked() || !is_tab_shortcut(press) {
            return glib::Propagation::Proceed;
        }
        self.active_tab().content.footer.shortcuts.cancel_chord();
        if press.is(TAB_NEW) {
            self.new_tab();
        } else if press.is(TAB_CLOSE) {
            self.close(self.active.get());
        } else if press.is(TAB_REOPEN) {
            self.reopen_closed_tab();
        } else if press.is(TAB_NEXT) {
            self.cycle(1);
        } else if press.is(TAB_PREVIOUS) {
            self.cycle(-1);
        } else if press.is(TAB_MOVE_LEFT) {
            self.move_active_tab(-1);
        } else if press.is(TAB_MOVE_RIGHT) {
            self.move_active_tab(1);
        } else if let Some(digit) = tab_number(press) {
            self.select_index(tab_index(digit));
        }
        glib::Propagation::Stop
    }

    fn install_keys(self: &Rc<Self>, window: &gtk::ApplicationWindow) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys::on_press(&keys, move |_, press| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |state| state.handle_key(press))
        });
        let weak = Rc::downgrade(self);
        keys::on_release(&keys, move |_, press| {
            if let Some(state) = weak.upgrade() {
                state.show_hints(
                    press.modifiers().difference(press.modifier_key()) == TAB_NUMBERS.modifiers(),
                );
            }
        });
        window.add_controller(keys);
    }
}

fn operations_active() -> crate::ui::close_guard::CloseBlocker {
    crate::ui::close_guard::CloseBlocker {
        title: crate::i18n::tr("File operations are still active"),
        detail: crate::i18n::tr(
            "Wait for these operations to finish, or cancel them before closing this tab or window. Cancellation does not undo completed changes.",
        ),
    }
}

pub(in crate::ui::window) fn is_tab_shortcut(press: &KeyPress) -> bool {
    press.is_any(&[
        TAB_NEW,
        TAB_CLOSE,
        TAB_REOPEN,
        TAB_NEXT,
        TAB_PREVIOUS,
        TAB_MOVE_LEFT,
        TAB_MOVE_RIGHT,
    ]) || tab_number(press).is_some()
}

/// A tab digit, unless the key types the `+` or `−` of a text size shortcut,
/// as the 1 key does with Shift on Swiss German. Only the primary enlarging
/// key counts, so German Shift+0, which types `=`, still selects tab 10.
fn tab_number(press: &KeyPress) -> Option<u8> {
    if TEXT_LARGER
        .primary()
        .is_some_and(|primary| press.is(primary))
        || press.is(TEXT_SMALLER)
    {
        return None;
    }
    press.digit_in(TAB_NUMBERS)
}

fn action_location(parameter: Option<&glib::Variant>) -> Option<Location> {
    let uri = parameter?.get::<String>()?;
    location_for_file(&gio::File::for_uri(&uri))
}

/// The tab a digit selects: 1 is the first tab and 0 the tenth.
pub(in crate::ui::window) fn tab_index(digit: u8) -> usize {
    (usize::from(digit) + 9) % 10
}

fn tab_label(location: Option<&Location>) -> String {
    let Some(location) = location else {
        return crate::i18n::tr("Home");
    };
    if let Some(path) = location.native_path() {
        if Some(path)
            == std::env::var_os("HOME")
                .as_deref()
                .map(std::path::Path::new)
        {
            return "~".into();
        }
        return path
            .file_name()
            .map_or_else(|| "/".into(), |name| name.to_string_lossy().into_owned());
    }
    let uri = location.uri_value().unwrap_or_default();
    match uri {
        "trash:///" => crate::i18n::tr("Trash"),
        "recent:///" => crate::i18n::tr("Recent"),
        "network:///" => crate::i18n::tr("Network"),
        _ => gio::File::for_uri(uri).basename().map_or_else(
            || crate::i18n::tr("Location"),
            |name| name.to_string_lossy().into_owned(),
        ),
    }
}
