// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc};

use gtk::{gdk, glib, prelude::*};

use super::super::TabWindow;
use crate::{
    model::Location,
    ui::{browser::active_drag_snapshot, window::pinned_drop},
};

enum Intent {
    Reorder(u64),
    Open(Rc<[Location]>),
}

pub(super) fn folder_locations(drop: &gdk::Drop) -> Option<Rc<[Location]>> {
    let snapshot = active_drag_snapshot(drop)?;
    snapshot.folders.filter(|folders| !folders.is_empty())?;
    Some(snapshot.locations)
}

pub(super) fn track_folder_drag(state: &Rc<TabWindow>) {
    let Some(window) = state.window.upgrade() else {
        return;
    };
    let motion = gtk::DropControllerMotion::new();
    let active = Rc::new(glib::WeakRef::<gdk::Drag>::new());
    let weak = Rc::downgrade(state);
    motion.connect_enter(move |motion, _, _| {
        let Some(state) = weak.upgrade().filter(|state| !state.blocked()) else {
            return;
        };
        let Some(drop) = motion
            .drop()
            .filter(|drop| folder_locations(drop).is_some())
        else {
            return;
        };
        let Some(drag) = drop.drag() else {
            return;
        };
        if active.upgrade().as_ref() == Some(&drag) {
            return;
        }
        active.set(Some(&drag));
        state.strip.show_open_drop(true);
        let weak = Rc::downgrade(&state);
        let active = active.clone();
        let finish = Rc::new(move |drag: &gdk::Drag| {
            if active.upgrade().as_ref() != Some(drag) {
                return;
            }
            active.set(None::<&gdk::Drag>);
            if let Some(state) = weak.upgrade() {
                state.strip.show_open_drop(false);
            }
        });
        let cancelled = finish.clone();
        drag.connect_cancel(move |drag, _| cancelled(drag));
        drag.connect_dnd_finished(move |drag| finish(drag));
    });
    window.add_controller(motion);
}

fn intent(
    state: &TabWindow,
    target: &gtk::DropTarget,
    value: Option<glib::Value>,
) -> Option<Intent> {
    if state.blocked() {
        return None;
    }
    if let Some(location) = target
        .current_drop()
        .as_ref()
        .and_then(pinned_drop::dragged_location)
    {
        return Some(Intent::Open(vec![location].into()));
    }
    if let Some(locations) = target.current_drop().as_ref().and_then(folder_locations) {
        return Some(Intent::Open(locations));
    }
    let value = value?.get::<String>().ok()?;
    let source = value
        .strip_prefix(&format!("strata-tab-{}:", state.drag_token))?
        .parse()
        .ok()?;
    state
        .tabs
        .borrow()
        .iter()
        .any(|tab| tab.id == source)
        .then_some(Intent::Reorder(source))
}

fn position(state: &TabWindow, x: f64, append: bool) -> usize {
    let headers = state.strip.headers.borrow();
    if !append {
        for (index, header) in headers.iter().enumerate() {
            if let Some(bounds) = header.widget.compute_bounds(&state.strip.overlay)
                && x < f64::from(bounds.x() + bounds.width() / 2.0)
            {
                return index;
            }
        }
    }
    headers.len()
}

fn clear(state: &TabWindow, target: &gtk::DropTarget) {
    state.strip.mark_insertion(None);
    if let Some(widget) = target.widget() {
        widget.remove_css_class("tab-drop-hover");
    }
}

pub(super) fn install(state: &Rc<TabWindow>, widget: &gtk::Widget, append: bool) {
    let target = gtk::DropTarget::new(
        String::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    target.set_types(&[String::static_type(), gdk::FileList::static_type()]);
    target.set_propagation_phase(gtk::PropagationPhase::Capture);
    target.set_preload(true);
    target.connect_accept(|_, drop| {
        if folder_locations(drop).is_some() {
            return true;
        }
        let formats = drop.formats();
        let decoded = formats.clone().union_deserialize_types();
        decoded.contains_type(String::static_type())
            && !decoded.contains_type(gdk::FileList::static_type())
            && !formats.contain_mime_type("text/uri-list")
    });
    let hover_x = Rc::new(Cell::new(0.0));
    let valid_source = Rc::new(Cell::new(false));
    let validity = valid_source.clone();
    let weak = Rc::downgrade(state);
    // Hover caches source validation; committing the drop always revalidates it.
    let update = Rc::new(move |target: &gtk::DropTarget, x: f64, reload: bool| {
        let Some(state) = weak.upgrade() else {
            return gdk::DragAction::empty();
        };
        if reload {
            validity.set(intent(&state, target, target.value()).is_some());
        }
        if !validity.get() || state.blocked() {
            clear(&state, target);
            return gdk::DragAction::empty();
        }
        state
            .strip
            .mark_insertion(Some(position(&state, x, append)));
        if append && let Some(widget) = target.widget() {
            widget.add_css_class("tab-drop-hover");
        }
        if target
            .current_drop()
            .is_some_and(|drop| drop.actions().contains(gdk::DragAction::COPY))
        {
            gdk::DragAction::COPY
        } else {
            gdk::DragAction::MOVE
        }
    });
    let enter_update = update.clone();
    let enter_x = hover_x.clone();
    target.connect_enter(move |target, x, _| {
        enter_x.set(x);
        enter_update(target, x, true)
    });
    let motion_update = update.clone();
    let motion_x = hover_x.clone();
    target.connect_motion(move |target, x, _| {
        motion_x.set(x);
        motion_update(target, x, false)
    });
    target.connect_value_notify(move |target| {
        if let Some(drop) = target.current_drop() {
            let action = update(target, hover_x.get(), true);
            drop.status(action, action);
        }
    });
    let weak = Rc::downgrade(state);
    target.connect_leave(move |target| {
        valid_source.set(false);
        if let Some(state) = weak.upgrade() {
            clear(&state, target);
        }
    });
    let weak = Rc::downgrade(state);
    target.connect_drop(move |target, value, x, _| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        let intent = intent(&state, target, Some(value.clone()));
        let insertion = position(&state, x, append);
        clear(&state, target);
        match intent {
            Some(Intent::Reorder(source)) => state.reorder_at(source, insertion),
            Some(Intent::Open(locations)) => {
                for (offset, location) in locations.iter().enumerate() {
                    state.open_in_new_tab(location.clone());
                    state.reorder_at(state.active.get(), insertion + offset);
                }
            }
            None => return false,
        }
        state.strip.show_open_drop(false);
        true
    });
    widget.add_controller(target);
}

pub(in super::super) fn install_new_tab_drop(state: &Rc<TabWindow>, button: &gtk::Button) {
    install(state, button.upcast_ref(), true);
}
