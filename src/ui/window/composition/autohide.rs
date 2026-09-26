// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

use gtk::{glib, prelude::*};

use crate::ui::{browser::BrowserView, motion::animations_enabled, preferences::PreferenceManager};

use super::super::{MIN_SIDEBAR_WIDTH, SidebarState, SidebarView, preferred_sidebar_width};
use super::layout::{BrowserLayout, Header};

const EDGE_SIZE: i32 = 6;
const HIDE_DELAY: Duration = Duration::from_millis(350);
const TRANSITION_MS: u32 = 180;

type RevealRequest = Box<dyn Fn(&FloatingPanel, bool)>;

/// A panel that floats over the content and slides in from a window edge.
struct FloatingPanel {
    edge: gtk::Box,
    revealer: gtk::Revealer,
    frame: gtk::Box,
    browser: BrowserView,
    pointer: RefCell<Vec<gtk::EventController>>,
    /// Revealed by keyboard or button rather than hover: focus inside keeps it open.
    sticky: Cell<bool>,
    hide_timer: RefCell<Option<glib::SourceId>>,
    hold: Box<dyn Fn() -> bool>,
    request: RefCell<Option<RevealRequest>>,
}

impl FloatingPanel {
    /// The caller stacks `edge` and `revealer` into the stage overlay.
    fn new(
        edge: gtk::PositionType,
        browser: &BrowserView,
        hold: impl Fn() -> bool + 'static,
    ) -> Rc<Self> {
        let horizontal = edge == gtk::PositionType::Left;
        let hot_zone = gtk::Box::new(gtk::Orientation::Vertical, 0);
        hot_zone.add_css_class("autohide-edge");
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let revealer = gtk::Revealer::builder()
            .child(&frame)
            .reveal_child(false)
            .visible(false)
            .build();
        hot_zone.set_visible(false);
        if horizontal {
            hot_zone.set_width_request(EDGE_SIZE);
            hot_zone.set_halign(gtk::Align::Start);
            hot_zone.set_valign(gtk::Align::Fill);
            frame.add_css_class("autohide-sidebar");
            revealer.set_transition_type(gtk::RevealerTransitionType::SlideRight);
            revealer.set_halign(gtk::Align::Start);
            revealer.set_valign(gtk::Align::Fill);
        } else {
            hot_zone.set_height_request(EDGE_SIZE);
            hot_zone.set_halign(gtk::Align::Fill);
            hot_zone.set_valign(gtk::Align::Start);
            frame.add_css_class("autohide-header");
            revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
            revealer.set_halign(gtk::Align::Fill);
            revealer.set_valign(gtk::Align::Start);
        }
        let panel = Rc::new(Self {
            edge: hot_zone,
            revealer,
            frame,
            browser: browser.clone(),
            pointer: RefCell::new(Vec::new()),
            sticky: Cell::new(false),
            hide_timer: RefCell::new(None),
            hold: Box::new(hold),
            request: RefCell::new(None),
        });
        panel.install_pointer_tracking();
        panel
    }

    fn install_pointer_tracking(self: &Rc<Self>) {
        for (widget, reveals) in [
            (self.edge.upcast_ref::<gtk::Widget>(), true),
            (self.revealer.upcast_ref::<gtk::Widget>(), false),
        ] {
            let motion = gtk::EventControllerMotion::new();
            let weak = Rc::downgrade(self);
            motion.connect_enter(move |_, _, _| {
                if let Some(panel) = weak.upgrade() {
                    panel.pointer_entered(reveals);
                }
            });
            let weak = Rc::downgrade(self);
            motion.connect_leave(move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.pointer_left();
                }
            });
            widget.add_controller(motion.clone());
            self.pointer.borrow_mut().push(motion.upcast());
            // Dragging files toward an edge reveals the panel so they can be dropped on it.
            let drop_motion = gtk::DropControllerMotion::new();
            let weak = Rc::downgrade(self);
            drop_motion.connect_enter(move |_, _, _| {
                if let Some(panel) = weak.upgrade() {
                    panel.pointer_entered(reveals);
                }
            });
            let weak = Rc::downgrade(self);
            drop_motion.connect_leave(move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.pointer_left();
                }
            });
            widget.add_controller(drop_motion.clone());
            self.pointer.borrow_mut().push(drop_motion.upcast());
        }
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_leave(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.schedule_hide();
            }
        });
        self.frame.add_controller(focus);
    }

    fn set_request(&self, request: impl Fn(&FloatingPanel, bool) + 'static) {
        self.request.replace(Some(Box::new(request)));
    }

    fn enabled(&self) -> bool {
        self.revealer.is_visible()
    }

    fn set_enabled(&self, enabled: bool) {
        self.cancel_hide();
        self.sticky.set(false);
        self.edge.set_visible(enabled);
        self.revealer.set_visible(enabled);
        self.revealer.set_reveal_child(false);
    }

    fn revealed(&self) -> bool {
        self.revealer.reveals_child()
    }

    fn pointer_entered(&self, reveals: bool) {
        if !self.enabled() {
            return;
        }
        self.cancel_hide();
        if reveals && !self.revealed() {
            self.request_reveal(true);
        }
    }

    fn pointer_left(self: &Rc<Self>) {
        self.schedule_hide();
    }

    fn hovered(&self) -> bool {
        self.pointer.borrow().iter().any(|controller| {
            controller
                .downcast_ref::<gtk::EventControllerMotion>()
                .map(|motion| motion.contains_pointer())
                .or_else(|| {
                    controller
                        .downcast_ref::<gtk::DropControllerMotion>()
                        .map(|motion| motion.contains_pointer())
                })
                .unwrap_or(false)
        })
    }

    fn request_reveal(&self, revealed: bool) {
        if let Some(request) = self.request.borrow().as_ref() {
            request(self, revealed);
        }
    }

    fn add_to(&self, stage: &gtk::Overlay) {
        stage.add_overlay(&self.edge);
    }

    /// Shows or hides the panel; `sticky` marks a reveal that did not come from hovering.
    fn apply(&self, revealed: bool, sticky: bool) {
        if !self.enabled() {
            return;
        }
        self.cancel_hide();
        if revealed {
            self.sticky.set(sticky);
        } else if self.contains_focus() {
            self.browser.browser().focus_active();
        }
        self.revealer
            .set_transition_duration(if animations_enabled() {
                TRANSITION_MS
            } else {
                0
            });
        self.revealer.set_reveal_child(revealed);
    }

    fn contains_focus(&self) -> bool {
        self.frame
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| focus.is_ancestor(&self.frame))
    }

    fn held(&self) -> bool {
        self.hovered()
            || has_open_popover(self.frame.upcast_ref())
            || (self.sticky.get() && self.contains_focus())
            || (self.hold)()
    }

    fn schedule_hide(self: &Rc<Self>) {
        if !self.enabled() || !self.revealed() {
            return;
        }
        self.cancel_hide();
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(HIDE_DELAY, move || {
            let Some(panel) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !panel.revealed() {
                panel.hide_timer.replace(None);
                return glib::ControlFlow::Break;
            }
            // Popovers and focus can go away without a crossing event, so keep polling.
            if panel.hovered() {
                panel.hide_timer.replace(None);
                return glib::ControlFlow::Break;
            }
            if panel.held() {
                return glib::ControlFlow::Continue;
            }
            panel.hide_timer.replace(None);
            panel.request_reveal(false);
            glib::ControlFlow::Break
        });
        self.hide_timer.replace(Some(source));
    }

    fn cancel_hide(&self) {
        if let Some(source) = self.hide_timer.take() {
            source.remove();
        }
    }
}

fn has_open_popover(widget: &gtk::Widget) -> bool {
    let mut child = widget.first_child();
    while let Some(current) = child {
        if (current.is::<gtk::Popover>() && current.is_visible()) || has_open_popover(&current) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

pub(super) fn install(
    layout: &BrowserLayout,
    header: &Header,
    sidebar: &SidebarView,
    browser: &BrowserView,
    preferences: &PreferenceManager,
) {
    let header_panel = install_header(layout, header, browser, preferences);
    let sidebar_panel = install_sidebar(layout, header, sidebar, browser, preferences);
    // Edges sit beneath both panels, and the header covers the sidebar's top.
    header_panel.add_to(&layout.stage);
    sidebar_panel.add_to(&layout.stage);
    layout.stage.add_overlay(&sidebar_panel.revealer);
    layout.stage.add_overlay(&header_panel.revealer);
}

fn install_header(
    layout: &BrowserLayout,
    header: &Header,
    browser: &BrowserView,
    preferences: &PreferenceManager,
) -> Rc<FloatingPanel> {
    let editing = browser.clone();
    let panel = FloatingPanel::new(gtk::PositionType::Top, browser, move || {
        editing.location_edit_is_active()
    });
    panel.set_request(|panel, revealed| panel.apply(revealed, false));

    let weak = Rc::downgrade(&panel);
    let refocus = browser.clone();
    browser.connect_location_edit_changed(move |editing| {
        let Some(panel) = weak.upgrade() else {
            return;
        };
        if !panel.enabled() {
            return;
        }
        if editing {
            panel.apply(true, true);
            // The entry may have been unmapped when the edit tried to focus it.
            let refocus = refocus.clone();
            glib::idle_add_local_once(move || {
                if refocus.location_edit_is_active() && !refocus.location_has_focus() {
                    refocus.begin_location_edit();
                }
            });
        } else {
            panel.schedule_hide();
        }
    });

    let root = layout.root.downgrade();
    let widget = header.widget.clone().upcast::<gtk::Widget>();
    let docked = panel.clone();
    preferences.bind_preference(
        &layout.stage,
        PreferenceManager::auto_hide_header,
        move |_, enabled| {
            let Some(root) = root.upgrade() else {
                return;
            };
            dock_header(&docked, &root, &widget, enabled);
        },
    );
    panel
}

fn dock_header(panel: &Rc<FloatingPanel>, root: &gtk::Box, header: &gtk::Widget, floating: bool) {
    if panel.contains_focus() {
        panel.browser.browser().focus_active();
    }
    panel.set_enabled(floating);
    if floating {
        if header.parent().as_ref() == Some(root.upcast_ref::<gtk::Widget>()) {
            root.remove(header);
            panel.frame.append(header);
        }
    } else if header.parent().as_ref() == Some(panel.frame.upcast_ref::<gtk::Widget>()) {
        panel.frame.remove(header);
        root.prepend(header);
    }
}

fn install_sidebar(
    layout: &BrowserLayout,
    header: &Header,
    sidebar: &SidebarView,
    browser: &BrowserView,
    preferences: &PreferenceManager,
) -> Rc<FloatingPanel> {
    let panel = FloatingPanel::new(gtk::PositionType::Left, browser, || false);
    let toggle = header.sidebar_toggle.clone();
    let hovering = Rc::new(Cell::new(false));
    let requested_by_hover = hovering.clone();
    let request_toggle = toggle.downgrade();
    panel.set_request(move |_, revealed| {
        let Some(toggle) = request_toggle.upgrade() else {
            return;
        };
        requested_by_hover.set(true);
        toggle.set_active(revealed);
        requested_by_hover.set(false);
    });

    // Without the docked split's divider, the sidebar would grow to its widest label.
    let clamp = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .vexpand(true)
        .build();
    panel.frame.append(&clamp);

    let floating = layout.sidebar_floating.clone();
    let weak = Rc::downgrade(&panel);
    let sized = clamp.downgrade();
    let state = Rc::downgrade(&sidebar.state);
    toggle.connect_toggled(move |toggle| {
        if !floating.get() {
            return;
        }
        let (Some(panel), Some(state), Some(clamp)) =
            (weak.upgrade(), state.upgrade(), sized.upgrade())
        else {
            return;
        };
        if toggle.is_active() {
            let width = state
                .saved_width
                .get()
                .unwrap_or_else(preferred_sidebar_width)
                .max(MIN_SIDEBAR_WIDTH);
            clamp.set_width_request(width);
        }
        panel.apply(toggle.is_active(), !hovering.get());
    });

    let dock = SidebarDock {
        panel: panel.clone(),
        clamp,
        split: layout.sidebar_split.downgrade(),
        toggle: toggle.downgrade(),
        floating: layout.sidebar_floating.clone(),
        state: Rc::downgrade(&sidebar.state),
        widget: sidebar.widget.clone(),
    };
    preferences.bind_preference(
        &layout.stage,
        PreferenceManager::auto_hide_sidebar,
        move |_, enabled| dock.set_floating(enabled),
    );
    panel
}

struct SidebarDock {
    panel: Rc<FloatingPanel>,
    clamp: gtk::ScrolledWindow,
    split: glib::WeakRef<gtk::Paned>,
    toggle: glib::WeakRef<gtk::ToggleButton>,
    floating: Rc<Cell<bool>>,
    state: Weak<SidebarState>,
    widget: gtk::Widget,
}

impl SidebarDock {
    fn set_floating(&self, floating: bool) {
        let (Some(split), Some(toggle), Some(state)) = (
            self.split.upgrade(),
            self.toggle.upgrade(),
            self.state.upgrade(),
        ) else {
            return;
        };
        if self.floating.get() == floating {
            return;
        }
        if self.panel.contains_focus()
            || split
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focus| focus.is_ancestor(&self.widget))
        {
            self.panel.browser.browser().focus_active();
        }
        if floating {
            self.floating.set(true);
            state.set_rail(false);
            split.set_start_child(None::<&gtk::Widget>);
            split.set_position(0);
            self.widget.set_visible(true);
            self.clamp.set_child(Some(&self.widget));
            self.panel.set_enabled(true);
            toggle.set_active(false);
        } else {
            toggle.set_active(false);
            self.panel.set_enabled(false);
            self.clamp.set_child(None::<&gtk::Widget>);
            self.floating.set(false);
            self.widget.set_visible(false);
            split.set_position(0);
            split.set_start_child(Some(&self.widget));
            toggle.set_active(true);
        }
    }
}
