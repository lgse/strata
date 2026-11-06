// SPDX-License-Identifier: MIT

//! Dragging rows out of a column and dropping files onto folder rows.

use super::{RowContext, RowHandle};
use crate::model::{FileEntry, Location};
use crate::ui::browser::{
    ViewState, arm_spring_load_navigation,
    clipboard::{
        FileDropState, PreparedFileDrop, clear_active_drag, drag_actions_for_modifiers,
        drag_preview_icon, file_drag_content, file_drag_hover_target, file_drop_action,
        prepare_file_drop_target,
    },
    content_drop::{DropHandler, attach_drop_target, commit_dropped_value},
};
use crate::ui::modal::slide_in_down;
use gtk::{glib, prelude::*};
use std::{rc::Rc, time::Duration};

type SpringNavigate = Rc<dyn Fn(Location)>;

/// Highlights the row while the hovered drop targets it and arms spring-loaded navigation.
fn track_drop_hover(
    target: &gtk::DropTarget,
    drop_state: &Rc<FileDropState>,
    row: &glib::WeakRef<gtk::Box>,
    navigate: &SpringNavigate,
) -> gtk::gdk::DragAction {
    let action = file_drop_action(target, drop_state);
    let hovered = file_drag_hover_target(drop_state, target).is_some();
    if let Some(row) = row.upgrade() {
        if hovered {
            row.add_css_class("drop-destination");
        } else {
            row.remove_css_class("drop-destination");
        }
    }
    arm_spring_load_navigation(drop_state, target, navigate);
    action
}

/// Search hits drag the selected hits when the pressed one is among them.
fn dragged_search_entries(context: &RowContext, position: u32) -> Option<Vec<FileEntry>> {
    let results = context.search_results.borrow();
    let entry = crate::ui::browser::search_result_entry(results.get(position as usize)?);
    let selected: Vec<_> =
        crate::ui::browser::collection::bitset_positions(&context.selection.selection())
            .into_iter()
            .filter_map(|position| results.get(position as usize))
            .map(crate::ui::browser::search_result_entry)
            .collect();
    if selected
        .iter()
        .any(|selected| selected.location == entry.location)
    {
        Some(selected)
    } else {
        Some(vec![entry])
    }
}

fn dragged_listing_entries(
    context: &RowContext,
    state: &ViewState,
    position: u32,
) -> Option<Vec<FileEntry>> {
    let depth = context.depth;
    let source_position = context.map.source_position(position)?;
    let entry = state.browser.entry_at(depth, source_position)?;
    if state
        .browser
        .active_selection_contains(depth, source_position)
    {
        Some(state.browser.selected_entries())
    } else {
        Some(vec![entry])
    }
}

impl RowHandle {
    pub(super) fn install_drag_source(&self, row: &gtk::Box) -> gtk::DragSource {
        let drag = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
            .build();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let handle = self.clone();
        drag.connect_prepare(move |source, x, y| handle.prepare_drag(source, x, y));
        let dragged_row = row.downgrade();
        let weak_state_for_begin = self.context.state.clone();
        let pending_activation_for_drag = self.pointer.pending_activation.clone();
        let depth = self.context.depth;
        drag.connect_drag_begin(move |_, _| {
            pending_activation_for_drag.borrow_mut().take();
            if let Some(row) = dragged_row.upgrade() {
                row.add_css_class("dragging");
            }
            if let Some(state) = weak_state_for_begin.upgrade() {
                state.drag_source_depth.set(Some(depth));
                state.cancel_peek();
            }
        });
        let dragged_row = row.downgrade();
        let weak_state_for_end = self.context.state.clone();
        drag.connect_drag_end(move |_, _, _| {
            clear_active_drag();
            if let Some(row) = dragged_row.upgrade() {
                row.remove_css_class("dragging");
            }
            if let Some(state) = weak_state_for_end.upgrade() {
                state.cancel_peek();
                state.stop_drag_autoscroll();
            }
        });
        row.add_controller(drag.clone());
        drag
    }

    /// A press past the drag threshold on the row's content, not an editor.
    fn drag_starts_at(&self, row: &gtk::Box, source: &gtk::DragSource, x: f64, y: f64) -> bool {
        if let Some(current) = source.current_event().and_then(|event| event.position())
            && !crate::ui::pointer::exceeds_drag_threshold(
                self.pointer.press_origin.get(),
                current,
                row.settings().gtk_dnd_drag_threshold(),
            )
        {
            return false;
        }
        let blocked = row
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|target| crate::ui::focus_navigation::editable(&target))
            || (!self.pointer.was_selected.get()
                && !crate::ui::pointer::hits_item_content(row.upcast_ref(), x, y));
        !blocked
    }

    fn prepare_drag(
        &self,
        source: &gtk::DragSource,
        x: f64,
        y: f64,
    ) -> Option<gtk::gdk::ContentProvider> {
        let prepare_row = self.row.upgrade()?;
        if !self.drag_starts_at(&prepare_row, source, x, y) {
            return None;
        }
        source.set_actions(drag_actions_for_modifiers(source.current_event_state()));
        let state = self.context.state.upgrade()?;
        let dragged_item = self.item.upgrade()?;
        let position = dragged_item.position();
        let entries = if self.context.search_active.get() {
            dragged_search_entries(&self.context, position)?
        } else {
            dragged_listing_entries(&self.context, &state, position)?
        };
        if let Some((texture, hot_x, hot_y)) = drag_preview_icon(&prepare_row, &entries) {
            source.set_icon(Some(&texture), hot_x, hot_y);
        }
        file_drag_content(source, &entries)
    }

    fn row_drop_destination(&self) -> impl Fn() -> Option<Location> + 'static {
        let weak_state = self.context.state.clone();
        let item = self.item.clone();
        let map = self.context.map.clone();
        let depth = self.context.depth;
        move || {
            let state = weak_state.upgrade()?;
            let item = item.upgrade()?;
            map.source_position(item.position())
                .and_then(|position| state.browser.entry_at(depth, position))
                .filter(FileEntry::is_directory)
                .map(|entry| entry.location)
        }
    }

    fn spring_navigate(&self) -> SpringNavigate {
        let weak_state = self.context.state.clone();
        let depth = self.context.depth;
        Rc::new(move |location| {
            if let Some(state) = weak_state.upgrade() {
                state.browser.descend(depth, location);
            }
        })
    }

    pub(super) fn install_drop_target(&self, row: &gtk::Box) {
        let PreparedFileDrop {
            target: drop,
            state: drop_state,
        } = prepare_file_drop_target(self.row_drop_destination());
        let spring_navigate = self.spring_navigate();
        let highlighted_row = row.downgrade();
        let state_for_enter = drop_state.clone();
        let navigate_for_enter = spring_navigate.clone();
        drop.connect_enter(move |target, _, _| {
            track_drop_hover(
                target,
                &state_for_enter,
                &highlighted_row,
                &navigate_for_enter,
            )
        });
        let highlighted_row = row.downgrade();
        let state_for_motion = drop_state.clone();
        let navigate_for_motion = spring_navigate.clone();
        drop.connect_motion(move |target, _, _| {
            track_drop_hover(
                target,
                &state_for_motion,
                &highlighted_row,
                &navigate_for_motion,
            )
        });
        let highlighted_row = row.downgrade();
        let state_for_value = drop_state.clone();
        let navigate_for_value = spring_navigate;
        drop.connect_value_notify(move |target| {
            if target.current_drop().is_none() {
                return;
            }
            track_drop_hover(
                target,
                &state_for_value,
                &highlighted_row,
                &navigate_for_value,
            );
        });
        let highlighted_row = row.downgrade();
        let state_for_leave = drop_state.clone();
        drop.connect_leave(move |_| {
            state_for_leave.cancel_spring_load_navigation();
            if let Some(row) = highlighted_row.upgrade() {
                row.remove_css_class("drop-destination");
            }
        });
        let handler: DropHandler = {
            let weak_state = self.context.state.clone();
            Rc::new(move |destination, request| {
                if let Some(state) = weak_state.upgrade() {
                    state.commit_drop(destination, request);
                }
            })
        };
        let weak_state_for_drop = self.context.state.clone();
        let dropped_row = row.downgrade();
        let state_for_drop = drop_state.clone();
        drop.connect_drop(move |target, value, _, _| {
            let Some(dropped_row) = dropped_row.upgrade() else {
                return false;
            };
            drop_on_row(
                target,
                value,
                &dropped_row,
                &state_for_drop,
                &weak_state_for_drop,
            )
        });
        attach_drop_target(row, &drop, &drop_state, handler);
    }
}

/// Slides the row in before committing the drop onto its folder.
fn drop_on_row(
    target: &gtk::DropTarget,
    value: &glib::Value,
    dropped_row: &gtk::Box,
    drop_state: &Rc<FileDropState>,
    weak_state: &std::rc::Weak<ViewState>,
) -> bool {
    drop_state.cancel_spring_load_navigation();
    dropped_row.remove_css_class("drop-destination");
    let Some(state) = weak_state.upgrade() else {
        return false;
    };
    state.stop_drag_autoscroll();
    let Some(destination) = drop_state.destination() else {
        return false;
    };
    let slid_row = dropped_row.downgrade();
    commit_dropped_value(
        target,
        value,
        destination,
        drop_state,
        move |destination, request| {
            if let Some(row) = slid_row.upgrade() {
                slide_in_down(&row);
            }
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                state.commit_drop(destination, request);
            });
        },
    )
}
