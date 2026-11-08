// SPDX-License-Identifier: MIT

//! Primary and middle clicks on a file row: selection, slow-click rename and activation.

use super::super::{
    FolderPress, PendingActivationKind, PendingPointerActivation, should_activate_single_click,
    should_preview_pointer_press,
};
use super::{RowContext, RowHandle};
use crate::model::{FileEntry, Location};
use crate::ui::browser::{
    ViewState, collection::activate_recursive_search_result,
    entry::entry_responds_to_preview_click, paths::is_trash_location,
    tab_location::TabLocationHold,
};
use crate::ui::browser_modes::{FilteredFileClick, filtered_file_click};
use crate::ui::collection_interaction::{
    MiddleClickAction, PointerSequence, SelectionChange, picks_editable,
};
use gtk::{gdk::ModifierType, glib, prelude::*};
use std::{cell::Cell, rc::Rc};

fn anchored_row(context: &RowContext) -> Option<u32> {
    let state = context.state.upgrade()?;
    let anchor = state.browser.selection_anchor_position(context.depth)?;
    context.map.view_position(anchor)
}

fn anchor_at(context: &RowContext, row: u32) {
    if let (Some(state), Some(source_position)) =
        (context.state.upgrade(), context.map.source_position(row))
    {
        state
            .browser
            .set_selection_anchor(context.depth, source_position);
    }
}

fn release_kind(
    activate: bool,
    plain_file_press: bool,
    previews: bool,
) -> Option<PendingActivationKind> {
    if activate {
        Some(PendingActivationKind::Standard { preview: false })
    } else if !plain_file_press {
        None
    } else if previews {
        Some(PendingActivationKind::Standard { preview: true })
    } else {
        Some(PendingActivationKind::CloseDeeper)
    }
}

/// Filtered folders keep one-click open.
fn filtered_click_kind(
    state: &ViewState,
    entry: &FileEntry,
    recursive: bool,
) -> Option<PendingActivationKind> {
    let chooser = state.browser.is_chooser_mode();
    if entry.is_directory() {
        return Some(filtered_folder_click_kind(recursive, chooser));
    }
    match filtered_file_click(
        entry,
        chooser,
        state.single_click_previews.get(),
        state.columns_click_activation.get().files,
    ) {
        FilteredFileClick::Open if recursive => Some(PendingActivationKind::RecursiveSearch),
        FilteredFileClick::Open => Some(PendingActivationKind::Mapped),
        FilteredFileClick::Preview if recursive => Some(PendingActivationKind::RecursivePreview),
        FilteredFileClick::Preview => Some(PendingActivationKind::Standard { preview: true }),
        FilteredFileClick::Ignore => None,
    }
}

fn filtered_folder_click_kind(recursive: bool, chooser: bool) -> PendingActivationKind {
    if recursive && chooser {
        PendingActivationKind::ChooserSearchNavigate
    } else if recursive {
        PendingActivationKind::RecursiveSearch
    } else {
        PendingActivationKind::Mapped
    }
}

fn open_row_in_new(state: &Rc<ViewState>, location: &Location, action: &str) {
    let uri = crate::adapters::gio_file_for_location(location)
        .uri()
        .to_variant();
    if let Err(error) = state.overlay.activate_action(action, Some(&uri)) {
        tracing::warn!(%error, action, "open-in-new location action unavailable");
    }
}

pub(super) fn open_middle_clicked(context: &RowContext, position: u32, modifiers: ModifierType) {
    let Some(state) = context.state.upgrade() else {
        return;
    };
    let Some(entry) = context.view_entry(&state, position) else {
        return;
    };
    let control = modifiers.contains(ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(ModifierType::SHIFT_MASK);
    match crate::ui::collection_interaction::middle_click_action(
        entry.is_directory(),
        control,
        shift,
    ) {
        MiddleClickAction::OpenTab => {
            open_row_in_new(&state, &entry.location, "win.open-tab-at");
        }
        MiddleClickAction::OpenWindow => {
            open_row_in_new(&state, &entry.location, "win.open-window-at");
        }
        MiddleClickAction::RevealParent => {
            if let Some(parent) = entry.location.parent() {
                state.reveal_locations(parent, vec![entry.location], false);
            }
        }
    }
}

/// Remembers a plain, unfiltered folder press so the next one can complete a double-click.
fn record_folder_press(state: &ViewState, press: &RowPress, depth: usize, entry: &FileEntry) {
    if press.filtered || !press.unmodified() {
        return;
    }
    if let Some(position) = press.source_position {
        state.folder_press.replace(Some(FolderPress {
            at: std::time::Instant::now(),
            point: press.point,
            depth,
            position,
            location: entry.location.clone(),
        }));
    }
}

/// A Control or Shift press on the row's content moves keyboard focus to it.
fn focus_modified_press(gesture: &gtk::GestureClick, press: &RowPress) {
    if !(press.control || press.shift) {
        return;
    }
    let Some(widget) = gesture.widget() else {
        return;
    };
    if !crate::ui::pointer::hits_item_content(&widget, press.x, press.y) {
        return;
    }
    if let Some(item_widget) = widget.parent() {
        item_widget.grab_focus();
    }
}

/// One primary-button press on a row.
struct RowPress {
    position: u32,
    count: i32,
    /// Where the press began; activation is cancelled once the pointer leaves it.
    point: (f64, f64),
    x: f64,
    y: f64,
    modifiers: ModifierType,
    control: bool,
    shift: bool,
    filtered: bool,
    source_position: Option<usize>,
}

impl RowPress {
    fn unmodified(&self) -> bool {
        !self.control && !self.shift
    }

    fn plain_single(&self) -> bool {
        self.count == 1 && self.unmodified()
    }
}

/// What the press did to the selection, and whether it opens the entry.
struct PressOutcome {
    selected_before: bool,
    selected_count_before: u64,
    preserve_group: bool,
    activate: bool,
    location_hold: Option<TabLocationHold>,
}

/// One row's primary-button gesture.
pub(super) struct RowClick {
    handle: RowHandle,
    label: glib::WeakRef<gtk::Label>,
    sequence: PointerSequence,
    press_moved: Cell<bool>,
    rename_position: Cell<Option<usize>>,
}

impl RowClick {
    pub(super) fn install(
        handle: &RowHandle,
        row: &gtk::Box,
        label: &gtk::Label,
    ) -> gtk::GestureClick {
        let selection_click = gtk::GestureClick::new();
        selection_click.set_button(1);
        selection_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let click = Rc::new(Self {
            handle: handle.clone(),
            label: label.downgrade(),
            sequence: PointerSequence::default(),
            press_moved: Cell::new(false),
            rename_position: Cell::new(None),
        });
        click.sequence.install(&selection_click);
        // Open on release so a press-and-move can start a drag first.
        let pressed = click.clone();
        selection_click.connect_pressed(move |gesture, press_count, x, y| {
            pressed.pressed(gesture, press_count, x, y);
        });
        let updated = click.clone();
        selection_click.connect_update(move |gesture, _| updated.updated(gesture));
        let released = click.clone();
        selection_click.connect_released(move |gesture, count, x, y| {
            released.released(gesture, count, x, y);
        });
        selection_click.connect_cancel(move |_, _| click.cancelled());
        row.add_controller(selection_click.clone());
        selection_click
    }

    fn context(&self) -> &RowContext {
        &self.handle.context
    }

    fn pressed(&self, gesture: &gtk::GestureClick, press_count: i32, x: f64, y: f64) {
        let pointer = &self.handle.pointer;
        pointer.pending_activation.take();
        self.rename_position.set(None);
        pointer.was_selected.set(false);
        self.press_moved.set(false);
        // Focusing a parent can scroll the row beneath a stationary pointer.
        let point = gesture
            .current_event()
            .and_then(|event| event.position())
            .unwrap_or((x, y));
        pointer.press_origin.set(point);
        let context = self.context();
        if let Some(state) = context.state.upgrade() {
            state.cancel_click_rename();
        }
        if picks_editable(gesture, x, y) {
            return;
        }
        let Some(clicked_item) = self.handle.item.upgrade() else {
            return;
        };
        let position = clicked_item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        let modifiers = gesture.current_event_state();
        let press = RowPress {
            position,
            count: press_count,
            point,
            x,
            y,
            modifiers,
            control: modifiers.contains(ModifierType::CONTROL_MASK),
            shift: modifiers.contains(ModifierType::SHIFT_MASK),
            filtered: context.search_active.get() || context.map.has_query(),
            source_position: context.map.source_position(position),
        };
        self.sequence.press(modifiers);
        let outcome = self.select_pressed(&press);
        focus_modified_press(gesture, &press);
        self.sequence
            .claim_double_press(gesture, press_count, (x, y));
        self.arm_pointer_activation(gesture, &press, outcome);
    }

    fn arm_pointer_activation(
        &self,
        gesture: &gtk::GestureClick,
        press: &RowPress,
        outcome: PressOutcome,
    ) {
        if press.count == 2 && press.unmodified() {
            self.arm_double_activation(press);
        } else if press.filtered {
            self.arm_filtered_activation(press, outcome.location_hold);
        } else {
            self.arm_standard_activation(gesture, press, outcome);
        }
    }

    fn select_pressed(&self, press: &RowPress) -> PressOutcome {
        let context = self.context();
        let selection = &context.selection;
        let selected_before = selection.is_selected(press.position);
        let selected_count_before = selection.selection().size();
        self.handle.pointer.was_selected.set(selected_before);
        let change = crate::ui::collection_interaction::pointer_selection(
            &selection.selection(),
            press.position,
            anchored_row(context),
            context
                .state
                .upgrade()
                .is_none_or(|state| state.multiple_selection.get()),
            press.modifiers,
        );
        let preserve_group = change.preserved_group;
        let (activate, location_hold) = self.classify_press(press).unwrap_or((false, None));
        if !press.shift
            && let Some(anchor) = change.anchor
        {
            anchor_at(context, anchor);
        }
        self.apply_pointer_selection(&change);
        PressOutcome {
            selected_before,
            selected_count_before,
            preserve_group,
            activate,
            location_hold,
        }
    }

    /// Whether the press opens its entry, and the tab location held while it does.
    fn classify_press(&self, press: &RowPress) -> Option<(bool, Option<TabLocationHold>)> {
        let context = self.context();
        let depth = context.depth;
        let state = context.state.upgrade()?;
        let entry = press
            .source_position
            .and_then(|position| state.browser.entry_at(depth, position))?;
        let directory = entry.is_directory();
        if directory {
            record_folder_press(&state, press, depth, &entry);
        }
        let activate = !press.filtered
            && should_activate_single_click(
                press.plain_single(),
                directory,
                state.columns_click_activation.get(),
            );
        let holds_location = !context.search_active.get()
            && directory
            && (activate || (press.filtered && press.plain_single()));
        let location_hold = holds_location.then(|| state.hold_tab_location());
        if activate && let Some(hold) = &location_hold {
            hold.opening(depth, entry.location.clone());
        }
        Some((activate, location_hold))
    }

    /// Owns this write for the whole synchronous selection-changed emission.
    /// A click on the cursor row is otherwise identical to a focus echo.
    fn apply_pointer_selection(&self, change: &SelectionChange) {
        let selection = &self.context().selection;
        let pointer_owner = self
            .context()
            .state
            .upgrade()
            .inspect(|state| state.pointer_owns_selection.set(true));
        selection.set_selection(
            &change.selected,
            &gtk::Bitset::new_range(0, selection.n_items()),
        );
        if let Some(state) = pointer_owner {
            state.pointer_owns_selection.set(false);
        }
    }

    fn arm(&self, activation: PendingPointerActivation) {
        self.handle
            .pointer
            .pending_activation
            .replace(Some(activation));
    }

    fn arm_double_activation(&self, press: &RowPress) {
        let context = self.context();
        let Some(state) = context.state.upgrade() else {
            return;
        };
        let Some(entry) = context.view_entry(&state, press.position) else {
            return;
        };
        let (position, kind) = if context.search_active.get() {
            (
                press.position as usize,
                PendingActivationKind::RecursiveSearch,
            )
        } else if let Some(position) = press.source_position {
            (position, PendingActivationKind::Standard { preview: false })
        } else {
            return;
        };
        self.arm(PendingPointerActivation {
            position,
            location: entry.location,
            press: press.point,
            moved: false,
            kind,
            location_hold: None,
        });
    }

    fn arm_filtered_activation(&self, press: &RowPress, location_hold: Option<TabLocationHold>) {
        if !press.plain_single() {
            return;
        }
        let context = self.context();
        let Some(state) = context.state.upgrade() else {
            return;
        };
        let position = press.position;
        let pending = if context.search_active.get() {
            context.search_entry(position).and_then(|entry| {
                let kind = filtered_click_kind(&state, &entry, true)?;
                Some((position as usize, entry.location, kind))
            })
        } else {
            context
                .map
                .source_position(position)
                .and_then(|source_position| {
                    let entry = state.browser.entry_at(context.depth, source_position)?;
                    state.browser.select(context.depth, source_position);
                    let kind = filtered_click_kind(&state, &entry, false)?;
                    Some((source_position, entry.location, kind))
                })
        };
        if let Some((position, location, kind)) = pending {
            self.arm(PendingPointerActivation {
                position,
                location,
                press: press.point,
                moved: false,
                kind,
                location_hold,
            });
        }
    }

    /// A plain press on the only selected row's name renames it on release.
    fn is_slow_click_rename(
        &self,
        press: &RowPress,
        outcome: &PressOutcome,
        reopening: bool,
    ) -> bool {
        let unmodified = !press.modifiers.intersects(
            ModifierType::CONTROL_MASK
                | ModifierType::SHIFT_MASK
                | ModifierType::ALT_MASK
                | ModifierType::SUPER_MASK
                | ModifierType::META_MASK,
        );
        press.count == 1
            && outcome.selected_before
            && outcome.selected_count_before == 1
            && unmodified
            && !outcome.preserve_group
            && (!outcome.activate || reopening)
    }

    fn renames_entry(&self, state: &ViewState, entry: &FileEntry) -> bool {
        !state.browser.is_chooser_mode() && !is_trash_location(&entry.location)
    }

    fn hits_name(&self, gesture: &gtk::GestureClick, press: &RowPress) -> bool {
        gesture
            .widget()
            .zip(self.label.upgrade())
            .is_some_and(|(row, label)| {
                crate::ui::pointer::hits_name_label(&row, label.upcast_ref(), press.x, press.y)
            })
    }

    fn arm_standard_activation(
        &self,
        gesture: &gtk::GestureClick,
        press: &RowPress,
        outcome: PressOutcome,
    ) {
        let context = self.context();
        let (Some(state), Some(source_position)) = (context.state.upgrade(), press.source_position)
        else {
            return;
        };
        let Some(entry) = state.browser.entry_at(context.depth, source_position) else {
            return;
        };
        let reopening = state.browser.is_open_child(context.depth, &entry.location);
        let slow_click_rename = self.is_slow_click_rename(press, &outcome, reopening)
            && self.renames_entry(&state, &entry)
            && self.hits_name(gesture, press);
        let activate = outcome.activate && !slow_click_rename;
        self.rename_position
            .set(slow_click_rename.then_some(source_position));
        let plain_file_press = !activate
            && !slow_click_rename
            && !entry.is_directory()
            && should_preview_pointer_press(press.count, press.control, press.shift);
        let previews = entry_responds_to_preview_click(&entry, state.single_click_previews.get());
        let Some(kind) = release_kind(activate, plain_file_press, previews) else {
            return;
        };
        self.arm(PendingPointerActivation {
            position: source_position,
            location: entry.location.clone(),
            press: press.point,
            moved: false,
            kind,
            location_hold: outcome.location_hold,
        });
    }

    fn updated(&self, gesture: &gtk::GestureClick) {
        let Some((x, y)) = gesture.current_event().and_then(|event| event.position()) else {
            return;
        };
        let Some(widget) = gesture.widget() else {
            return;
        };
        let threshold = widget.settings().gtk_dnd_drag_threshold();
        let cancelled_hold = self
            .handle
            .pointer
            .pending_activation
            .borrow_mut()
            .as_mut()
            .and_then(|pending| {
                pending.update(x, y, threshold);
                if pending.moved {
                    pending.location_hold.take()
                } else {
                    None
                }
            });
        drop(cancelled_hold);
        if crate::ui::pointer::exceeds_drag_threshold(
            self.handle.pointer.press_origin.get(),
            (x, y),
            threshold,
        ) {
            self.press_moved.set(true);
        }
    }

    /// A still, unmodified release on a row that was already selected.
    fn releases_into_rename(&self, gesture: &gtk::GestureClick, count: i32) -> bool {
        count == 1
            && !self.press_moved.get()
            && !gesture
                .current_event_state()
                .intersects(ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK)
            && self.handle.pointer.was_selected.get()
    }

    fn schedule_click_rename(&self) {
        if let Some(state) = self.context().state.upgrade()
            && let Some(position) = self.rename_position.get()
        {
            state.schedule_click_rename(self.context().depth, position);
        }
    }

    fn released(&self, gesture: &gtk::GestureClick, count: i32, x: f64, y: f64) {
        let pending = self.handle.pointer.pending_activation.take();
        let (x, y) = gesture
            .current_event()
            .and_then(|event| event.position())
            .unwrap_or((x, y));
        if pending.is_none() && self.releases_into_rename(gesture, count) {
            self.schedule_click_rename();
        }
        let Some(mut pending) = pending else {
            return;
        };
        let Some(widget) = gesture.widget() else {
            return;
        };
        pending.update(x, y, widget.settings().gtk_dnd_drag_threshold());
        let Some(state) = self.context().state.upgrade() else {
            return;
        };
        // GTK 4.14's DragSource needs the release to reset before the next press.
        self.activate_pending(&state, pending);
    }

    fn activate_pending(&self, state: &Rc<ViewState>, mut pending: PendingPointerActivation) {
        match pending.kind {
            PendingActivationKind::Standard { preview } => {
                self.activate_standard(state, &mut pending, preview);
            }
            PendingActivationKind::Mapped => self.activate_mapped(state, &mut pending),
            PendingActivationKind::ChooserSearchNavigate => {
                self.navigate_chooser_search(state, &pending);
            }
            PendingActivationKind::RecursiveSearch => {
                self.activate_recursive_search(state, &pending);
            }
            PendingActivationKind::RecursivePreview => {
                self.preview_recursive_search(state, &pending);
            }
            PendingActivationKind::CloseDeeper => {
                let deeper = self.context().depth + 1;
                if self.pending_entry_matches(state, &pending)
                    && state.browser.location_at(deeper).is_some()
                {
                    state.browser.close_column(deeper);
                }
            }
        }
    }

    fn pending_entry_matches(&self, state: &ViewState, pending: &PendingPointerActivation) -> bool {
        state
            .browser
            .entry_at(self.context().depth, pending.position)
            .is_some_and(|entry| pending.can_activate(&entry.location))
    }

    fn activate_standard(
        &self,
        state: &Rc<ViewState>,
        pending: &mut PendingPointerActivation,
        preview: bool,
    ) {
        if !self.pending_entry_matches(state, pending) {
            return;
        }
        let depth = self.context().depth;
        let position = pending.position;
        if preview {
            if state.single_click_previews.get() {
                state.browser.preview(depth, position);
            }
            return;
        }
        let activate = || state.activate_column_entry(depth, position);
        if let Some(hold) = pending.location_hold.take() {
            hold.navigate(depth, activate);
        } else {
            activate();
        }
    }

    fn activate_mapped(&self, state: &Rc<ViewState>, pending: &mut PendingPointerActivation) {
        if !self.pending_entry_matches(state, pending) {
            return;
        }
        if state.browser.is_chooser_mode() {
            return;
        }
        let depth = self.context().depth;
        let position = pending.position;
        let activate = || state.browser.activate_in_place(depth, position);
        if let Some(hold) = pending.location_hold.take() {
            hold.navigate(depth, activate);
        } else {
            activate();
        }
    }

    fn navigate_chooser_search(&self, state: &ViewState, pending: &PendingPointerActivation) {
        let Some(item) = self
            .context()
            .search_results
            .borrow()
            .get(pending.position)
            .cloned()
        else {
            return;
        };
        let location = Location::local(item.path.clone());
        if item.is_directory && pending.can_activate(&location) {
            state.browser.navigate(location);
        }
    }

    fn activate_recursive_search(&self, state: &ViewState, pending: &PendingPointerActivation) {
        let search_results = &self.context().search_results;
        let matches = search_results
            .borrow()
            .get(pending.position)
            .is_some_and(|item| pending.can_activate(&Location::local(item.path.clone())));
        if matches {
            activate_recursive_search_result(
                &Rc::downgrade(&state.browser),
                search_results,
                pending.position as u32,
            );
        }
    }

    fn preview_recursive_search(&self, state: &ViewState, pending: &PendingPointerActivation) {
        let Some(item) = self
            .context()
            .search_results
            .borrow()
            .get(pending.position)
            .cloned()
        else {
            return;
        };
        let entry = crate::ui::browser::search_result_entry(&item);
        if pending.can_activate(&entry.location) && state.single_click_previews.get() {
            state.browser.request_preview(entry);
        }
    }

    fn cancelled(&self) {
        self.handle.pointer.pending_activation.take();
        if let Some(state) = self.context().state.upgrade() {
            state.cancel_click_rename();
        }
    }
}
