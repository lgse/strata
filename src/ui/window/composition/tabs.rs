// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    model::Location,
    ui::{blur::BlurBin, browser::BrowserView, preferences::PreferenceManager},
};

use super::{WindowContent, layout, tenxer_splash};

mod strip;
use strip::TabStrip;

#[cfg(test)]
mod tests;

struct Tab {
    id: u64,
    content: Rc<WindowContent>,
    focus: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
}

pub(in crate::ui::window) struct TabWindow {
    window: glib::WeakRef<gtk::ApplicationWindow>,
    preferences: Rc<PreferenceManager>,
    stack: gtk::Stack,
    drag_token: String,
    strip: TabStrip,
    tabs: RefCell<Vec<Rc<Tab>>>,
    active: Cell<u64>,
    next_id: Cell<u64>,
    hints: Cell<bool>,
}

impl TabWindow {
    pub(in crate::ui::window) fn new(
        window: &gtk::ApplicationWindow,
        preferences: &Rc<PreferenceManager>,
    ) -> Rc<Self> {
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
            active: Cell::new(0),
            next_id: Cell::new(1),
            hints: Cell::new(false),
            drag_token: glib::uuid_string_random().to_string(),
        });
        state.add(None);
        state.install_actions(window);
        state.install_keys(window);
        super::super::install_modal_focus_trap(window);
        tenxer_splash::install(window, &overlay, preferences);
        let weak = Rc::downgrade(&state);
        window.connect_close_request(move |window| {
            let Some(state) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if state
                .tabs
                .borrow()
                .iter()
                .any(|tab| tab.content.browser.browser().has_background_operations())
            {
                operations_active(window);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
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
                "previous-tab",
                "select-tab",
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
        state
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
        let location = self.active_browser().browser().active_location();
        self.add(location.or_else(|| Some(super::super::startup_location(&self.preferences))));
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
        let weak = Rc::downgrade(self);
        content.browser.observe_tab_location(move |location| {
            if let Some(state) = weak.upgrade() {
                state.strip.label(id, &tab_label(location));
            }
        });
        if id == 1 {
            super::super::schedule_after_first_paint(&window, &content.sidebar, &self.preferences);
        } else {
            content.sidebar.schedule_after_first_paint(&content.overlay);
        }
        self.select(id);
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
            *previous.focus.borrow_mut() =
                gtk::prelude::RootExt::focus(&window).map(|widget| widget.downgrade());
            previous.content.footer.shortcuts.cancel_chord();
            previous.content.browser.cancel_location_edit();
        }
        self.active.set(id);
        self.stack.set_visible_child(&tab.content.overlay);
        tab.content.activate_actions(&window);
        self.refresh_chrome();
        self.strip.select(id);
        if let Some(focus) = tab
            .focus
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
            .filter(|widget| widget.is_mapped())
        {
            focus.grab_focus();
        } else {
            tab.content.browser.browser().focus_active();
        }
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
            operations_active(&window);
            return;
        }
        if tabs.len() == 1 {
            drop(tabs);
            window.close();
            return;
        }
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
    }

    fn refresh_chrome(&self) {
        let multiple = self.tabs.borrow().len() > 1;
        self.strip.widget().set_visible(multiple);
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
        if source == target || self.blocked() {
            return;
        }
        let mut tabs = self.tabs.borrow_mut();
        let Some(from) = tabs.iter().position(|tab| tab.id == source) else {
            return;
        };
        let Some(to) = tabs.iter().position(|tab| tab.id == target) else {
            return;
        };
        let tab = tabs.remove(from);
        tabs.insert(to, tab);
        self.strip
            .reorder(&tabs.iter().map(|tab| tab.id).collect::<Vec<_>>());
        self.strip.hints(self.hints.get());
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

    fn handle_key(
        self: &Rc<Self>,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> glib::Propagation {
        use gdk::{Key, ModifierType as M};
        let mods = modifiers
            & (M::CONTROL_MASK
                | M::SHIFT_MASK
                | M::ALT_MASK
                | M::SUPER_MASK
                | M::META_MASK
                | M::HYPER_MASK);
        let ctrl_shift = M::CONTROL_MASK | M::SHIFT_MASK;
        let held = mods
            | match key {
                Key::Control_L | Key::Control_R => M::CONTROL_MASK,
                Key::Shift_L | Key::Shift_R => M::SHIFT_MASK,
                _ => M::empty(),
            };
        self.show_hints(held == ctrl_shift);
        if self.blocked() || !is_tab_shortcut(key, modifiers) {
            return glib::Propagation::Proceed;
        }
        self.active_tab().content.footer.shortcuts.cancel_chord();
        if mods == M::CONTROL_MASK && matches!(key, Key::t | Key::T) {
            self.new_tab();
        } else if mods == M::CONTROL_MASK && matches!(key, Key::w | Key::W) {
            self.close(self.active.get());
        } else if mods == M::CONTROL_MASK
            && matches!(key, Key::Tab | Key::Page_Down | Key::KP_Page_Down)
        {
            self.cycle(1);
        } else if (mods == M::CONTROL_MASK && matches!(key, Key::Page_Up | Key::KP_Page_Up))
            || (mods == ctrl_shift && matches!(key, Key::Tab | Key::ISO_Left_Tab))
        {
            self.cycle(-1);
        } else if mods == ctrl_shift
            && let Some(delta) = super::super::page_direction(key)
        {
            self.move_active_tab(delta);
        } else if mods == ctrl_shift
            && let Some(index) = tab_index(key)
        {
            self.select_index(index);
        } else {
            return glib::Propagation::Proceed;
        }
        glib::Propagation::Stop
    }

    fn install_keys(self: &Rc<Self>, window: &gtk::ApplicationWindow) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            weak.upgrade().map_or(glib::Propagation::Proceed, |state| {
                state.handle_key(key, modifiers)
            })
        });
        let weak = Rc::downgrade(self);
        keys.connect_key_released(move |_, key, _, modifiers| {
            if let Some(state) = weak.upgrade() {
                use gdk::{Key, ModifierType as M};
                let mut mods = modifiers;
                if matches!(key, Key::Control_L | Key::Control_R) {
                    mods.remove(M::CONTROL_MASK);
                }
                if matches!(key, Key::Shift_L | Key::Shift_R) {
                    mods.remove(M::SHIFT_MASK);
                }
                state.show_hints(mods.contains(M::CONTROL_MASK | M::SHIFT_MASK));
            }
        });
        window.add_controller(keys);
    }
}

fn operations_active(window: &gtk::ApplicationWindow) {
    crate::ui::modal::show_error_dialog(
        window,
        "File operations are still active",
        "Wait for these operations to finish, or cancel them before closing this tab or window. Cancellation does not undo completed changes.",
    );
}

pub(in crate::ui::window) fn is_tab_shortcut(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    use gdk::{Key, ModifierType as M};
    let mods = modifiers
        & (M::CONTROL_MASK
            | M::SHIFT_MASK
            | M::ALT_MASK
            | M::SUPER_MASK
            | M::META_MASK
            | M::HYPER_MASK);
    (mods == M::CONTROL_MASK
        && matches!(
            key,
            Key::t
                | Key::T
                | Key::w
                | Key::W
                | Key::Tab
                | Key::Page_Up
                | Key::KP_Page_Up
                | Key::Page_Down
                | Key::KP_Page_Down
        ))
        || (mods == (M::CONTROL_MASK | M::SHIFT_MASK)
            && (matches!(key, Key::Tab | Key::ISO_Left_Tab)
                || super::super::page_direction(key).is_some()
                || tab_index(key).is_some()))
}

pub(in crate::ui::window) fn tab_index(key: gdk::Key) -> Option<usize> {
    use gdk::Key;
    Some(match key {
        Key::_1 | Key::exclam | Key::KP_1 => 0,
        Key::_2 | Key::at | Key::KP_2 => 1,
        Key::_3 | Key::numbersign | Key::KP_3 => 2,
        Key::_4 | Key::dollar | Key::KP_4 => 3,
        Key::_5 | Key::percent | Key::KP_5 => 4,
        Key::_6 | Key::asciicircum | Key::KP_6 => 5,
        Key::_7 | Key::ampersand | Key::KP_7 => 6,
        Key::_8 | Key::asterisk | Key::KP_8 => 7,
        Key::_9 | Key::parenleft | Key::KP_9 => 8,
        Key::_0 | Key::parenright | Key::KP_0 => 9,
        _ => return None,
    })
}

fn tab_label(location: Option<&Location>) -> String {
    let Some(location) = location else {
        return "Home".into();
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
        "trash:///" => "Trash".into(),
        "recent:///" => "Recent".into(),
        "network:///" => "Network".into(),
        _ => gio::File::for_uri(uri).basename().map_or_else(
            || "Location".into(),
            |name| name.to_string_lossy().into_owned(),
        ),
    }
}
