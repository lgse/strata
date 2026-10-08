// SPDX-License-Identifier: MIT

use super::{
    BoundRow, PendingActivationKind, PendingPointerActivation, column_size_text,
    set_active_path_style, set_mark_path_style, should_activate_single_click,
    should_preview_pointer_press,
};
use crate::ui::{
    browser::{
        ViewState,
        clipboard::{
            ClipboardMark, PreparedFileDrop, clipboard_mark, drag_actions_for_modifiers,
            drag_preview_icon, file_drag_content, file_drag_hover_target, file_drop_action,
            file_drop_commit, locations_from_file_list_value, prepare_file_drop_target,
        },
        collection::{ViewMap, activate_recursive_search_result},
        entry::{
            entry_icon, entry_responds_to_preview_click, metadata_needs_fill, model_display_name,
        },
        paths::is_trash_location,
    },
    browser_modes::BrowserMode,
    modal::slide_in_down,
};
use crate::{
    model::{FileEntry, Location},
    services::SearchItem,
    ui::browser::arm_spring_load_navigation,
};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

fn anchored_row(state: &std::rc::Weak<ViewState>, depth: usize, map: &ViewMap) -> Option<u32> {
    let state = state.upgrade()?;
    let anchor = state.browser.selection_anchor_position(depth)?;
    map.view_position(anchor)
}

fn anchor_at(state: &std::rc::Weak<ViewState>, depth: usize, map: &ViewMap, row: u32) {
    if let (Some(state), Some(source_position)) = (state.upgrade(), map.source_position(row)) {
        state.browser.set_selection_anchor(depth, source_position);
    }
}

pub(super) struct ColumnRows {
    pub(super) factory: gtk::SignalListItemFactory,
    pub(super) bound_rows: Rc<RefCell<Vec<BoundRow>>>,
    pub(super) scrolling: Rc<Cell<bool>>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "rows read the column's directory map, selection, and search hits it owns"
)]
pub(super) fn column_rows(
    state: &Rc<ViewState>,
    depth: usize,
    map: &ViewMap,
    selection: &gtk::MultiSelection,
    modified_selection: &Rc<Cell<bool>>,
    recursive_search_active: &Rc<Cell<bool>>,
    search_results: &Rc<RefCell<Vec<SearchItem>>>,
    hits: &Rc<RefCell<super::ColumnHits>>,
) -> ColumnRows {
    let scrolling = Rc::new(Cell::new(false));
    let factory = gtk::SignalListItemFactory::new();
    let bound_rows: Rc<RefCell<Vec<BoundRow>>> = Rc::new(RefCell::new(Vec::new()));
    let rows_for_setup = bound_rows.clone();
    let search_active_for_factory = recursive_search_active.clone();
    let search_results_for_factory = search_results.clone();
    let weak_state = Rc::downgrade(state);
    let modified_selection_for_rows = modified_selection.clone();
    let selection_for_rows = selection.clone();
    let map_for_hover = map.clone();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("file-row");
        let icon = crate::ui::thumbnail::ThumbnailSlot::new(17);
        icon.add_css_class("file-icon");
        icon.set_valign(gtk::Align::Center);
        let label = gtk::Label::builder()
            .halign(gtk::Align::Fill)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        let rename = gtk::Entry::new();
        rename.add_css_class("inline-rename");
        crate::ui::accessibility::set_label(&rename, "Rename");
        rename.set_hexpand(true);
        rename.set_width_chars(1);
        rename.set_visible(false);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.add_css_class("file-row-spacer");
        let editor = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        editor.append(&label);
        editor.append(&rename);
        editor.append(&spacer);
        let size = gtk::Label::new(None);
        size.add_css_class("file-size");
        size.set_halign(gtk::Align::End);
        size.set_valign(gtk::Align::Fill);
        size.set_xalign(1.0);
        size.set_yalign(0.5);
        let middle = gtk::Overlay::new();
        middle.add_css_class("file-row-content");
        middle.set_hexpand(true);
        middle.set_valign(gtk::Align::Fill);
        let path = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .visible(false)
            .build();
        path.add_css_class("file-search-path");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.set_valign(gtk::Align::Center);
        content.append(&editor);
        content.append(&path);
        middle.set_child(Some(&content));
        middle.add_overlay(&size);
        let chevron = crate::assets::primary_icon(crate::assets::icons::CHEVRON_RIGHT, 15);
        chevron.add_css_class("file-chevron");
        chevron.set_valign(gtk::Align::Center);
        row.append(&icon);
        row.append(&middle);
        row.append(&chevron);
        item.set_child(Some(&row));
        let pending_activation = Rc::new(RefCell::new(None::<PendingPointerActivation>));
        let pending_activation_for_unmap = pending_activation.clone();
        row.connect_unmap(move |_| {
            pending_activation_for_unmap.take();
        });
        let was_selected = Rc::new(Cell::new(false));
        let mut content_drag: Option<gtk::DragSource> = None;
        if weak_state.upgrade().is_some_and(|state| state.interactive) {
            let drag = gtk::DragSource::builder()
                .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
                .build();
            drag.set_propagation_phase(gtk::PropagationPhase::Capture);
            crate::ui::browser::track_file_drag(&drag);
            let weak_state_for_drag = weak_state.clone();
            let dragged_item = item.downgrade();
            let map_for_drag = map_for_hover.clone();
            let prepare_row = row.downgrade();
            let search_active_for_drag = search_active_for_factory.clone();
            let search_results_for_drag = search_results_for_factory.clone();
            let selection_for_drag = selection_for_rows.clone();
            let was_selected_for_drag = was_selected.clone();
            drag.connect_prepare(move |source, x, y| {
                let prepare_row = prepare_row.upgrade()?;
                if prepare_row
                    .pick(x, y, gtk::PickFlags::DEFAULT)
                    .is_some_and(|target| crate::ui::focus_navigation::editable(&target))
                    || (!was_selected_for_drag.get()
                        && !crate::ui::pointer::hits_item_content(prepare_row.upcast_ref(), x, y))
                {
                    return None;
                }
                source.set_actions(drag_actions_for_modifiers(source.current_event_state()));
                let state = weak_state_for_drag.upgrade()?;
                let dragged_item = dragged_item.upgrade()?;
                let position = dragged_item.position();
                let (entry, selected) = if search_active_for_drag.get() {
                    let results = search_results_for_drag.borrow();
                    let entry =
                        crate::ui::browser::search_result_entry(results.get(position as usize)?);
                    let selected = crate::ui::browser::collection::bitset_positions(
                        &selection_for_drag.selection(),
                    )
                    .into_iter()
                    .filter_map(|position| results.get(position as usize))
                    .map(crate::ui::browser::search_result_entry)
                    .collect();
                    (entry, selected)
                } else {
                    let source_position = map_for_drag.source_position(position)?;
                    (
                        state.browser.entry_at(depth, source_position)?,
                        state.browser.selected_entries(),
                    )
                };
                let entries = if selected
                    .iter()
                    .any(|selected| selected.location == entry.location)
                {
                    selected
                } else {
                    vec![entry]
                };
                if let Some((texture, hot_x, hot_y)) = drag_preview_icon(&prepare_row, &entries) {
                    source.set_icon(Some(&texture), hot_x, hot_y);
                }
                file_drag_content(&entries)
            });
            let dragged_row = row.downgrade();
            let weak_state_for_begin = weak_state.clone();
            let pending_activation_for_drag = pending_activation.clone();
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
            let weak_state_for_end = weak_state.clone();
            drag.connect_drag_end(move |_, _, _| {
                if let Some(row) = dragged_row.upgrade() {
                    row.remove_css_class("dragging");
                }
                if let Some(state) = weak_state_for_end.upgrade() {
                    state.cancel_peek();
                    state.stop_drag_autoscroll();
                }
            });
            row.add_controller(drag.clone());
            content_drag = Some(drag);

            let dest_for_row = {
                let weak_state = weak_state.clone();
                let item = item.downgrade();
                let map = map_for_hover.clone();
                move || {
                    let state = weak_state.upgrade()?;
                    let item = item.upgrade()?;
                    map.source_position(item.position())
                        .and_then(|position| state.browser.entry_at(depth, position))
                        .filter(FileEntry::is_directory)
                        .map(|entry| entry.location)
                }
            };
            let PreparedFileDrop {
                target: drop,
                state: drop_state,
            } = prepare_file_drop_target(dest_for_row);
            let spring_navigate: Rc<dyn Fn(Location)> = {
                let weak_state = weak_state.clone();
                Rc::new(move |location| {
                    if let Some(state) = weak_state.upgrade() {
                        state.browser.descend(depth, location);
                    }
                })
            };
            let highlighted_row = row.downgrade();
            let state_for_enter = drop_state.clone();
            let navigate_for_enter = spring_navigate.clone();
            drop.connect_enter(move |target, _, _| {
                let action = file_drop_action(target, &state_for_enter);
                let hovered = file_drag_hover_target(&state_for_enter, target).is_some();
                if let Some(row) = highlighted_row.upgrade() {
                    if hovered {
                        row.add_css_class("drop-destination");
                    } else {
                        row.remove_css_class("drop-destination");
                    }
                }
                arm_spring_load_navigation(&state_for_enter, target, &navigate_for_enter);
                action
            });
            let highlighted_row = row.downgrade();
            let state_for_motion = drop_state.clone();
            let navigate_for_motion = spring_navigate.clone();
            drop.connect_motion(move |target, _, _| {
                let action = file_drop_action(target, &state_for_motion);
                let hovered = file_drag_hover_target(&state_for_motion, target).is_some();
                if let Some(row) = highlighted_row.upgrade() {
                    if hovered {
                        row.add_css_class("drop-destination");
                    } else {
                        row.remove_css_class("drop-destination");
                    }
                }
                arm_spring_load_navigation(&state_for_motion, target, &navigate_for_motion);
                action
            });
            let highlighted_row = row.downgrade();
            let state_for_value = drop_state.clone();
            let navigate_for_value = spring_navigate.clone();
            drop.connect_value_notify(move |target| {
                if target.current_drop().is_none() {
                    return;
                }
                file_drop_action(target, &state_for_value);
                let hovered = file_drag_hover_target(&state_for_value, target).is_some();
                if let Some(row) = highlighted_row.upgrade() {
                    if hovered {
                        row.add_css_class("drop-destination");
                    } else {
                        row.remove_css_class("drop-destination");
                    }
                }
                arm_spring_load_navigation(&state_for_value, target, &navigate_for_value);
            });
            let highlighted_row = row.downgrade();
            let state_for_leave = drop_state.clone();
            drop.connect_leave(move |_| {
                state_for_leave.cancel_spring_load_navigation();
                if let Some(row) = highlighted_row.upgrade() {
                    row.remove_css_class("drop-destination");
                }
            });
            let weak_state_for_accept = weak_state.clone();
            let accepted_item = item.downgrade();
            let map_for_accept = map_for_hover.clone();
            drop.connect_accept(move |_, offered| {
                let Some(state) = weak_state_for_accept.upgrade() else {
                    return false;
                };
                let Some(accepted_item) = accepted_item.upgrade() else {
                    return false;
                };
                let entry = map_for_accept
                    .source_position(accepted_item.position())
                    .and_then(|position| state.browser.entry_at(depth, position));
                entry.is_some_and(|entry| {
                    entry.is_directory()
                        && !is_trash_location(&entry.location)
                        && offered
                            .formats()
                            .contains_type(gtk::gdk::FileList::static_type())
                })
            });
            let weak_state_for_drop = weak_state.clone();
            let dropped_row = row.downgrade();
            drop.connect_drop(move |target, value, _, _| {
                let Some(dropped_row) = dropped_row.upgrade() else {
                    return false;
                };
                drop_state.cancel_spring_load_navigation();
                dropped_row.remove_css_class("drop-destination");
                let Some(state) = weak_state_for_drop.upgrade() else {
                    return false;
                };
                state.stop_drag_autoscroll();
                let Some(destination) = drop_state.destination() else {
                    return false;
                };
                let Some(sources) = locations_from_file_list_value(value) else {
                    return false;
                };
                let commit = file_drop_commit(target, &destination, &sources, &drop_state);
                slide_in_down(&dropped_row);
                glib::timeout_add_local_once(Duration::from_millis(300), move || {
                    state.commit_file_drop(destination, sources, commit);
                });
                true
            });
            row.add_controller(drop);
        }

        let selection_click = gtk::GestureClick::new();
        let weak_state_for_click = weak_state.clone();
        selection_click.set_button(1);
        selection_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let clicked_item = item.downgrade();
        let selection_for_click = selection_for_rows.clone();
        let modified_for_click = modified_selection_for_rows.clone();
        let map_for_click = map_for_hover.clone();
        let search_active_for_click = search_active_for_factory.clone();
        let search_results_for_click = search_results_for_factory.clone();
        // Open on release so a press-and-move can start a drag first.
        let pending_activation_for_press = pending_activation.clone();
        let pending_activation_for_motion = pending_activation.clone();
        let pending_activation_for_release = pending_activation.clone();
        let pending_activation_for_cancel = pending_activation;
        let was_selected_for_press = was_selected.clone();
        let was_selected_for_release = was_selected.clone();
        let sequence = crate::ui::collection_interaction::PointerSequence::default();
        sequence.install(&selection_click);
        let press_moved = Rc::new(Cell::new(false));
        let press_moved_for_press = press_moved.clone();
        let press_moved_for_update = press_moved.clone();
        let press_moved_for_release = press_moved.clone();
        let press_origin = Rc::new(Cell::new((0.0, 0.0)));
        let press_origin_for_press = press_origin.clone();
        let press_origin_for_update = press_origin.clone();
        let rename_position = Rc::new(Cell::new(None::<usize>));
        let rename_position_for_press = rename_position.clone();
        let rename_position_for_release = rename_position.clone();
        let name_label_for_press = label.downgrade();
        selection_click.connect_pressed(move |gesture, press_count, x, y| {
            pending_activation_for_press.take();
            rename_position_for_press.set(None);
            was_selected_for_press.set(false);
            press_moved_for_press.set(false);
            // Focusing a parent can scroll the row beneath a stationary pointer.
            let press = gesture
                .current_event()
                .and_then(|event| event.position())
                .unwrap_or((x, y));
            press_origin_for_press.set(press);
            if let Some(state) = weak_state_for_click.upgrade() {
                state.cancel_click_rename();
            }
            if gesture
                .widget()
                .and_then(|row| row.pick(x, y, gtk::PickFlags::DEFAULT))
                .is_some_and(|target| crate::ui::focus_navigation::editable(&target))
            {
                return;
            }
            let Some(clicked_item) = clicked_item.upgrade() else {
                return;
            };
            let position = clicked_item.position();
            if position == gtk::INVALID_LIST_POSITION {
                return;
            }
            let modifiers = gesture.current_event_state();
            let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            sequence.press(modifiers);
            let selected_before = selection_for_click.is_selected(position);
            let selected_count_before = selection_for_click.selection().size();
            was_selected_for_press.set(selected_before);
            let change = crate::ui::collection_interaction::pointer_selection(
                &selection_for_click.selection(),
                position,
                anchored_row(&weak_state_for_click, depth, &map_for_click),
                weak_state_for_click
                    .upgrade()
                    .is_none_or(|state| state.multiple_selection.get()),
                modifiers,
            );
            let preserve_group = change.preserved_group;
            let filtered = search_active_for_click.get() || map_for_click.has_query();
            let source_position = map_for_click.source_position(position);
            let (activate, location_hold) = if let Some(state) = weak_state_for_click.upgrade()
                && let Some(entry) =
                    source_position.and_then(|position| state.browser.entry_at(depth, position))
            {
                let activate = !filtered
                    && should_activate_single_click(
                        press_count,
                        entry.is_directory(),
                        state.columns_click_activation.get(),
                        control,
                        shift,
                        preserve_group,
                    )
                    && !state.browser.is_open_child(depth, &entry.location);
                let location_hold = (!search_active_for_click.get()
                    && entry.is_directory()
                    && (activate || (filtered && press_count == 1 && !control && !shift)))
                    .then(|| state.hold_tab_location());
                (activate, location_hold)
            } else {
                (false, None)
            };
            modified_for_click.set(control || shift);
            if !shift && let Some(anchor) = change.anchor {
                anchor_at(&weak_state_for_click, depth, &map_for_click, anchor);
            }
            // Own this write for the whole synchronous selection-changed emission.
            // A click on the cursor row is otherwise identical to a focus echo.
            let pointer_owner = weak_state_for_click
                .upgrade()
                .inspect(|state| state.pointer_owns_selection.set(true));
            selection_for_click.set_selection(
                &change.selected,
                &gtk::Bitset::new_range(0, selection_for_click.n_items()),
            );
            if let Some(state) = pointer_owner {
                state.pointer_owns_selection.set(false);
            }
            if (control || shift)
                && let Some(widget) = gesture.widget()
                && crate::ui::pointer::hits_item_content(&widget, x, y)
                && let Some(item_widget) = widget.parent()
            {
                item_widget.grab_focus();
            }
            modified_for_click.set(false);

            if filtered {
                if press_count == 1
                    && !control
                    && !shift
                    && let Some(state) = weak_state_for_click.upgrade()
                {
                    let pending = if search_active_for_click.get() {
                        search_results_for_click
                            .borrow()
                            .get(position as usize)
                            .map(|item| {
                                let kind = if state.browser.is_chooser_mode() {
                                    PendingActivationKind::ChooserSearchNavigate
                                } else {
                                    PendingActivationKind::RecursiveSearch
                                };
                                (
                                    position as usize,
                                    crate::model::Location::local(item.path.clone()),
                                    kind,
                                )
                            })
                    } else {
                        map_for_click
                            .source_position(position)
                            .and_then(|source_position| {
                                let entry = state.browser.entry_at(depth, source_position)?;
                                state.browser.select(depth, source_position);
                                if is_trash_location(&entry.location) && !entry.is_directory() {
                                    if state.single_click_previews.get() {
                                        return Some((
                                            source_position,
                                            entry.location,
                                            PendingActivationKind::Standard { preview: true },
                                        ));
                                    }
                                    return None;
                                }
                                Some((
                                    source_position,
                                    entry.location,
                                    PendingActivationKind::Mapped,
                                ))
                            })
                    };
                    if let Some((position, location, kind)) = pending {
                        pending_activation_for_press.replace(Some(PendingPointerActivation {
                            position,
                            location,
                            press,
                            moved: false,
                            kind,
                            location_hold,
                        }));
                    }
                }
                return;
            }

            if let (Some(state), Some(source_position)) =
                (weak_state_for_click.upgrade(), source_position)
            {
                let entry = state.browser.entry_at(depth, source_position);
                if let Some(entry) = entry.as_ref() {
                    let slow_click_rename = press_count == 1
                        && selected_before
                        && selected_count_before == 1
                        && !modifiers.intersects(
                            gtk::gdk::ModifierType::CONTROL_MASK
                                | gtk::gdk::ModifierType::SHIFT_MASK
                                | gtk::gdk::ModifierType::ALT_MASK
                                | gtk::gdk::ModifierType::SUPER_MASK
                                | gtk::gdk::ModifierType::META_MASK,
                        )
                        && !preserve_group
                        && !activate
                        && !state.browser.is_chooser_mode()
                        && !is_trash_location(&entry.location)
                        && gesture
                            .widget()
                            .zip(name_label_for_press.upgrade())
                            .is_some_and(|(row, label)| {
                                crate::ui::pointer::hits_name_label(&row, label.upcast_ref(), x, y)
                            });
                    rename_position_for_press.set(if slow_click_rename {
                        Some(source_position)
                    } else {
                        None
                    });
                    let preview = !activate
                        && !slow_click_rename
                        && should_preview_pointer_press(
                            press_count,
                            control,
                            shift,
                            preserve_group,
                        )
                        && entry_responds_to_preview_click(
                            entry,
                            state.single_click_previews.get(),
                        );
                    if activate || preview {
                        pending_activation_for_press.replace(Some(PendingPointerActivation {
                            position: source_position,
                            location: entry.location.clone(),
                            press,
                            moved: false,
                            kind: PendingActivationKind::Standard { preview },
                            location_hold,
                        }));
                    }
                }
            }
        });
        selection_click.connect_update(move |gesture, _| {
            let Some((x, y)) = gesture.current_event().and_then(|event| event.position()) else {
                return;
            };
            let Some(widget) = gesture.widget() else {
                return;
            };
            let threshold = widget.settings().gtk_dnd_drag_threshold();
            let cancelled_hold = pending_activation_for_motion
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
                press_origin_for_update.get(),
                (x, y),
                threshold,
            ) {
                press_moved_for_update.set(true);
            }
        });
        let weak_state_for_release = weak_state.clone();
        let search_results_for_release = search_results_for_factory.clone();
        selection_click.connect_released(move |gesture, count, x, y| {
            let pending = pending_activation_for_release.take();
            let (x, y) = gesture
                .current_event()
                .and_then(|event| event.position())
                .unwrap_or((x, y));
            if pending.is_none()
                && count == 1
                && !press_moved_for_release.get()
                && !gesture.current_event_state().intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
                )
                && was_selected_for_release.get()
                && let Some(state) = weak_state_for_release.upgrade()
                && let Some(position) = rename_position_for_release.get()
            {
                state.schedule_click_rename(depth, position);
            }
            let Some(mut pending) = pending else {
                return;
            };
            let Some(widget) = gesture.widget() else {
                return;
            };
            pending.update(x, y, widget.settings().gtk_dnd_drag_threshold());
            let Some(state) = weak_state_for_release.upgrade() else {
                return;
            };
            // GTK 4.14's DragSource needs the release to reset before the next press.
            match pending.kind {
                PendingActivationKind::Standard { preview } => {
                    if !state
                        .browser
                        .entry_at(depth, pending.position)
                        .is_some_and(|entry| pending.can_activate(&entry.location))
                    {
                        return;
                    }
                    if preview {
                        if state.single_click_previews.get() {
                            state.browser.preview(depth, pending.position);
                        }
                    } else {
                        let activate = || state.browser.activate(depth, pending.position);
                        if let Some(hold) = pending.location_hold.take() {
                            hold.navigate(depth, activate);
                        } else {
                            activate();
                        }
                    }
                }
                PendingActivationKind::Mapped => {
                    if !state
                        .browser
                        .entry_at(depth, pending.position)
                        .is_some_and(|entry| pending.can_activate(&entry.location))
                    {
                        return;
                    }
                    if !state.browser.is_chooser_mode() {
                        let activate = || state.browser.activate_in_place(depth, pending.position);
                        if let Some(hold) = pending.location_hold.take() {
                            hold.navigate(depth, activate);
                        } else {
                            activate();
                        }
                    }
                }
                PendingActivationKind::ChooserSearchNavigate => {
                    let Some(item) = search_results_for_release
                        .borrow()
                        .get(pending.position)
                        .cloned()
                    else {
                        return;
                    };
                    let location = crate::model::Location::local(item.path.clone());
                    if item.is_directory && pending.can_activate(&location) {
                        state.browser.navigate(location);
                    }
                }
                PendingActivationKind::RecursiveSearch => {
                    let matches = search_results_for_release
                        .borrow()
                        .get(pending.position)
                        .is_some_and(|item| {
                            pending.can_activate(&crate::model::Location::local(item.path.clone()))
                        });
                    if matches {
                        activate_recursive_search_result(
                            &Rc::downgrade(&state.browser),
                            &search_results_for_release,
                            pending.position as u32,
                        );
                    }
                }
            }
        });
        let weak_state_for_cancel = weak_state.clone();
        selection_click.connect_cancel(move |_, _| {
            pending_activation_for_cancel.take();
            if let Some(state) = weak_state_for_cancel.upgrade() {
                state.cancel_click_rename();
            }
        });
        row.add_controller(selection_click.clone());
        if let Some(drag) = &content_drag {
            drag.group_with(&selection_click);
        }
        let weak_item = glib::WeakRef::new();
        weak_item.set(Some(item));
        let weak_row = glib::WeakRef::new();
        weak_row.set(Some(&row));
        let weak_rename_label = glib::WeakRef::new();
        weak_rename_label.set(Some(&label));
        rows_for_setup.borrow_mut().push(BoundRow {
            item: weak_item,
            row: weak_row,
            rename_label: weak_rename_label,
            edit: crate::ui::collection_edit::EditWidgets::new(&rename, &label),
            spacer,
            size,
        });
    });
    let map_for_bind = map.clone();
    let weak_state_for_bind = Rc::downgrade(state);
    let search_active_for_bind = recursive_search_active.clone();
    let search_results_for_bind = search_results.clone();
    let hits_for_bind = hits.clone();
    let rows_for_bind = bound_rows.clone();
    let scrolling_for_bind = scrolling.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(value) = item.item().and_downcast::<gtk::StringObject>() else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon) = row
            .first_child()
            .and_downcast::<crate::ui::thumbnail::ThumbnailSlot>()
        else {
            return;
        };
        let Some(middle) = icon.next_sibling().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let Some(content) = middle.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(editor) = content.first_child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(path) = content.last_child().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(label) = editor.first_child().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(rename) = label.next_sibling().and_downcast::<gtk::Entry>() else {
            return;
        };
        let Some(spacer) = rename.next_sibling().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(size) = middle.last_child().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(chevron) = middle.next_sibling().and_downcast::<gtk::Image>() else {
            return;
        };
        row.remove_css_class("keyboard-cursor");
        if let Some(state) = weak_state_for_bind.upgrade()
            && state.focused_column_depth() == Some(depth)
            && state
                .browser
                .focused_item()
                .is_some_and(|(focused_depth, position, _)| {
                    focused_depth == depth
                        && map_for_bind.view_position(position) == Some(item.position())
                })
        {
            row.add_css_class("keyboard-cursor");
        }
        label.set_label(model_display_name(&value.string()));
        let searching = search_active_for_bind.get();
        if searching {
            row.add_css_class("filter-result");
        } else {
            row.remove_css_class("filter-result");
        }
        let source_position = (!searching)
            .then(|| map_for_bind.source_position(item.position()))
            .flatten();
        let state = weak_state_for_bind.upgrade();
        let browser = state.as_ref().map(|state| &state.browser);
        let entry = if searching {
            search_results_for_bind
                .borrow()
                .get(item.position() as usize)
                .map(crate::ui::browser::search_result_entry)
        } else {
            source_position.and_then(|position| browser?.entry_at(depth, position))
        };
        let edit = rows_for_bind
            .borrow()
            .iter()
            .find(|bound| bound.item.upgrade().as_ref() == Some(item))
            .map(|bound| bound.edit.clone());
        if let Some(edit) = &edit {
            if let Some(entry) = &entry {
                edit.bind(&entry.location);
            } else {
                edit.unbind();
            }
        }
        let editing = edit.as_ref().is_some_and(|edit| edit.is_editing());
        rename.set_visible(editing);
        label.set_visible(!editing);
        spacer.set_visible(!editing);
        if let Some(entry) = entry.as_ref() {
            let pending_name = state
                .as_ref()
                .and_then(|state| state.pending_rename_name(entry));
            label.set_label(pending_name.as_deref().unwrap_or(&entry.display_name));
            label.set_opacity(if entry.is_hidden { 0.65 } else { 1.0 });
        } else {
            label.set_opacity(1.0);
        }
        let hits = hits_for_bind.borrow();
        let find = state.as_ref().and_then(|state| state.find_highlight());
        if searching {
            let hit = search_results_for_bind
                .borrow()
                .get(item.position() as usize)
                .and_then(|hit| hits.ranges.get(&hit.path).cloned());
            crate::ui::browser::find::highlight_hit_name(
                label.upcast_ref(),
                find.as_deref(),
                hit.as_deref(),
            );
        } else {
            crate::ui::browser::find::highlight_listing_name(
                label.upcast_ref(),
                find.as_deref(),
                &map_for_bind.query(),
            );
        }
        let origin = entry
            .as_ref()
            .filter(|_| searching)
            .map(|entry| entry.location.display_path());
        path.set_label(origin.as_deref().unwrap_or_default());
        path.set_visible(origin.is_some() && hits.recursive);
        row.set_widget_name(origin.as_deref().unwrap_or("file-row"));
        crate::ui::accessibility::set_description(&row, origin.as_deref());
        let active = entry.as_ref().is_some_and(|entry| {
            browser
                .as_ref()
                .is_some_and(|browser| browser.is_open_child(depth, &entry.location))
        });
        let immediate =
            browser.as_ref().and_then(|browser| browser.active_depth()) == Some(depth + 1);
        set_active_path_style(&row, active, immediate);
        set_mark_path_style(
            &row,
            entry
                .as_ref()
                .map_or(ClipboardMark::None, |entry| clipboard_mark(&entry.location)),
        );
        crate::ui::file_providers::bind(
            &icon,
            entry.as_ref().and_then(|e| e.location.native_path()),
        );
        if let Some(entry) = entry.as_ref() {
            let mode_active = state
                .as_ref()
                .is_some_and(|state| state.mode_views.borrow().mode() == BrowserMode::Columns);
            if searching && mode_active && !entry.is_directory() {
                // Search results have no directory metadata-fill producer.
                crate::ui::thumbnail::set_thumbnail_or_icon_for_path(
                    &icon,
                    entry.local_thumbnail_path().expect("local search result"),
                    entry_icon(entry),
                    17,
                    17,
                );
            } else if entry.is_directory() || mode_active {
                crate::ui::thumbnail::set_thumbnail_or_icon(
                    &icon,
                    entry,
                    entry_icon(entry),
                    17,
                    17,
                );
            } else {
                crate::ui::thumbnail::show_fallback_icon(&icon, entry_icon(entry), 17);
            }
            icon.set_hidden(entry.is_hidden);
            icon.set_base_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
            chevron.set_visible(entry.is_directory());
            if mode_active
                && !scrolling_for_bind.get()
                && let Some(state) = state.as_ref()
                && let Some(position) = source_position
                && metadata_needs_fill(entry)
            {
                crate::ui::thumbnail::request_metadata(
                    &icon,
                    &row,
                    &state.browser,
                    depth,
                    position,
                    entry.location.clone(),
                    false,
                );
            }
        } else {
            crate::ui::thumbnail::show_fallback_icon(&icon, crate::assets::icons::DOCUMENTS, 17);
            icon.set_hidden(false);
            icon.set_mark(ClipboardMark::None);
            icon.set_base_opacity(0.72);
            chevron.set_visible(false);
        }
        let size_text = column_size_text(entry.as_ref());
        size.set_label(&size_text);
        size.set_visible(!editing && !size_text.is_empty());
        crate::ui::accessibility::describe_entry(item, &label.label(), entry.as_ref());
    });
    let rows_for_unbind = bound_rows.clone();
    factory.connect_unbind(move |_, object| {
        let edit = rows_for_unbind
            .borrow()
            .iter()
            .find(|bound| {
                bound
                    .item
                    .upgrade()
                    .as_ref()
                    .map(|item| item.upcast_ref::<glib::Object>())
                    == Some(object)
            })
            .map(|bound| bound.edit.clone());
        if let Some(edit) = edit {
            edit.unbind();
        }
        crate::ui::thumbnail::cancel_list_item_thumbnails(object);
    });
    let rows_for_teardown = bound_rows.clone();
    factory.connect_teardown(move |_, object| {
        rows_for_teardown.borrow_mut().retain(|bound| {
            bound
                .item
                .upgrade()
                .is_some_and(|item| item.upcast_ref::<glib::Object>() != object)
        });
    });
    ColumnRows {
        factory,
        bound_rows,
        scrolling,
    }
}
