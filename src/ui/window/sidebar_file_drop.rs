// SPDX-License-Identifier: MIT

//! Dropping files onto sidebar places.

use std::rc::Rc;

use gtk::{glib, prelude::*};

use crate::{
    model::Location,
    ui::browser::{
        DropHandler, PaneTarget, PreparedFileDrop, WeakBrowserView, arm_spring_load_navigation,
        attach_drop_target, commit_dropped_value, file_drag_hover_target, file_drop_action,
        prepare_file_drop_target,
    },
};

use super::{
    SidebarState, sidebar_drop_feedback::SidebarDropFeedback,
    sidebar_trash::install_sidebar_trash_drop,
};

pub(super) fn sidebar_accepts_file_drop(location: &Location) -> bool {
    location.native_path().is_some()
}

type DropHover = Rc<dyn Fn(&gtk::DropTarget, &gtk::Widget)>;

/// What the hover handlers on a place's drop target share.
#[derive(Clone)]
struct PlaceDrop {
    feedback: Rc<SidebarDropFeedback>,
    action: Rc<dyn Fn(&gtk::DropTarget) -> gtk::gdk::DragAction>,
    /// Highlights the row while files hover it and arms spring-loaded opening.
    hover: DropHover,
    cancel_spring_load: Rc<dyn Fn()>,
}

pub(super) fn install_sidebar_file_drop(
    state: &SidebarState,
    row: &impl IsA<gtk::Widget>,
    destination: Location,
) {
    if destination == Location::uri("trash:///") {
        install_sidebar_trash_drop(state, row);
        return;
    }
    if !sidebar_accepts_file_drop(&destination) {
        return;
    }
    row.add_css_class("file-drop-zone");
    let PreparedFileDrop {
        target: drop,
        state: drop_state,
    } = prepare_file_drop_target({
        let destination = destination.clone();
        move || Some(destination.clone())
    });
    drop.set_propagation_phase(gtk::PropagationPhase::Capture);
    let navigate = spring_navigation(&state.target, row.upcast_ref());
    let feedback = state.drop_feedback.clone();
    let place = PlaceDrop {
        feedback: feedback.clone(),
        action: {
            let drop_state = drop_state.clone();
            Rc::new(move |target| file_drop_action(target, &drop_state))
        },
        hover: {
            let drop_state = drop_state.clone();
            Rc::new(move |target, row| {
                let hovered = file_drag_hover_target(&drop_state, target).is_some();
                feedback.folder(row, hovered);
                arm_spring_load_navigation(&drop_state, target, &navigate);
            })
        },
        cancel_spring_load: {
            let drop_state = drop_state.clone();
            Rc::new(move || drop_state.cancel_spring_load_navigation())
        },
    };
    connect_enter(&drop, place.clone());
    connect_motion(&drop, place.clone());
    connect_value(&drop, place);
    let leave_feedback = state.drop_feedback.clone();
    drop.connect_leave(move |target| {
        if let Some(row) = target.widget() {
            leave_feedback.leave(&row);
        }
    });
    let handler = commit_handler(state.view().downgrade());
    let commit = {
        let drop_state = drop_state.clone();
        move |target: &gtk::DropTarget, value: &glib::Value, handler: DropHandler| {
            commit_dropped_value(
                target,
                value,
                destination.clone(),
                &drop_state,
                move |destination, request| handler(destination, request),
            )
        }
    };
    connect_drop(&drop, state.drop_feedback.clone(), handler.clone(), commit);
    attach_drop_target(row, &drop, &drop_state, handler);
}

/// Spring-loaded opening navigates the active pane and keeps focus on the row.
fn spring_navigation(target: &PaneTarget, row: &gtk::Widget) -> Rc<dyn Fn(Location)> {
    let target = target.clone();
    let row = row.downgrade();
    Rc::new(move |location| {
        target.browser().navigate_location(location, false);
        if let Some(row) = row.upgrade() {
            row.grab_focus();
        }
    })
}

fn connect_enter(drop: &gtk::DropTarget, place: PlaceDrop) {
    drop.connect_enter(move |target, _, _| {
        let Some(row) = target.widget() else {
            return gtk::gdk::DragAction::empty();
        };
        let cancel = place.cancel_spring_load.clone();
        place.feedback.enter_folder(&row, move || cancel());
        let action = (place.action)(target);
        (place.hover)(target, &row);
        action
    });
}

fn connect_motion(drop: &gtk::DropTarget, place: PlaceDrop) {
    drop.connect_motion(move |target, _, _| {
        let Some(row) = target.widget().filter(|row| place.feedback.is_current(row)) else {
            return gtk::gdk::DragAction::empty();
        };
        let action = (place.action)(target);
        (place.hover)(target, &row);
        action
    });
}

fn connect_value(drop: &gtk::DropTarget, place: PlaceDrop) {
    drop.connect_value_notify(move |target| {
        if target.current_drop().is_none() {
            return;
        }
        let Some(row) = target.widget().filter(|row| place.feedback.is_current(row)) else {
            return;
        };
        (place.hover)(target, &row);
    });
}

fn commit_handler(view: WeakBrowserView) -> DropHandler {
    Rc::new(move |destination, request| {
        if let Some(view) = view.upgrade() {
            view.commit_drop(destination, request);
        }
    })
}

fn connect_drop(
    drop: &gtk::DropTarget,
    feedback: Rc<SidebarDropFeedback>,
    handler: DropHandler,
    commit: impl Fn(&gtk::DropTarget, &glib::Value, DropHandler) -> bool + 'static,
) {
    drop.connect_drop(move |target, value, _, _| {
        if let Some(row) = target.widget() {
            feedback.leave(&row);
        }
        commit(target, value, handler.clone())
    });
}
