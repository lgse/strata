// SPDX-License-Identifier: MIT

//! Pointer clicks on a rendered item: activation, previews, slow-click rename, and
//! modified selection.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{gdk, gio, glib, prelude::*};

use super::{
    ClickActivation, ClickCount,
    search_options::{FilteredFileClick, filtered_file_click, should_activate_filtered_pointer},
    source_index::{PanePositions, SourceIndexMap},
};
use crate::{
    app::Browser,
    model::{FileEntry, Location},
    ui::{
        browser::{ViewState, paths::is_trash_location},
        collection_interaction::{self, MiddleClickAction, PointerSequence},
        pointer,
    },
};

// Capture must snapshot selection before GTK changes it for the bubble gesture.
#[derive(Default)]
pub(super) struct SlowClickRename {
    was_selected: Cell<bool>,
    selected_count_before: Cell<u64>,
}

impl SlowClickRename {
    pub(super) fn was_selected(&self) -> bool {
        self.was_selected.get()
    }

    /// The item was the only selected one before this press.
    fn was_sole_selection(&self) -> bool {
        self.was_selected.get() && self.selected_count_before.get() == 1
    }
}

fn press_modifiers(modifiers: gdk::ModifierType) -> (bool, bool) {
    (
        modifiers.contains(gdk::ModifierType::CONTROL_MASK),
        modifiers.contains(gdk::ModifierType::SHIFT_MASK),
    )
}

/// The view position of a still-shown item.
fn listed_position(item: &glib::WeakRef<gtk::ListItem>) -> Option<u32> {
    let position = item.upgrade()?.position();
    (position != gtk::INVALID_LIST_POSITION).then_some(position)
}

/// A plain click's activation, preview, and slow-click rename.
pub(super) struct PreviewClick {
    pub(super) browser: Weak<Browser>,
    pub(super) state: Weak<ViewState>,
    pub(super) previews: Rc<Cell<bool>>,
    pub(super) activation: Rc<Cell<ClickActivation>>,
    pub(super) depth: usize,
    pub(super) positions: Option<PanePositions>,
    pub(super) filter_query: Rc<RefCell<String>>,
    pub(super) slow_click: Rc<SlowClickRename>,
}

/// The entry a click landed on.
struct ClickedEntry {
    browser: Rc<Browser>,
    position: usize,
    entry: FileEntry,
}

impl PreviewClick {
    pub(super) fn install(
        self,
        widget: &impl IsA<gtk::Widget>,
        item: &gtk::ListItem,
        rename_label: &impl IsA<gtk::Widget>,
    ) {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        let clicked_item = item.downgrade();
        let rename_label = rename_label.as_ref().downgrade();
        pointer::connect_click_release(&click, item, move |gesture, press_count, x, y| {
            let modifiers = gesture.current_event_state();
            if modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK)
            {
                return;
            }
            let Some(clicked) = self.clicked_entry(&clicked_item) else {
                return;
            };
            let release = Release {
                gesture,
                press_count,
                modifiers,
                at: (x, y),
            };
            self.released(&release, &clicked, &rename_label);
        });
        widget.add_controller(click);
    }

    fn clicked_entry(&self, item: &glib::WeakRef<gtk::ListItem>) -> Option<ClickedEntry> {
        let position = listed_position(item)?;
        let source_position = self
            .positions
            .as_ref()
            .map_or(Some(position as usize), |positions| {
                positions.source_position(position)
            });
        let browser = self.browser.upgrade()?;
        let position = source_position?;
        let entry = browser.entry_at(self.depth, position)?;
        Some(ClickedEntry {
            browser,
            position,
            entry,
        })
    }

    fn released(
        &self,
        release: &Release,
        clicked: &ClickedEntry,
        rename_label: &glib::WeakRef<gtk::Widget>,
    ) {
        let ClickedEntry {
            browser,
            position,
            entry,
        } = clicked;
        let is_directory = entry.is_directory();
        if should_activate_filtered_pointer(release.press_count, &self.filter_query) {
            release.gesture.set_state(gtk::EventSequenceState::Claimed);
            browser.select(self.depth, *position);
            self.open_filtered(clicked);
        } else if should_activate_pointer_click(
            release.press_count,
            is_directory,
            self.activation.get(),
        ) {
            if !browser.is_chooser_mode() || is_directory {
                release.gesture.set_state(gtk::EventSequenceState::Claimed);
                browser.activate_in_place(self.depth, *position);
            }
        } else if self.starts_slow_rename(release, clicked) {
            self.schedule_rename(release, rename_label, *position);
        } else if self.previews_on_click(release.press_count, entry) {
            browser.preview(self.depth, *position);
        }
    }

    /// A filtered listing opens folders and follows the file click rules on one press.
    fn open_filtered(&self, clicked: &ClickedEntry) {
        let click = if clicked.entry.is_directory() {
            FilteredFileClick::Open
        } else {
            filtered_file_click(
                &clicked.entry,
                clicked.browser.is_chooser_mode(),
                self.previews.get(),
                self.activation.get().files,
            )
        };
        match click {
            FilteredFileClick::Open => clicked
                .browser
                .activate_in_place(self.depth, clicked.position),
            FilteredFileClick::Preview => clicked.browser.preview(self.depth, clicked.position),
            FilteredFileClick::Ignore => {}
        }
    }

    /// A plain single press on the only selected item may rename it.
    fn starts_slow_rename(&self, release: &Release, clicked: &ClickedEntry) -> bool {
        let plain = !release.modifiers.intersects(
            gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK
                | gdk::ModifierType::META_MASK,
        );
        let renames_here =
            !clicked.browser.is_chooser_mode() && !is_trash_location(&clicked.entry.location);
        release.press_count == 1 && plain && self.slow_click.was_sole_selection() && renames_here
    }

    fn schedule_rename(
        &self,
        release: &Release,
        rename_label: &glib::WeakRef<gtk::Widget>,
        position: usize,
    ) {
        let (Some(surface), Some(label)) = (release.gesture.widget(), rename_label.upgrade())
        else {
            return;
        };
        let (x, y) = release.at;
        if !pointer::hits_name_label(&surface, &label, x, y) {
            return;
        }
        if let Some(state) = self.state.upgrade() {
            state.schedule_click_rename(self.depth, position);
        }
    }

    fn previews_on_click(&self, press_count: i32, entry: &FileEntry) -> bool {
        let previewable =
            !entry.is_directory() && crate::ui::preview::entry_supports_quick_preview(entry);
        press_count == 1 && self.previews.get() && previewable
    }
}

/// One button release on an item.
struct Release<'a> {
    gesture: &'a gtk::GestureClick,
    press_count: i32,
    modifiers: gdk::ModifierType,
    at: (f64, f64),
}

#[expect(
    clippy::too_many_arguments,
    reason = "List rows call this with their inputs explicit"
)]
pub(super) fn install_preview_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    rename_label: &impl IsA<gtk::Widget>,
    browser: Weak<Browser>,
    weak_state: Weak<ViewState>,
    enabled: Rc<Cell<bool>>,
    click_activation: Rc<Cell<ClickActivation>>,
    depth: usize,
    position_map: Option<(SourceIndexMap, gio::ListModel)>,
    filter_query: Rc<RefCell<String>>,
    slow_click: Rc<SlowClickRename>,
) {
    PreviewClick {
        browser,
        state: weak_state,
        previews: enabled,
        activation: click_activation,
        depth,
        positions: position_map.map(|(index, view)| PanePositions { index, view }),
        filter_query,
        slow_click,
    }
    .install(widget, item, rename_label);
}

fn should_activate_pointer_click(
    press_count: i32,
    is_directory: bool,
    activation: ClickActivation,
) -> bool {
    let configured = if is_directory {
        activation.folders
    } else {
        activation.files
    };
    press_count == 1 && configured == ClickCount::One
}

/// Control and Shift clicks that edit the selection, and middle clicks that open in new.
pub(super) struct SelectionClick {
    pub(super) selection: gtk::MultiSelection,
    pub(super) syncing: Rc<Cell<bool>>,
    pub(super) browser: Weak<Browser>,
    pub(super) state: Weak<ViewState>,
    pub(super) depth: usize,
    pub(super) positions: PanePositions,
    pub(super) slow_click: Rc<SlowClickRename>,
}

impl SelectionClick {
    pub(super) fn install(
        self,
        widget: &impl IsA<gtk::Widget>,
        item: &gtk::ListItem,
    ) -> gtk::GestureClick {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let sequence = PointerSequence::default();
        sequence.install(&click);
        let middle = MiddleClick {
            browser: self.browser.clone(),
            row: widget.upcast_ref::<gtk::Widget>().downgrade(),
            positions: self.positions.clone(),
            depth: self.depth,
        };
        collection_interaction::install_row_middle_click(
            widget,
            item,
            move |position, modifiers| {
                middle.clicked(position, modifiers);
            },
        );
        let item = item.downgrade();
        let weak_state_for_cancel = self.state.clone();
        click.connect_pressed(move |gesture, _, x, y| {
            self.pressed(gesture, &item, &sequence, (x, y));
        });
        click.connect_cancel(move |_, _| {
            if let Some(state) = weak_state_for_cancel.upgrade() {
                state.cancel_click_rename();
            }
        });
        widget.add_controller(click.clone());
        click
    }

    fn pressed(
        &self,
        gesture: &gtk::GestureClick,
        item: &glib::WeakRef<gtk::ListItem>,
        sequence: &PointerSequence,
        at: (f64, f64),
    ) {
        let Some(position) = listed_position(item) else {
            return;
        };
        let Some(browser) = self.browser.upgrade() else {
            return;
        };
        if let Some(state) = self.state.upgrade() {
            state.cancel_click_rename();
        }
        let modifiers = gesture.current_event_state();
        let (control, shift) = press_modifiers(modifiers);
        sequence.press(modifiers);
        self.slow_click
            .was_selected
            .set(self.selection.is_selected(position));
        self.slow_click
            .selected_count_before
            .set(self.selection.selection().size());
        self.select(gesture, &browser, position, modifiers);
        if !control && !shift {
            return;
        }
        focus_clicked_card(gesture, at);
    }

    fn select(
        &self,
        gesture: &gtk::GestureClick,
        browser: &Browser,
        position: u32,
        modifiers: gdk::ModifierType,
    ) {
        let (control, shift) = press_modifiers(modifiers);
        let change = collection_interaction::pointer_selection(
            &self.selection.selection(),
            position,
            browser
                .selection_anchor_position(self.depth)
                .and_then(|anchor| self.positions.view_position(anchor)),
            true,
            modifiers,
        );
        if let Some(anchor) = change.anchor.filter(|_| !shift) {
            anchor_at(browser, self.depth, &self.positions, anchor);
        }
        let moves_origin = control && !shift;
        if moves_origin && let Some(row) = gesture.widget() {
            move_range_origin(&row, &self.selection, &self.syncing, position);
        }
        self.selection.set_selection(
            &change.selected,
            &gtk::Bitset::new_range(0, self.selection.n_items()),
        );
    }
}

/// A modified click on an icon card's content focuses that card.
fn focus_clicked_card(gesture: &gtk::GestureClick, (x, y): (f64, f64)) {
    let Some(widget) = gesture.widget() else {
        return;
    };
    if !pointer::hits_icon_card_content(&widget, x, y) {
        return;
    }
    if let Some(item_widget) = widget.parent() {
        item_widget.grab_focus();
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "List rows call this with their inputs explicit"
)]
pub(super) fn install_modified_selection_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    selection: gtk::MultiSelection,
    syncing: Rc<Cell<bool>>,
    browser: Weak<Browser>,
    weak_state: Weak<ViewState>,
    depth: usize,
    positions: PanePositions,
    slow_click: Rc<SlowClickRename>,
) -> gtk::GestureClick {
    SelectionClick {
        selection,
        syncing,
        browser,
        state: weak_state,
        depth,
        positions,
        slow_click,
    }
    .install(widget, item)
}

/// A middle click opens the item in a new tab or window, or reveals it in its folder.
struct MiddleClick {
    browser: Weak<Browser>,
    row: glib::WeakRef<gtk::Widget>,
    positions: PanePositions,
    depth: usize,
}

impl MiddleClick {
    fn clicked(&self, view_position: u32, modifiers: gdk::ModifierType) {
        let Some(browser) = self.browser.upgrade() else {
            return;
        };
        let Some(source_position) = self.positions.source_position(view_position) else {
            return;
        };
        let Some(entry) = browser.entry_at(self.depth, source_position) else {
            return;
        };
        let (control, shift) = press_modifiers(modifiers);
        match collection_interaction::middle_click_action(entry.is_directory(), control, shift) {
            MiddleClickAction::OpenTab => {
                open_entry_in_new(&self.row, &entry.location, "win.open-tab-at");
            }
            MiddleClickAction::OpenWindow => {
                open_entry_in_new(&self.row, &entry.location, "win.open-window-at");
            }
            MiddleClickAction::RevealParent => {
                if let Some(parent) = entry.location.parent() {
                    browser.reveal_locations(parent, vec![entry.location]);
                }
            }
        }
    }
}

fn open_entry_in_new(row: &glib::WeakRef<gtk::Widget>, location: &Location, action: &str) {
    let Some(widget) = row.upgrade() else {
        return;
    };
    let uri = crate::adapters::gio_file_for_location(location)
        .uri()
        .to_variant();
    if let Err(error) = widget.activate_action(action, Some(&uri)) {
        tracing::warn!(%error, action, "open-in-new location action unavailable");
    }
}

/// Modified clicks are claimed before GTK's own click handling, so GTK's Shift+arrow origin
/// stays put. Only `list.select-item` moves it, and it replaces the selection, so the bits
/// are put back.
pub(super) fn move_range_origin(
    row: &gtk::Widget,
    selection: &gtk::MultiSelection,
    syncing: &Cell<bool>,
    position: u32,
) {
    let selected = selection.selection().copy();
    syncing.set(true);
    if let Err(error) = row.activate_action(
        "list.select-item",
        Some(&(position, false, false).to_variant()),
    ) {
        tracing::warn!(%error, "range origin action unavailable");
    }
    selection.set_selection(&selected, &gtk::Bitset::new_range(0, selection.n_items()));
    syncing.set(false);
}

fn anchor_at(browser: &Browser, depth: usize, positions: &PanePositions, view_position: u32) {
    if let Some(source_position) = positions.source_position(view_position) {
        browser.set_selection_anchor(depth, source_position);
    }
}
