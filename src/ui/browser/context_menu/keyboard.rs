// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk::Key, glib, prelude::*};

pub(super) struct NativeMenuNavigation {
    popover: glib::WeakRef<gtk::PopoverMenu>,
    initial_focus: Cell<bool>,
    focus_queued: Cell<bool>,
    pointer_position: Cell<Option<(f64, f64)>>,
    keyboard_owned: Cell<bool>,
    pointer_controllers: RefCell<Vec<(glib::WeakRef<gtk::Widget>, gtk::EventController)>>,
}

impl NativeMenuNavigation {
    pub(super) fn new(popover: &gtk::PopoverMenu) -> Rc<Self> {
        let navigation = Rc::new(Self {
            popover: popover.downgrade(),
            initial_focus: Cell::new(false),
            focus_queued: Cell::new(false),
            pointer_position: Cell::new(None),
            keyboard_owned: Cell::new(false),
            pointer_controllers: RefCell::new(Vec::new()),
        });
        install_menu_edges(popover, &navigation);
        let state = navigation.clone();
        popover.connect_move_focus(move |_, _| state.keyboard_navigation());
        let state = navigation.clone();
        popover.connect_closed(move |_| state.pointer_navigation());
        let pointer = gtk::EventControllerMotion::new();
        pointer.set_name(Some("strata-context-pointer"));
        pointer.set_propagation_phase(gtk::PropagationPhase::Capture);
        pointer.set_propagation_limit(gtk::PropagationLimit::None);
        let state = navigation.clone();
        pointer.connect_enter(move |_, _, _| state.pointer_motion());
        let state = navigation.clone();
        pointer.connect_motion(move |_, _, _| state.pointer_motion());
        popover.add_controller(pointer);
        let state = navigation.clone();
        let input = gtk::EventControllerLegacy::new();
        input.set_propagation_phase(gtk::PropagationPhase::Capture);
        input.set_propagation_limit(gtk::PropagationLimit::None);
        input.connect_event(move |_, event| {
            if matches!(
                event.event_type(),
                gtk::gdk::EventType::ButtonPress | gtk::gdk::EventType::TouchBegin
            ) {
                state.pointer_navigation();
            }
            glib::Propagation::Proceed
        });
        popover.add_controller(input);
        navigation
    }

    pub(super) fn begin(&self) {
        self.initial_focus.set(true);
        self.keyboard_owned.set(true);
        self.pointer_position.set(self.pointer_position());
    }

    fn pointer_position(&self) -> Option<(f64, f64)> {
        let popover = self.popover.upgrade()?;
        let window = popover.root()?.downcast::<gtk::Window>().ok()?;
        let pointer = WidgetExt::display(&window).default_seat()?.pointer()?;
        let root = window.surface()?;
        let (surface, mut x, mut y) = pointer.surface_at_position();
        let mut surface = surface?;
        // A popup crossing is not mouse movement. Compare in the same
        // toplevel coordinates on both X11 and Wayland.
        while surface != root {
            let popup = surface.downcast::<gtk::gdk::Popup>().ok()?;
            x += f64::from(popup.position_x());
            y += f64::from(popup.position_y());
            surface = popup.parent()?;
        }
        Some((x, y))
    }

    fn keyboard_navigation(&self) {
        self.initial_focus.set(false);
        self.keyboard_owned.set(true);
        self.pointer_position.set(self.pointer_position());
        if let Some(popover) = self.popover.upgrade() {
            self.suspend_native_pointer(popover.upcast_ref());
        }
    }

    fn pointer_motion(&self) {
        let position = self.pointer_position();
        if position != self.pointer_position.replace(position) {
            self.pointer_navigation();
        }
    }

    fn pointer_navigation(&self) {
        self.initial_focus.set(false);
        self.keyboard_owned.set(false);
        for (widget, controller) in self.pointer_controllers.take() {
            if let Some(widget) = widget.upgrade() {
                widget.add_controller(controller);
            }
        }
    }

    fn suspend_native_pointer(&self, widget: &gtk::Widget) {
        let controllers = widget.observe_controllers();
        let motion = (0..controllers.n_items())
            .filter_map(|index| {
                controllers
                    .item(index)
                    .and_downcast::<gtk::EventControllerMotion>()
            })
            .filter(|controller| controller.name().as_deref() != Some("strata-context-pointer"))
            .collect::<Vec<_>>();
        for controller in motion {
            // GTK delivers pointer crossings even with propagation phase None.
            widget.remove_controller(&controller);
            self.pointer_controllers
                .borrow_mut()
                .push((widget.downgrade(), controller.upcast()));
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            self.suspend_native_pointer(&widget);
            child = widget.next_sibling();
        }
    }

    pub(super) fn model_changed(self: &Rc<Self>) {
        self.pointer_controllers
            .borrow_mut()
            .retain(|(widget, _)| widget.upgrade().is_some());
        if self.keyboard_owned.get()
            && let Some(popover) = self.popover.upgrade()
        {
            self.suspend_native_pointer(popover.upcast_ref());
        }
        if !self.initial_focus.get() || self.focus_queued.replace(true) {
            return;
        }
        let Some(popover) = self.popover.upgrade() else {
            self.focus_queued.set(false);
            return;
        };
        let state = self.clone();
        // Wait for native rows, their action state and popup pointer crossing
        // to settle. Later MIME updates may introduce an earlier menu item.
        popover.add_tick_callback(move |_, _| {
            let state = state.clone();
            glib::idle_add_local_once(move || {
                state.focus_queued.set(false);
                if state.initial_focus.get()
                    && let Some(popover) = state.popover.upgrade()
                    && popover.is_mapped()
                {
                    focus_first_or_last_menu_item(popover.upcast_ref(), true);
                }
            });
            glib::ControlFlow::Break
        });
    }
}

fn install_menu_edges(popover: &gtk::PopoverMenu, navigation: &Rc<NativeMenuNavigation>) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.set_propagation_limit(gtk::PropagationLimit::None);
    let weak = popover.downgrade();
    let navigation = navigation.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        navigation.keyboard_navigation();
        if !modifiers.is_empty() || !matches!(key, Key::Home | Key::End) {
            return glib::Propagation::Proceed;
        }
        let Some(popover) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let active = popover
            .root()
            .and_then(|root| root.focus())
            .and_then(|focus| focus.ancestor(gtk::Popover::static_type()))
            .and_downcast::<gtk::Popover>()
            .unwrap_or_else(|| popover.clone().upcast());
        focus_first_or_last_menu_item(&active, key == Key::Home);
        glib::Propagation::Stop
    });
    popover.add_controller(keys);
}

pub(super) fn install_submenu_return(
    submenu: &gtk::PopoverMenu,
    owner: &gtk::Widget,
    navigation: &Rc<NativeMenuNavigation>,
) {
    let weak_owner = owner.downgrade();
    let navigation = navigation.clone();
    // Native shortcuts emit move-focus before ordinary key controllers run.
    submenu.connect_move_focus(move |submenu, direction| {
        navigation.keyboard_navigation();
        if direction != gtk::DirectionType::Left {
            return;
        }
        submenu.stop_signal_emission_by_name("move-focus");
        // Hide without cascading, then let GTK clear its open-submenu state.
        // Focusing the owner alone leaves arrows routed to the hidden branch.
        submenu.set_visible(false);
        if let Some(owner) = weak_owner.upgrade() {
            if let Some(parent) = owner.ancestor(gtk::PopoverMenu::static_type()) {
                parent.child_focus(gtk::DirectionType::Left);
            }
            owner.grab_focus();
        }
    });
}

pub(super) fn install(popover: &gtk::Popover) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = popover.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(popover) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if modifiers.intersects(
            gtk::accelerator_get_default_mod_mask() & !gtk::gdk::ModifierType::SHIFT_MASK,
        ) {
            return glib::Propagation::Stop;
        }
        // Captured menu keys bypass GTK's focus-indicator timeout refresh.
        if let Some(window) = popover.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        match key {
            Key::Escape => popover.popdown(),
            Key::Home => focus_first_or_last_menu_item(&popover, true),
            Key::End => focus_first_or_last_menu_item(&popover, false),
            Key::Up | Key::Down | Key::Tab | Key::ISO_Left_Tab => {
                let forward = key == Key::Down
                    || (key == Key::Tab && !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK));
                let buttons = menu_buttons(&popover);
                if !buttons.is_empty() {
                    let current = buttons.iter().position(|button| button.has_focus());
                    let next = match (current, forward) {
                        (Some(index), true) => (index + 1) % buttons.len(),
                        (Some(index), false) => (index + buttons.len() - 1) % buttons.len(),
                        (None, true) => 0,
                        (None, false) => buttons.len() - 1,
                    };
                    buttons[next].grab_focus();
                }
            }
            Key::Return | Key::KP_Enter | Key::space => {
                if let Some(button) = menu_buttons(&popover)
                    .iter()
                    .find(|button| button.has_focus())
                {
                    // activate() waits for a key release that this controller consumes.
                    if let Some(button) = button.downcast_ref::<gtk::Button>() {
                        button.emit_clicked();
                    } else {
                        button.activate();
                    }
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    popover.add_controller(keys);
}

pub(super) fn focus_first_or_last_menu_item(popover: &gtk::Popover, first: bool) {
    let buttons = menu_buttons(popover);
    if let Some(button) = if first {
        buttons.first()
    } else {
        buttons.last()
    } {
        button.grab_focus();
    }
}

fn menu_buttons(popover: &gtk::Popover) -> Vec<gtk::Widget> {
    let mut buttons = Vec::new();
    collect_buttons(popover.upcast_ref(), popover, &mut buttons);
    buttons
}

fn collect_buttons(widget: &gtk::Widget, popover: &gtk::Popover, buttons: &mut Vec<gtk::Widget>) {
    if !widget.is_visible()
        || !widget.is_sensitive()
        || (widget.is::<gtk::Popover>() && widget != popover.upcast_ref::<gtk::Widget>())
    {
        return;
    }
    if widget.is::<gtk::Button>()
        || (widget.is_focusable() && widget.accessible_role() == gtk::AccessibleRole::MenuItem)
    {
        buttons.push(widget.clone());
        return;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        collect_buttons(&widget, popover, buttons);
        child = widget.next_sibling();
    }
}
