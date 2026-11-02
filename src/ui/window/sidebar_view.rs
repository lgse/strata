// SPDX-License-Identifier: MIT

//! The sidebar handle a window holds, rebuilding and scroll preservation.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::prelude::*;

use super::{SidebarState, device_release};

pub(in crate::ui) struct SidebarView {
    pub(in crate::ui) widget: gtk::Widget,
    pub(in crate::ui) state: Rc<SidebarState>,
    pub(super) update_notice: gtk::Button,
    pub(super) update_area: gtk::Box,
    pub(super) update_label: gtk::Label,
    pub(super) update_version: gtk::Label,
    pub(super) handlers: RefCell<Vec<glib::SignalHandlerId>>,
    pub(super) mount_handler: RefCell<Option<glib::SignalHandlerId>>,
    pub(super) recent_setting_handler: RefCell<Option<(gtk::Settings, glib::SignalHandlerId)>>,
    #[expect(
        dead_code,
        reason = "held so Drop keeps this window registered for pending-release rebuilds"
    )]
    pub(super) release_watch: device_release::SidebarWatch,
    pub(super) bookmark_watch: RefCell<Option<crate::adapters::bookmarks::BookmarkWatch>>,
}

impl SidebarView {
    // Device discovery can block on D-Bus; let the chooser paint before starting it.
    pub(in crate::ui) fn schedule_after_first_paint(&self, window: &impl IsA<gtk::Widget>) {
        let weak_state = Rc::downgrade(&self.state);
        let armed = Rc::new(Cell::new(false));
        let arm = {
            let weak_state = weak_state.clone();
            let armed = armed.clone();
            move |widget: &gtk::Widget| {
                if armed.get() {
                    return;
                }
                armed.set(true);
                let Some(clock) = widget.frame_clock() else {
                    let weak = weak_state.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(state) = weak.upgrade() {
                            state.rebuild();
                        }
                    });
                    return;
                };
                let handler = Rc::new(RefCell::new(None));
                let handler_for_paint = handler.clone();
                let weak = weak_state.clone();
                let id = clock.connect_after_paint(move |clock| {
                    if let Some(id) = handler_for_paint.borrow_mut().take() {
                        clock.disconnect(id);
                    }
                    let weak = weak.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(state) = weak.upgrade() {
                            state.rebuild();
                        }
                    });
                });
                handler.replace(Some(id));
            }
        };
        if window.is_mapped() {
            arm(window.upcast_ref());
        } else {
            let arm_on_map = arm.clone();
            window.connect_map(move |widget| {
                arm_on_map(widget.upcast_ref());
            });
        }
    }

    pub(in crate::ui) fn disconnect(&self) {
        unparent_sidebar_popovers(self.state.widget.upcast_ref());
        self.state.trash_monitor.take();
        self.bookmark_watch.take();
        for handler in self.handlers.take() {
            self.state.volume_monitor.disconnect(handler);
        }
        if let Some(handler) = self.mount_handler.take() {
            self.state.mount_monitor.disconnect(handler);
        }
        if let Some((settings, handler)) = self.recent_setting_handler.take() {
            settings.disconnect(handler);
        }
    }
}

pub(super) fn unparent_sidebar_popovers(widget: &gtk::Widget) {
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if widget.is::<gtk::Button>() && current.is::<gtk::Popover>() {
            current.unparent();
        } else {
            unparent_sidebar_popovers(&current);
        }
    }
}

impl SidebarState {
    pub(super) fn queue_rebuild(self: &Rc<Self>) {
        self.capture_scroll();
        if self.rebuild_queued.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.rebuild_queued.set(false);
                state.rebuild();
            }
        });
    }

    pub(super) fn rebuild(self: &Rc<Self>) {
        self.capture_scroll();
        while let Some(child) = self.widget.first_child() {
            unparent_sidebar_popovers(&child);
            self.widget.remove(&child);
        }
        self.drop_feedback.reset();
        self.place_rows.borrow_mut().clear();
        self.keycaps.borrow_mut().clear();
        self.visible_pins.borrow_mut().clear();
        self.pinned_section.replace(None);
        self.devices_section.replace(None);

        self.append_static_places();
        self.append_devices();
        self.sync_active_place();
        self.schedule_scroll_restore();
        self.sync_rail_rows();
    }

    pub(super) fn sidebar_scroller(&self) -> Option<gtk::ScrolledWindow> {
        self.widget
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast()
    }

    pub(super) fn capture_scroll(&self) {
        // Removing all rows temporarily clamps the adjustment to zero.
        if self.pending_scroll.get().is_some() {
            return;
        }
        let value = self
            .sidebar_scroller()
            .map(|scroller| scroller.vadjustment().value())
            .unwrap_or(0.0);
        if value > 0.0 {
            self.pending_scroll.set(Some(value));
        }
    }

    pub(super) fn schedule_scroll_restore(self: &Rc<Self>) {
        if self.pending_scroll.get().is_none() || self.scroll_restore_queued.replace(true) {
            return;
        }
        let Some(scroller) = self.sidebar_scroller() else {
            self.scroll_restore_queued.set(false);
            return;
        };
        let weak = Rc::downgrade(self);
        scroller.add_tick_callback(move |_, _| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    if let Some(position) = state.pending_scroll.take()
                        && let Some(scroller) = state.sidebar_scroller()
                    {
                        scroller.vadjustment().set_value(position);
                    }
                    state.scroll_restore_queued.set(false);
                }
            });
            glib::ControlFlow::Break
        });
    }
}
