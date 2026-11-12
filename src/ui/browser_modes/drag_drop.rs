// SPDX-License-Identifier: MIT

//! Dragging items out of a pane, dropping onto its folders or the pane itself, and
//! peeking into a folder the pointer rests on.

use std::rc::{Rc, Weak};

use gtk::{gdk, gio, glib, prelude::*};

use super::{
    TransferHandler, TransferHandlerSlot,
    clicks::SlowClickRename,
    source_index::{PanePositions, SourceIndexMap},
};
use crate::{
    app::Browser,
    model::{FileEntry, Location},
    ui::{
        browser::{self, ViewState, paths::is_trash_location},
        pointer,
    },
};

/// Shows the folder an item opens while the pointer rests on it.
pub(super) struct FolderPeek {
    pub(super) state: Option<Weak<ViewState>>,
    pub(super) browser: Weak<Browser>,
    pub(super) positions: PanePositions,
    pub(super) depth: usize,
}

impl FolderPeek {
    pub(super) fn install(self, card: &impl IsA<gtk::Widget>, item: &gtk::ListItem) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let motion = gtk::EventControllerMotion::new();
        let entered_item = item.downgrade();
        let state_for_enter = state.clone();
        motion.connect_enter(move |controller, _, _| {
            let Some(entry) = self.entered_directory(&entered_item) else {
                return;
            };
            if let (Some(state), Some(anchor)) = (state_for_enter.upgrade(), controller.widget()) {
                state.schedule_peek(self.depth, entry.location, anchor);
            }
        });
        motion.connect_leave(move |_| {
            if let Some(state) = state.upgrade() {
                state.schedule_close_peek();
            }
        });
        card.as_ref().add_controller(motion);
    }

    fn entered_directory(&self, item: &glib::WeakRef<gtk::ListItem>) -> Option<FileEntry> {
        let position = item.upgrade()?.position();
        if position == gtk::INVALID_LIST_POSITION {
            return None;
        }
        let source_position = self.positions.source_position(position);
        let entry = self.browser.upgrade().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(self.depth, position))
        })?;
        entry.is_directory().then_some(entry)
    }
}

pub(super) fn install_folder_peek(
    card: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    state: Option<Weak<ViewState>>,
    browser: Weak<Browser>,
    source_index: SourceIndexMap,
    filtered: gio::ListModel,
    depth: usize,
) {
    FolderPeek {
        state,
        browser,
        positions: PanePositions {
            index: source_index,
            view: filtered,
        },
        depth,
    }
    .install(card, item);
}

/// Trash, Recent, and Computer roots refuse dropped files.
fn refuses_drops(destination: &Location) -> bool {
    is_trash_location(destination)
        || destination.is_recent_location()
        || destination.is_computer_root()
}

pub(super) fn install_mode_directory_drop_target(
    widget: &impl IsA<gtk::Widget>,
    destination: Location,
    transfer_handler: TransferHandlerSlot,
) {
    let no_handler = transfer_handler.borrow().is_none();
    if no_handler || refuses_drops(&destination) {
        return;
    }
    widget.add_css_class("file-drop-zone");
    let browser::PreparedFileDrop {
        target: drop,
        state: drop_state,
    } = browser::prepare_file_drop_target({
        let destination = destination.clone();
        move || Some(destination.clone())
    });
    let state_for_enter = drop_state.clone();
    drop.connect_enter(move |target, _, _| browser::file_drop_action(target, &state_for_enter));
    let state_for_motion = drop_state.clone();
    drop.connect_motion(move |target, _, _| browser::file_drop_action(target, &state_for_motion));
    let handler = mode_drop_handler(&transfer_handler);
    let state_for_drop = drop_state.clone();
    drop.connect_drop(move |target, value, _, _| {
        let Some(handler) = transfer_handler.borrow().clone() else {
            return false;
        };
        browser::commit_dropped_value(
            target,
            value,
            destination.clone(),
            &state_for_drop,
            move |destination, request| handler(destination, request),
        )
    });
    browser::attach_drop_target(widget, &drop, &drop_state, handler);
}

/// Reads the slot when a drop lands, as the view's handler can be replaced.
fn mode_drop_handler(transfer_handler: &TransferHandlerSlot) -> TransferHandler {
    let transfer_handler = transfer_handler.clone();
    Rc::new(move |destination, request| {
        if let Some(handler) = transfer_handler.borrow().clone() {
            handler(destination, request);
        }
    })
}

/// The view-to-source position a dragged or dropped-on item maps through, if any.
fn mapped_source_position(positions: Option<&PanePositions>, position: u32) -> Option<usize> {
    positions.map_or(Some(position as usize), |positions| {
        positions.source_position(position)
    })
}

/// Dragging an item out, and dropping files onto it when it is a folder.
pub(super) struct ItemDragDrop {
    pub(super) browser: Weak<Browser>,
    pub(super) transfer: TransferHandlerSlot,
    pub(super) depth: usize,
    pub(super) positions: Option<PanePositions>,
    pub(super) state: Option<Weak<ViewState>>,
}

/// How a drag may start on an item: the click it groups with, whether the item is a
/// List row, and what that click saw selected.
pub(super) struct DragStart<'a> {
    pub(super) content_click: &'a gtk::GestureClick,
    pub(super) list_rows: bool,
    pub(super) intent: Rc<SlowClickRename>,
}

impl ItemDragDrop {
    pub(super) fn install(
        &self,
        row: &impl IsA<gtk::Widget>,
        item: &gtk::ListItem,
        start: DragStart,
    ) {
        if self.transfer.borrow().is_none() {
            return;
        }
        let row = row.as_ref();
        self.install_drag_source(row, item, &start);
        self.install_drop_target(row, item);
    }

    fn install_drag_source(&self, row: &gtk::Widget, item: &gtk::ListItem, start: &DragStart) {
        let drag = gtk::DragSource::builder()
            .actions(gdk::DragAction::COPY | gdk::DragAction::MOVE)
            .build();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let source = ItemDragSource {
            item: item.downgrade(),
            row: row.downgrade(),
            browser: self.browser.clone(),
            positions: self.positions.clone(),
            depth: self.depth,
            list_rows: start.list_rows,
            intent: start.intent.clone(),
        };
        drag.connect_prepare(move |drag, x, y| source.prepare(drag, x, y));
        let dragged_row = row.downgrade();
        let weak_state_for_begin = self.state.clone();
        drag.connect_drag_begin(move |_, _| {
            if let Some(row) = dragged_row.upgrade() {
                row.add_css_class("dragging");
            }
            cancel_peek(weak_state_for_begin.as_ref());
        });
        let dragged_row = row.downgrade();
        let weak_state_for_end = self.state.clone();
        drag.connect_drag_end(move |source, _, _| {
            browser::clear_active_drag();
            source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
            if let Some(row) = dragged_row.upgrade() {
                row.remove_css_class("dragging");
            }
            cancel_peek(weak_state_for_end.as_ref());
        });
        row.add_controller(drag.clone());
        // Grouping requires both gestures already attached; claiming the modifier-click
        // on content must not deny this drag before it reaches the threshold.
        drag.group_with(start.content_click);
    }

    fn install_drop_target(&self, row: &gtk::Widget, item: &gtk::ListItem) {
        let destination = RowDestination {
            item: item.downgrade(),
            browser: self.browser.clone(),
            positions: self.positions.clone(),
            depth: self.depth,
        };
        let browser::PreparedFileDrop {
            target: drop,
            state: drop_state,
        } = browser::prepare_file_drop_target(move || destination.location());
        let spring_navigate: Rc<dyn Fn(Location)> = {
            let browser = self.browser.clone();
            Rc::new(move |location| {
                if let Some(browser) = browser.upgrade() {
                    browser.navigate_location(location, false);
                }
            })
        };
        let hover = {
            let row = row.downgrade();
            let state = drop_state.clone();
            Rc::new(move |target: &gtk::DropTarget| {
                let action = browser::file_drop_action(target, &state);
                let hovered = browser::file_drag_hover_target(&state, target).is_some();
                mark_drop_destination(&row, hovered);
                browser::arm_spring_load_navigation(&state, target, &spring_navigate);
                action
            })
        };
        let hover_on_enter = hover.clone();
        drop.connect_enter(move |target, _, _| hover_on_enter(target));
        let hover_on_motion = hover.clone();
        drop.connect_motion(move |target, _, _| hover_on_motion(target));
        drop.connect_value_notify(move |target| {
            if target.current_drop().is_some() {
                hover(target);
            }
        });
        let highlighted_row = row.downgrade();
        let state_for_leave = drop_state.clone();
        drop.connect_leave(move |_| {
            state_for_leave.cancel_spring_load_navigation();
            mark_drop_destination(&highlighted_row, false);
        });
        let handler = mode_drop_handler(&self.transfer);
        let dropped_row = row.downgrade();
        let state_for_drop = drop_state.clone();
        let transfer_handler = self.transfer.clone();
        drop.connect_drop(move |target, value, _, _| {
            mark_drop_destination(&dropped_row, false);
            state_for_drop.cancel_spring_load_navigation();
            let Some(destination) = state_for_drop.destination() else {
                return false;
            };
            let Some(handler) = transfer_handler.borrow().clone() else {
                return false;
            };
            browser::commit_dropped_value(
                target,
                value,
                destination,
                &state_for_drop,
                move |destination, request| handler(destination, request),
            )
        });
        browser::attach_drop_target(row, &drop, &drop_state, handler);
    }
}

fn cancel_peek(state: Option<&Weak<ViewState>>) {
    if let Some(state) = state.and_then(Weak::upgrade) {
        state.cancel_peek();
    }
}

fn mark_drop_destination(row: &glib::WeakRef<gtk::Widget>, hovered: bool) {
    let Some(row) = row.upgrade() else {
        return;
    };
    if hovered {
        row.add_css_class("drop-destination");
    } else {
        row.remove_css_class("drop-destination");
    }
}

/// The folder a dropped-on item opens.
struct RowDestination {
    item: glib::WeakRef<gtk::ListItem>,
    browser: Weak<Browser>,
    positions: Option<PanePositions>,
    depth: usize,
}

impl RowDestination {
    fn location(&self) -> Option<Location> {
        let browser = self.browser.upgrade()?;
        let position = self.item.upgrade()?.position();
        let position = match &self.positions {
            None => (position != gtk::INVALID_LIST_POSITION).then_some(position as usize),
            Some(positions) => positions.source_position(position),
        };
        position
            .and_then(|position| browser.entry_at(self.depth, position))
            .filter(FileEntry::is_directory)
            .map(|entry| entry.location)
    }
}

/// The files a drag from one item carries.
struct ItemDragSource {
    item: glib::WeakRef<gtk::ListItem>,
    row: glib::WeakRef<gtk::Widget>,
    browser: Weak<Browser>,
    positions: Option<PanePositions>,
    depth: usize,
    list_rows: bool,
    intent: Rc<SlowClickRename>,
}

impl ItemDragSource {
    fn prepare(&self, source: &gtk::DragSource, x: f64, y: f64) -> Option<gdk::ContentProvider> {
        let row = self.row.upgrade()?;
        if !self.grabs_item(&row, x, y) {
            return None;
        }
        source.set_actions(browser::drag_actions_for_modifiers(
            source.current_event_state(),
        ));
        let browser = self.browser.upgrade()?;
        let position = self.item.upgrade()?.position();
        if position == gtk::INVALID_LIST_POSITION {
            return None;
        }
        let position = mapped_source_position(self.positions.as_ref(), position)?;
        let entry = browser.entry_at(self.depth, position)?;
        let entries = if browser.active_selection_contains(self.depth, position) {
            browser.selected_entries()
        } else {
            vec![entry]
        };
        if let Some((texture, hot_x, hot_y)) = browser::drag_preview_icon(&row, &entries) {
            source.set_icon(Some(&texture), hot_x, hot_y);
        }
        browser::file_drag_content(source, &entries)
    }

    /// List keeps the same icon/name hit region before and after selection.
    /// Icons retains its selected-card drag region. Editors keep their own drags.
    fn grabs_item(&self, row: &gtk::Widget, x: f64, y: f64) -> bool {
        let on_editable = row
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|target| crate::ui::focus_navigation::editable(&target));
        if on_editable {
            return false;
        }
        (!self.list_rows && self.intent.was_selected()) || self.hits_content(row, x, y)
    }

    fn hits_content(&self, row: &gtk::Widget, x: f64, y: f64) -> bool {
        if self.list_rows {
            pointer::hits_list_item_content(row, x, y)
        } else {
            pointer::hits_icon_card_content(row, x, y)
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "List rows call this with their inputs explicit"
)]
pub(super) fn install_list_drag_drop(
    row: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    browser: Weak<Browser>,
    transfer_handler: TransferHandlerSlot,
    depth: usize,
    position_map: Option<(SourceIndexMap, gio::ListModel)>,
    state: Option<Weak<ViewState>>,
    drag_icon_and_content_click: (&gtk::GestureClick, bool, Rc<SlowClickRename>),
) {
    let (content_click, list_rows, intent) = drag_icon_and_content_click;
    ItemDragDrop {
        browser,
        transfer: transfer_handler,
        depth,
        positions: position_map.map(|(index, view)| PanePositions { index, view }),
        state,
    }
    .install(
        row,
        item,
        DragStart {
            content_click,
            list_rows,
            intent,
        },
    );
}
