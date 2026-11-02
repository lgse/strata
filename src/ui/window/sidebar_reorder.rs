// SPDX-License-Identifier: MIT

//! Dragging sidebar places and pins to reorder them.

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;

use crate::model::Location;

use super::{
    SidebarState,
    places::STANDARD_PLACE_IDS,
    sidebar,
    sidebar::PlaceNavigation,
    sidebar_drop_feedback::{self, SidebarDropFeedback},
    sidebar_rows::{customize_sidebar_folder_icon, sidebar_button},
};

pub(super) const PINNED_DRAG_PREFIX: &str = "pinned:";

pub(super) type SidebarReorder = Rc<dyn Fn(&Rc<SidebarState>, &str, bool) -> bool>;

/// A drop target for reordering: a row, which places a drop by its half, or
/// the gap fixed above or below it.
pub(super) struct ReorderSlot {
    payload: String,
    fixed_after: Option<bool>,
}

impl ReorderSlot {
    fn row(payload: &str) -> Self {
        Self {
            payload: payload.to_owned(),
            fixed_after: None,
        }
    }

    fn gap(payload: &str, after: bool) -> Self {
        Self {
            payload: payload.to_owned(),
            fixed_after: Some(after),
        }
    }

    /// A row dropped on its own gap stays put; a payload of another kind is refused.
    fn settled_without_move(&self, source: &str) -> Option<bool> {
        if source == self.payload && self.fixed_after.is_some() {
            return Some(true);
        }
        (!sidebar_reorder_matches(source, &self.payload)).then_some(false)
    }
}

/// Tracks the pointer over a reorder target and shows where a drop would land.
struct ReorderHover {
    payload: String,
    y: Rc<Cell<f64>>,
    feedback: Rc<SidebarDropFeedback>,
}

impl ReorderHover {
    fn connect(&self, drop: &gtk::DropTarget) {
        let enter_y = self.y.clone();
        let enter_payload = self.payload.clone();
        let enter_feedback = self.feedback.clone();
        drop.connect_enter(move |target, _, y| {
            enter_y.set(y);
            if let Some(row) = target.widget() {
                enter_feedback.enter(&row);
            }
            sidebar_reorder_feedback(target, &enter_payload, y, &enter_feedback)
        });
        let motion_y = self.y.clone();
        let motion_payload = self.payload.clone();
        let motion_feedback = self.feedback.clone();
        drop.connect_motion(move |target, _, y| {
            motion_y.set(y);
            sidebar_reorder_feedback(target, &motion_payload, y, &motion_feedback)
        });
        let hover_y = self.y.clone();
        let value_payload = self.payload.clone();
        let value_feedback = self.feedback.clone();
        drop.connect_value_notify(move |target| {
            let Some(offered) = target.current_drop() else {
                return;
            };
            if !target
                .widget()
                .is_some_and(|row| value_feedback.is_current(&row))
            {
                return;
            }
            let action =
                sidebar_reorder_feedback(target, &value_payload, hover_y.get(), &value_feedback);
            offered.status(action, action);
        });
    }
}

impl SidebarState {
    pub(super) fn make_reorderable(
        self: &Rc<Self>,
        row: &gtk::Button,
        payload: String,
        on_drop: impl Fn(&Rc<Self>, &str, bool) -> bool + 'static,
    ) {
        row.add_css_class("reorderable");
        row.set_cursor_from_name(Some("pointer"));

        let pinned = parse_pinned_drag_source(&payload);
        let folder = pinned
            .and_then(|index| self.pinned_places.borrow().get(index).cloned())
            .or_else(|| {
                self.place_rows
                    .borrow()
                    .iter()
                    .find(|(_, candidate)| candidate == row)
                    .map(|(location, _)| (location.clone(), location.display_path()))
            });
        if let Some(folder) = folder {
            sidebar::install_drag(
                row,
                folder,
                payload.clone(),
                pinned.is_some(),
                self.drop_feedback.clone(),
            );
        }

        let on_drop: SidebarReorder = Rc::new(on_drop);
        self.install_reorder_target(row, ReorderSlot::row(&payload), on_drop.clone());
        if parse_pinned_drag_source(&payload).is_none() {
            self.install_reorder_gaps(row, &payload, &on_drop);
        }
    }

    /// Standard places also take drops in the gaps above and below them.
    fn install_reorder_gaps(
        self: &Rc<Self>,
        row: &gtk::Button,
        payload: &str,
        on_drop: &SidebarReorder,
    ) {
        for after in [false, true] {
            if let Some(gap) = self.insert_reorder_gap(row.upcast_ref(), after) {
                self.install_reorder_target(
                    &gap,
                    ReorderSlot::gap(payload, after),
                    on_drop.clone(),
                );
            }
        }
    }

    pub(super) fn insert_reorder_gap(&self, row: &gtk::Widget, after: bool) -> Option<gtk::Box> {
        let neighbour = if after {
            row.next_sibling()
        } else {
            row.prev_sibling()
        };
        if neighbour
            .as_ref()
            .is_some_and(sidebar_drop_feedback::is_gap)
        {
            return None;
        }
        let gap = sidebar_drop_feedback::row_gap(None);
        let previous = if after { Some(row.clone()) } else { neighbour };
        self.widget.insert_child_after(&gap, previous.as_ref());
        Some(gap)
    }

    pub(super) fn install_reorder_target(
        self: &Rc<Self>,
        widget: &impl IsA<gtk::Widget>,
        slot: ReorderSlot,
        on_drop: SidebarReorder,
    ) {
        let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        drop.set_preload(true);
        drop.connect_accept(|_, offered| {
            accepts_sidebar_reorder_payload(
                offered.formats().contains_type(String::static_type()),
                offered
                    .formats()
                    .contains_type(gtk::gdk::FileList::static_type()),
            )
        });
        let hover = ReorderHover {
            payload: slot.payload.clone(),
            y: Rc::new(Cell::new(0.0)),
            feedback: self.drop_feedback.clone(),
        };
        hover.connect(&drop);
        let leave_feedback = self.drop_feedback.clone();
        drop.connect_leave(move |target| {
            if let Some(row) = target.widget() {
                leave_feedback.leave(&row);
            }
        });
        self.connect_reorder_drop(&drop, slot, on_drop);
        widget.add_controller(drop);
    }

    fn connect_reorder_drop(
        self: &Rc<Self>,
        drop: &gtk::DropTarget,
        slot: ReorderSlot,
        on_drop: SidebarReorder,
    ) {
        let weak_state = Rc::downgrade(self);
        let drop_feedback = self.drop_feedback.clone();
        drop.connect_drop(move |target, value, _, y| {
            let Some(row) = target.widget() else {
                return false;
            };
            drop_feedback.leave(&row);
            let Ok(source) = value.get::<String>() else {
                return false;
            };
            if let Some(settled) = slot.settled_without_move(&source) {
                return settled;
            }
            let after = slot
                .fixed_after
                .unwrap_or_else(|| y >= f64::from(row.height()) / 2.0);
            weak_state
                .upgrade()
                .is_some_and(|state| on_drop(&state, &source, after))
        });
    }

    pub(super) fn append_reorderable_place(
        self: &Rc<Self>,
        id: &'static str,
        icon: &str,
        name: &str,
        location: Location,
    ) -> gtk::Button {
        let row = sidebar_button(icon, name);
        customize_sidebar_folder_icon(&row, &location, icon);
        crate::ui::accessibility::set_description(&row, Some(&location.display_path()));
        self.bind_place_row(&row, location.clone(), PlaceNavigation::Direct);
        self.attach_place_context_menu(&row, location, move |state| {
            let manager = &state.preference_manager;
            match id {
                "desktop" => manager.set_sidebar_show_desktop(false),
                "documents" => manager.set_sidebar_show_documents(false),
                "downloads" => manager.set_sidebar_show_downloads(false),
                "music" => manager.set_sidebar_show_music(false),
                "pictures" => manager.set_sidebar_show_pictures(false),
                "videos" => manager.set_sidebar_show_videos(false),
                _ => {}
            }
        });

        self.append_sidebar_row(&row);
        self.make_place_reorderable(&row, id);
        row
    }

    pub(super) fn make_place_reorderable(self: &Rc<Self>, row: &gtk::Button, id: &'static str) {
        self.make_reorderable(row, id.to_owned(), move |state, source, after| {
            if source.starts_with(PINNED_DRAG_PREFIX) {
                return false;
            }
            state.reorder_place(source, id, after);
            true
        });
    }

    pub(super) fn reorder_place(self: &Rc<Self>, source: &str, target: &str, after: bool) {
        let changed = reorder_places(&mut self.place_order.borrow_mut(), source, target, after);
        if changed {
            let order = self
                .place_order
                .borrow()
                .iter()
                .map(|place| (*place).to_owned())
                .collect();
            self.preference_manager.set_sidebar_order(order);
            self.rebuild();
        }
    }

    pub(super) fn reorder_pinned_place(self: &Rc<Self>, source: usize, target: usize, after: bool) {
        let (source, target) = {
            let places = self.pinned_places.borrow();
            match (places.get(source), places.get(target)) {
                (Some(source), Some(target)) => (source.0.clone(), target.0.clone()),
                _ => return,
            }
        };
        self.update_pinned_places(
            move |places| {
                let position =
                    |location: &Location| places.iter().position(|(pinned, _)| pinned == location);
                match (position(&source), position(&target)) {
                    (Some(source), Some(target)) => {
                        reorder_pinned_places(places, source, target, after)
                    }
                    _ => false,
                }
            },
            |_| {},
        );
    }

    pub(super) fn make_pinned_row_reorderable(self: &Rc<Self>, row: &gtk::Button, index: usize) {
        self.make_reorderable(
            row,
            format!("{PINNED_DRAG_PREFIX}{index}"),
            move |state, source, after| {
                let Some(source) = parse_pinned_drag_source(source) else {
                    return false;
                };
                state.reorder_pinned_place(source, index, after);
                true
            },
        );
    }
}

pub(super) fn accepts_sidebar_reorder_payload(has_string: bool, has_file_list: bool) -> bool {
    has_string && !has_file_list
}

pub(super) fn sidebar_reorder_matches(source: &str, target: &str) -> bool {
    source != target
        && if parse_pinned_drag_source(target).is_some() {
            parse_pinned_drag_source(source).is_some()
        } else {
            STANDARD_PLACE_IDS.contains(&source) && STANDARD_PLACE_IDS.contains(&target)
        }
}

pub(super) fn sidebar_reorder_feedback(
    target: &gtk::DropTarget,
    payload: &str,
    y: f64,
    feedback: &sidebar_drop_feedback::SidebarDropFeedback,
) -> gtk::gdk::DragAction {
    let Some(row) = target.widget() else {
        return gtk::gdk::DragAction::empty();
    };
    if !feedback.is_current(&row) {
        return gtk::gdk::DragAction::empty();
    }
    let Some(value) = target.value() else {
        return gtk::gdk::DragAction::MOVE;
    };
    if !value.get::<String>().is_ok_and(|source| {
        sidebar_reorder_matches(&source, payload)
            || (source == payload && sidebar_drop_feedback::is_gap(&row))
    }) {
        return gtk::gdk::DragAction::empty();
    }
    feedback.reorder(&row, y >= f64::from(row.height()) / 2.0);
    gtk::gdk::DragAction::MOVE
}

pub(super) fn reorder_places(
    order: &mut Vec<&'static str>,
    source: &str,
    target: &str,
    after: bool,
) -> bool {
    if source == target {
        return false;
    }
    let Some(source_index) = order.iter().position(|place| *place == source) else {
        return false;
    };
    let source = order.remove(source_index);
    let Some(target_index) = order.iter().position(|place| *place == target) else {
        order.insert(source_index, source);
        return false;
    };
    order.insert(target_index + usize::from(after), source);
    true
}

pub(super) fn reorder_pinned_places(
    places: &mut Vec<(Location, String)>,
    source: usize,
    target: usize,
    after: bool,
) -> bool {
    if source == target || source >= places.len() || target >= places.len() {
        return false;
    }
    let place = places.remove(source);
    // `source` is a pre-removal index, but `target` must remap onto the shrunken
    // vector: any target that sat after `source` slides left by one. Since
    // `destination == source` is checked in post-removal coordinates, a match
    // means re-inserting into the original slot — a no-op worth reporting.
    let target = target - usize::from(target > source);
    let destination = (target + usize::from(after)).min(places.len());
    if destination == source {
        places.insert(source, place);
        return false;
    }
    places.insert(destination, place);
    true
}

pub(super) fn parse_pinned_drag_source(source: &str) -> Option<usize> {
    source.strip_prefix(PINNED_DRAG_PREFIX)?.parse().ok()
}
