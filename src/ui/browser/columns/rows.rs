// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    BoundRow, PendingPointerActivation, column_size_text, set_active_path_style,
    set_cut_path_style, should_activate_single_click, should_preserve_drag_selection,
    should_preview_pointer_press,
};
use crate::ui::{
    browser::{
        ViewState,
        clipboard::{
            file_drag_content, file_drop_action, locations_equal, locations_from_file_list_value,
            shared_cut_locations,
        },
        collection::{ViewMap, cancel_source},
        entry::{
            entry_icon, entry_responds_to_preview_click, metadata_needs_fill, model_display_name,
        },
        inline_edit::update_basename_validation,
    },
    browser_modes::BrowserMode,
    modal::{slide_in_down, slide_out},
};
use crate::{
    model::{EntryKind, FileEntry, Location},
    services::SearchItem,
};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub(super) struct ColumnRows {
    pub(super) factory: gtk::SignalListItemFactory,
    pub(super) bound_rows: Rc<RefCell<Vec<BoundRow>>>,
}

pub(super) fn column_rows(
    state: &Rc<ViewState>,
    depth: usize,
    map: &ViewMap,
    selection: &gtk::MultiSelection,
    modified_selection: &Rc<Cell<bool>>,
    recursive_search_active: &Rc<Cell<bool>>,
    search_results: &Rc<RefCell<Vec<SearchItem>>>,
) -> ColumnRows {
    let factory = gtk::SignalListItemFactory::new();
    let bound_rows: Rc<RefCell<Vec<BoundRow>>> = Rc::new(RefCell::new(Vec::new()));
    let rows_for_setup = bound_rows.clone();
    let weak_state = Rc::downgrade(state);
    let modified_selection_for_rows = modified_selection.clone();
    let selection_for_rows = selection.clone();
    let mouse_selection_anchor = Rc::new(Cell::new(None::<u32>));
    let map_for_hover = map.clone();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("file-row");
        row.add_css_class("file-appear");
        let weak_row = row.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(row) = weak_row.upgrade() {
                row.remove_css_class("file-appear");
            }
        });
        let icon = gtk::Image::new();
        icon.add_css_class("file-icon");
        icon.set_pixel_size(17);
        let label = gtk::Label::builder()
            .halign(gtk::Align::Fill)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let rename = gtk::Entry::new();
        rename.add_css_class("inline-rename");
        rename.set_hexpand(true);
        rename.set_visible(false);
        rename.connect_changed(|field| {
            update_basename_validation(field);
        });
        let weak_state_for_rename = weak_state.clone();
        rename.connect_activate(move |field| {
            if let Some(state) = weak_state_for_rename.upgrade() {
                state.submit_rename(field);
            }
        });
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.add_css_class("file-row-spacer");
        let editor = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        editor.append(&label);
        editor.append(&rename);
        editor.append(&spacer);
        let size = gtk::Label::new(None);
        size.add_css_class("file-size");
        size.set_halign(gtk::Align::End);
        size.set_valign(gtk::Align::Center);
        size.set_xalign(1.0);
        let middle = gtk::Overlay::new();
        middle.set_hexpand(true);
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
        content.append(&editor);
        content.append(&path);
        middle.set_child(Some(&content));
        middle.add_overlay(&size);
        let chevron = crate::assets::primary_icon(crate::assets::icons::CHEVRON_RIGHT, 15);
        chevron.add_css_class("file-chevron");
        row.append(&icon);
        row.append(&middle);
        row.append(&chevron);
        let motion = gtk::EventControllerMotion::new();
        let list_item = item.downgrade();
        let weak_state_for_enter = weak_state.clone();
        let map_for_enter = map_for_hover.clone();
        motion.connect_enter(move |controller, _, _| {
            let Some(item) = list_item.upgrade() else {
                return;
            };
            if let Some(state) = weak_state_for_enter.upgrade() {
                let source_position = map_for_enter.source_position(item.position());
                let entry =
                    source_position.and_then(|position| state.browser.entry_at(depth, position));
                if let Some(entry) = entry {
                    if entry.is_directory() {
                        if let Some(anchor) = controller.widget() {
                            state.schedule_peek(depth, entry.location, anchor);
                        }
                    } else {
                        cancel_source(&state.pending_peek);
                        state.browser.close_peek();
                    }
                }
            }
        });
        let weak_state_for_leave = weak_state.clone();
        motion.connect_leave(move |_| {
            if let Some(state) = weak_state_for_leave.upgrade() {
                state.schedule_close_peek();
            }
        });
        row.add_controller(motion);

        if weak_state.upgrade().is_some_and(|state| state.interactive) {
            let drag = gtk::DragSource::builder()
                .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
                .build();
            let weak_state_for_drag = weak_state.clone();
            let dragged_item = item.downgrade();
            let map_for_drag = map_for_hover.clone();
            let prepare_row = row.downgrade();
            drag.connect_prepare(move |source, x, y| {
                let prepare_row = prepare_row.upgrade()?;
                prepare_row.remove_css_class("slide-out");
                let state = weak_state_for_drag.upgrade()?;
                let dragged_item = dragged_item.upgrade()?;
                let source_position = map_for_drag.source_position(dragged_item.position())?;
                let entry = state.browser.entry_at(depth, source_position)?;
                let selected = state.browser.selected_entries();
                let entries = if selected
                    .iter()
                    .any(|selected| selected.location == entry.location)
                {
                    selected
                } else {
                    vec![entry]
                };
                let paintable = gtk::WidgetPaintable::new(source.widget().as_ref());
                source.set_icon(Some(&paintable), x.round() as i32, y.round() as i32);
                file_drag_content(&entries)
            });
            let dragged_row = row.downgrade();
            drag.connect_drag_begin(move |_, _| {
                if let Some(row) = dragged_row.upgrade() {
                    row.add_css_class("dragging");
                }
            });
            let dragged_row = row.downgrade();
            drag.connect_drag_end(move |_, _, _| {
                if let Some(row) = dragged_row.upgrade() {
                    row.remove_css_class("dragging");
                    slide_out(&row);
                }
            });
            row.add_controller(drag);

            let drop = gtk::DropTarget::new(
                gtk::gdk::FileList::static_type(),
                gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
            );
            let highlighted_row = row.downgrade();
            drop.connect_enter(move |target, _, _| {
                if let Some(row) = highlighted_row.upgrade() {
                    row.add_css_class("drop-destination");
                }
                file_drop_action(target)
            });
            let highlighted_row = row.downgrade();
            drop.connect_motion(move |target, _, _| {
                if let Some(row) = highlighted_row.upgrade() {
                    row.add_css_class("drop-destination");
                }
                file_drop_action(target)
            });
            let highlighted_row = row.downgrade();
            drop.connect_leave(move |_| {
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
                        && offered
                            .formats()
                            .contains_type(gtk::gdk::FileList::static_type())
                })
            });
            let weak_state_for_drop = weak_state.clone();
            let dropped_item = item.downgrade();
            let map_for_drop = map_for_hover.clone();
            let dropped_row = row.downgrade();
            drop.connect_drop(move |target, value, _, _| {
                let Some(dropped_row) = dropped_row.upgrade() else {
                    return false;
                };
                dropped_row.remove_css_class("drop-destination");
                let Some(state) = weak_state_for_drop.upgrade() else {
                    return false;
                };
                let Some(dropped_item) = dropped_item.upgrade() else {
                    return false;
                };
                let Some(destination) = map_for_drop
                    .source_position(dropped_item.position())
                    .and_then(|position| state.browser.entry_at(depth, position))
                    .filter(FileEntry::is_directory)
                    .map(|entry| entry.location)
                else {
                    return false;
                };
                let Some(sources) = locations_from_file_list_value(value) else {
                    return false;
                };
                let move_sources = file_drop_action(target) == gtk::gdk::DragAction::MOVE;
                slide_in_down(&dropped_row);
                glib::timeout_add_local_once(Duration::from_millis(300), move || {
                    state.start_transfer(destination, sources, move_sources);
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
        let selection_anchor_for_click = mouse_selection_anchor.clone();
        let modified_for_click = modified_selection_for_rows.clone();
        let map_for_click = map_for_hover.clone();
        // Open on release so a press-and-move can start a drag first.
        let pending_activation = Rc::new(RefCell::new(None::<PendingPointerActivation>));
        let pending_activation_for_press = pending_activation.clone();
        let pending_activation_for_motion = pending_activation.clone();
        let pending_activation_for_release = pending_activation.clone();
        let pending_activation_for_cancel = pending_activation;
        selection_click.connect_pressed(move |gesture, press_count, x, y| {
            pending_activation_for_press.take();
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
            let preserve_group = !control
                && !shift
                && should_preserve_drag_selection(
                    selection_for_click.is_selected(position),
                    selection_for_click.selection().size(),
                );
            modified_for_click.set(control || shift);
            if shift {
                let anchor = selection_anchor_for_click.get().unwrap_or(position);
                let start = anchor.min(position);
                let count = anchor.max(position).saturating_sub(start) + 1;
                selection_for_click.select_range(start, count, true);
            } else if control {
                selection_anchor_for_click.set(Some(position));
                if selection_for_click.is_selected(position) {
                    selection_for_click.unselect_item(position);
                } else {
                    selection_for_click.select_item(position, false);
                }
            } else {
                selection_anchor_for_click.set(Some(position));
                if !preserve_group {
                    selection_for_click.select_item(position, true);
                }
            }
            if control || shift {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
            modified_for_click.set(false);

            let source_position = map_for_click.source_position(position);
            if let (Some(state), Some(source_position)) =
                (weak_state_for_click.upgrade(), source_position)
            {
                let entry = state.browser.entry_at(depth, source_position);
                if let Some(entry) = entry.as_ref().filter(|entry| {
                    should_activate_single_click(
                        press_count,
                        entry.is_directory(),
                        state.columns_click_activation.get(),
                        control,
                        shift,
                        preserve_group,
                    )
                }) {
                    pending_activation_for_press.replace(Some(PendingPointerActivation {
                        position: source_position,
                        location: entry.location.clone(),
                        press: (x, y),
                        moved: false,
                    }));
                } else if should_preview_pointer_press(press_count, control, shift, preserve_group)
                    && entry.as_ref().is_some_and(|entry| {
                        entry_responds_to_preview_click(entry, state.single_click_previews.get())
                    })
                {
                    state.browser.preview(depth, source_position);
                }
            }
        });
        selection_click.connect_update(move |gesture, sequence| {
            if let (Some(pending), Some((x, y)), Some(widget)) = (
                pending_activation_for_motion.borrow_mut().as_mut(),
                gesture.point(sequence),
                gesture.widget(),
            ) {
                pending.update(x, y, widget.settings().gtk_dnd_drag_threshold());
            }
        });
        let weak_state_for_release = weak_state.clone();
        selection_click.connect_released(move |gesture, _, _, _| {
            let Some(pending) = pending_activation_for_release.take() else {
                return;
            };
            let Some(state) = weak_state_for_release.upgrade() else {
                return;
            };
            if !state
                .browser
                .entry_at(depth, pending.position)
                .is_some_and(|entry| pending.can_activate(&entry.location))
            {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            state.browser.activate(depth, pending.position);
        });
        selection_click.connect_cancel(move |_, _| {
            pending_activation_for_cancel.take();
        });
        row.add_controller(selection_click);
        item.set_child(Some(&row));
        let weak_item = glib::WeakRef::new();
        weak_item.set(Some(item));
        let weak_row = glib::WeakRef::new();
        weak_row.set(Some(&row));
        rows_for_setup.borrow_mut().push(BoundRow {
            item: weak_item,
            row: weak_row,
        });
    });
    let map_for_bind = map.clone();
    let weak_state_for_bind = Rc::downgrade(state);
    let search_active_for_bind = recursive_search_active.clone();
    let search_results_for_bind = search_results.clone();
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
        let Some(icon) = row.first_child().and_downcast::<gtk::Image>() else {
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
        rename.set_visible(false);
        label.set_visible(true);
        spacer.set_visible(true);
        let searching = search_active_for_bind.get();
        let source_position = (!searching)
            .then(|| map_for_bind.source_position(item.position()))
            .flatten();
        let state = weak_state_for_bind.upgrade();
        let browser = state.as_ref().map(|state| &state.browser);
        let entry = if searching {
            search_results_for_bind
                .borrow()
                .get(item.position() as usize)
                .map(|item| FileEntry {
                    location: Location::local(item.path.clone()),
                    native_name: item.path.file_name().unwrap_or_default().to_os_string(),
                    thumbnail_path: None,
                    display_name: item.name.clone(),
                    kind: if item.is_directory {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    size: crate::model::MetadataValue::Unknown,
                    modified_unix_seconds: crate::model::MetadataValue::Unknown,
                    is_hidden: false,
                    mode: crate::model::MetadataValue::Unknown,
                })
        } else {
            source_position.and_then(|position| browser?.entry_at(depth, position))
        };
        let origin = entry
            .as_ref()
            .filter(|_| searching)
            .map(|entry| entry.location.display_path());
        path.set_label(origin.as_deref().unwrap_or_default());
        path.set_visible(origin.is_some());
        row.set_tooltip_text(origin.as_deref());
        let active = entry.as_ref().is_some_and(|entry| {
            browser
                .as_ref()
                .is_some_and(|browser| browser.is_open_child(depth, &entry.location))
        });
        set_active_path_style(&row, active);
        set_cut_path_style(
            &row,
            entry.as_ref().is_some_and(|entry| {
                shared_cut_locations()
                    .iter()
                    .any(|cut| locations_equal(cut, &entry.location))
            }),
        );
        if let Some(entry) = entry.as_ref() {
            let mode_active = state
                .as_ref()
                .is_some_and(|state| state.mode_views.borrow().mode() == BrowserMode::Columns);
            if entry.is_directory() || mode_active {
                crate::ui::thumbnail::set_thumbnail_or_icon(
                    &icon,
                    entry,
                    entry_icon(entry),
                    17,
                    17,
                );
            } else {
                crate::assets::set_primary_icon(&icon, entry_icon(entry));
            }
            icon.set_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
            chevron.set_visible(entry.is_directory());
            if mode_active
                && let Some(state) = state.as_ref()
                && let Some(position) = source_position
                && metadata_needs_fill(entry)
            {
                state
                    .browser
                    .request_metadata_fill(depth, position, entry.location.clone());
            }
        } else {
            crate::ui::thumbnail::show_fallback_icon(&icon, crate::assets::icons::DOCUMENTS, 17);
            icon.set_opacity(0.72);
            chevron.set_visible(false);
        }
        let size_text = column_size_text(entry.as_ref());
        size.set_label(&size_text);
        size.set_visible(!size_text.is_empty());
    });
    factory.connect_unbind(|_, item| crate::ui::thumbnail::cancel_list_item_thumbnails(item));
    ColumnRows {
        factory,
        bound_rows,
    }
}
