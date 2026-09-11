// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
    prelude::*,
};

use crate::{
    app::Browser,
    ui::{
        browser::BrowserView, preview::PreviewDrawer, shortcut_footer::ShortcutFooter,
        top_bar_navigation::TopBarNavigation,
    },
};

use super::{SidebarState, SidebarView, TypeToSearch, visible_modal_layer};

mod commands;
mod focus;
mod items;

// None tries the next Strata stage; Some(Proceed) gives the event to GTK instead.
type KeyResult = Option<Propagation>;

pub(super) struct Bindings {
    pub view: BrowserView,
    pub top_bar: TopBarNavigation,
    pub preview: PreviewDrawer,
    pub type_to_search: TypeToSearch,
    pub shortcuts: ShortcutFooter,
}

pub(super) fn install(window: &gtk::ApplicationWindow, sidebar: &SidebarView, bindings: Bindings) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_browser = Rc::downgrade(&bindings.view.browser());
    let dispatcher = Dispatcher {
        window: window.clone(),
        view: bindings.view,
        top_bar: bindings.top_bar,
        preview: bindings.preview,
        type_to_search: bindings.type_to_search,
        shortcuts: bindings.shortcuts,
        sidebar: SidebarFocus {
            state: sidebar.state.clone(),
            widget: sidebar.widget.clone(),
            previous: RefCell::new(None),
        },
    };
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(browser) = weak_browser.upgrade() else {
            return Propagation::Proceed;
        };
        dispatcher.handle_key(&browser, key, modifiers)
    });
    window.add_controller(keys);
}

struct Dispatcher {
    window: gtk::ApplicationWindow,
    view: BrowserView,
    sidebar: SidebarFocus,
    top_bar: TopBarNavigation,
    preview: PreviewDrawer,
    type_to_search: TypeToSearch,
    shortcuts: ShortcutFooter,
}

struct KeyEvent {
    key: Key,
    modifiers: Modifiers,
    focused: Option<gtk::Widget>,
    vim_navigation: bool,
    header_left_boundary: bool,
}

impl KeyEvent {
    fn control(&self) -> bool {
        self.modifiers.contains(Modifiers::CONTROL_MASK)
    }

    fn shift(&self) -> bool {
        self.modifiers.contains(Modifiers::SHIFT_MASK)
    }

    fn alt(&self) -> bool {
        self.modifiers.contains(Modifiers::ALT_MASK)
    }

    fn without(&self, modifiers: Modifiers) -> bool {
        !self.modifiers.intersects(modifiers)
    }

    fn text_has_focus(&self) -> bool {
        self.focused.as_ref().is_some_and(|widget| {
            widget.is::<gtk::Text>() || widget.is::<gtk::TextView>() || widget.is::<gtk::Entry>()
        })
    }
}

impl Dispatcher {
    fn handle_key(&self, browser: &Rc<Browser>, key: Key, modifiers: Modifiers) -> Propagation {
        if let Some(result) = self.input_owner(key, modifiers) {
            return result;
        }
        let focused = gtk::prelude::RootExt::focus(&self.window);
        let navigation_key = crate::ui::focus_navigation::navigation_key(
            key,
            modifiers,
            self.type_to_search.preferences.type_to_search(),
            focused.as_ref(),
        );
        let mut event = KeyEvent {
            key: navigation_key,
            modifiers,
            focused,
            vim_navigation: navigation_key != key,
            header_left_boundary: false,
        };
        self.window_commands(&event)
            .or_else(|| self.inline_editing(&event))
            .or_else(|| self.filter_and_location_commands(&event))
            .or_else(|| self.video_controls(&event))
            .or_else(|| self.sidebar_commands(browser, &event))
            .or_else(|| {
                // Search rows own navigation; directory commands must not act on hidden selections.
                (self.view.selected_search_results().is_some() && !event.text_has_focus())
                    .then_some(Propagation::Proceed)
            })
            .or_else(|| self.text_input(&event))
            .or_else(|| self.file_commands(browser, &event))
            .or_else(|| self.focus_navigation(browser, &mut event))
            .or_else(|| self.dismissal(browser, &event))
            .or_else(|| self.item_navigation(browser, &event))
            .unwrap_or(Propagation::Proceed)
    }

    fn input_owner(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        if let Some(layer) = visible_modal_layer(&self.window) {
            let focus_is_inside = gtk::prelude::RootExt::focus(&self.window)
                .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
            if !focus_is_inside {
                layer.grab_focus();
                return Some(Propagation::Stop);
            }
            return Some(Propagation::Proceed);
        }
        if !self.inline_editing_active()
            && let Some(result) = self.shortcuts.handle_key(key, modifiers)
        {
            return Some(result);
        }
        if key == Key::Escape && crate::ui::scrolling::stop_autoscroll() {
            return Some(Propagation::Stop);
        }
        None
    }

    fn inline_editing_active(&self) -> bool {
        self.view.rename_is_active() || self.view.new_entry_is_active()
    }
}

struct SidebarFocus {
    state: Rc<SidebarState>,
    widget: gtk::Widget,
    previous: RefCell<Option<gtk::Widget>>,
}

impl SidebarFocus {
    fn contains(&self, focused: &Option<gtk::Widget>) -> bool {
        focused
            .as_ref()
            .is_some_and(|widget| widget == &self.widget || widget.is_ancestor(&self.widget))
    }

    fn enter(&self, focused: &Option<gtk::Widget>) {
        self.previous.replace(focused.clone());
        self.state.focus_active_place();
    }

    fn restore(&self, browser: &Browser, require_mapped: bool) {
        let restored =
            self.previous.borrow_mut().take().is_some_and(|widget| {
                (!require_mapped || widget.is_mapped()) && widget.grab_focus()
            });
        if !restored {
            browser.focus_active();
        }
    }
}
