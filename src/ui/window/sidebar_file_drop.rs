// SPDX-License-Identifier: MIT

//! Dropping files onto sidebar places.

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    model::Location,
    ui::browser::{
        DropHandler, PreparedFileDrop, arm_spring_load_navigation, attach_drop_target,
        commit_dropped_value, file_drag_hover_target, file_drop_action, prepare_file_drop_target,
    },
};

use super::{SidebarState, sidebar_trash::install_sidebar_trash_drop};

pub(super) fn sidebar_accepts_file_drop(location: &Location) -> bool {
    location.native_path().is_some()
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
    let spring_navigate: Rc<dyn Fn(Location)> = {
        let view = state.view.downgrade();
        let row = row.downgrade();
        Rc::new(move |location| {
            let Some(view) = view.upgrade() else {
                return;
            };
            view.browser().navigate_location(location, false);
            if let Some(row) = row.upgrade() {
                row.grab_focus();
            }
        })
    };
    let state_for_enter = drop_state.clone();
    let navigate_for_enter = spring_navigate.clone();
    let enter_feedback = state.drop_feedback.clone();
    drop.connect_enter(move |target, _, _| {
        let Some(row) = target.widget() else {
            return gtk::gdk::DragAction::empty();
        };
        let cancel_state = state_for_enter.clone();
        enter_feedback.enter_folder(&row, move || cancel_state.cancel_spring_load_navigation());
        let action = file_drop_action(target, &state_for_enter);
        let hovered = file_drag_hover_target(&state_for_enter, target).is_some();
        enter_feedback.folder(&row, hovered);
        arm_spring_load_navigation(&state_for_enter, target, &navigate_for_enter);
        action
    });
    let state_for_motion = drop_state.clone();
    let navigate_for_motion = spring_navigate.clone();
    let motion_feedback = state.drop_feedback.clone();
    drop.connect_motion(move |target, _, _| {
        let Some(row) = target
            .widget()
            .filter(|row| motion_feedback.is_current(row))
        else {
            return gtk::gdk::DragAction::empty();
        };
        let action = file_drop_action(target, &state_for_motion);
        let hovered = file_drag_hover_target(&state_for_motion, target).is_some();
        motion_feedback.folder(&row, hovered);
        arm_spring_load_navigation(&state_for_motion, target, &navigate_for_motion);
        action
    });
    let state_for_value = drop_state.clone();
    let navigate_for_value = spring_navigate.clone();
    let value_feedback = state.drop_feedback.clone();
    drop.connect_value_notify(move |target| {
        if target.current_drop().is_none() {
            return;
        }
        let Some(row) = target.widget().filter(|row| value_feedback.is_current(row)) else {
            return;
        };
        let hovered = file_drag_hover_target(&state_for_value, target).is_some();
        value_feedback.folder(&row, hovered);
        arm_spring_load_navigation(&state_for_value, target, &navigate_for_value);
    });
    let leave_feedback = state.drop_feedback.clone();
    drop.connect_leave(move |target| {
        if let Some(row) = target.widget() {
            leave_feedback.leave(&row);
        }
    });
    let handler: DropHandler = {
        let view = state.view.downgrade();
        Rc::new(move |destination, request| {
            if let Some(view) = view.upgrade() {
                view.commit_drop(destination, request);
            }
        })
    };
    let handler_for_drop = handler.clone();
    let state_for_drop = drop_state.clone();
    let drop_feedback = state.drop_feedback.clone();
    drop.connect_drop(move |target, value, _, _| {
        if let Some(row) = target.widget() {
            drop_feedback.leave(&row);
        }
        let handler = handler_for_drop.clone();
        commit_dropped_value(
            target,
            value,
            destination.clone(),
            &state_for_drop,
            move |destination, request| handler(destination, request),
        )
    });
    attach_drop_target(row, &drop, &drop_state, handler);
}
