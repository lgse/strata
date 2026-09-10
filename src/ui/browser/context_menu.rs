// SPDX-License-Identifier: MIT

use super::chooser_context;
use crate::adapters::gio_file_for_location;
use crate::model::{FileEntry, Location};
use crate::services::ArchiveFormat;
use crate::ui::browser::clipboard::{copy_locations, copy_names, locations_equal};
use crate::ui::browser::customization::show_customize_modal;
use crate::ui::browser::desktop::{can_open_terminal, launch_terminal};
use crate::ui::browser::entry::{entry_icon, entry_supports_printing};
use crate::ui::browser::paths::{
    can_remove_location, compact_display_path, is_trash_item, is_trash_location,
};
use crate::ui::browser::{PinStatus, ViewState};
use crate::ui::browser_modes::BrowserMode;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CONTEXT_MENU_EDGE_MARGIN: i32 = 16;

fn context_menu_placement(anchor_height: i32, click_y: f64) -> (gtk::PositionType, i32) {
    let click_y = click_y.round() as i32;
    let above = click_y.clamp(0, anchor_height);
    let below = anchor_height.saturating_sub(above);
    let position = if below >= above {
        gtk::PositionType::Bottom
    } else {
        gtk::PositionType::Top
    };

    (
        position,
        anchor_height
            .saturating_sub(CONTEXT_MENU_EDGE_MARGIN * 2)
            .max(1),
    )
}

// GTK popovers do not shift their anchor to use space across the click point.
fn shifted_anchor_y(
    position: gtk::PositionType,
    anchor_height: i32,
    click_y: i32,
    content_height: i32,
) -> i32 {
    let near_edge = CONTEXT_MENU_EDGE_MARGIN;
    let far_edge = anchor_height.saturating_sub(CONTEXT_MENU_EDGE_MARGIN);
    match position {
        gtk::PositionType::Bottom => {
            click_y.min(far_edge.saturating_sub(content_height).max(near_edge))
        }
        _ => click_y.max(near_edge.saturating_add(content_height).min(far_edge)),
    }
}

pub(super) fn context_menu_popover(
    content: &impl IsA<gtk::Widget>,
) -> (gtk::Popover, gtk::ScrolledWindow) {
    let scroll = gtk::ScrolledWindow::builder()
        .child(content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .build();
    scroll.add_css_class("context-menu-scroll");

    (
        gtk::Popover::builder()
            .child(&scroll)
            .autohide(true)
            .has_arrow(false)
            .build(),
        scroll,
    )
}

pub(super) fn bind_column_context_owner(
    state: &Rc<ViewState>,
    popover: &gtk::Popover,
    depth: usize,
) {
    let weak = Rc::downgrade(state);
    popover.connect_closed(move |_| {
        if let Some(state) = weak.upgrade()
            && state.context_menu_column.get() == Some(depth)
        {
            // Unmapping can focus the window's first control (for example Home).
            // Restore before another key is dispatched, not from a later idle callback.
            if state.browser.active_depth() == Some(depth) && !column_search_active(&state, depth) {
                state.browser.focus_active();
            }
            let generation = state.context_menu_generation.get();
            let weak = Rc::downgrade(&state);
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade()
                    && state.context_menu_generation.get() == generation
                    && state.context_menu_column.get() == Some(depth)
                {
                    state.context_menu_column.set(None);
                    state.refresh_destination_style();
                }
            });
        }
    });
}

fn column_search_active(state: &ViewState, depth: usize) -> bool {
    state
        .columns
        .borrow()
        .get(depth)
        .is_some_and(|column| column.search_handle.borrow().is_some())
}

pub(super) fn focus_context_column(state: &Rc<ViewState>, depth: usize) {
    if state.mode_views.borrow().mode() != crate::ui::browser_modes::BrowserMode::Columns {
        return;
    }
    state
        .context_menu_generation
        .set(state.context_menu_generation.get().wrapping_add(1));
    state.context_menu_column.set(Some(depth));
    state.browser.set_active_column(depth);
    // GTK restores pre-popup focus on dismissal; make that the menu's own column.
    if !column_search_active(state, depth) {
        state.browser.focus_active();
    }
    state.pointer_navigation();
}

pub(super) fn show_context_popover(
    popover: &gtk::Popover,
    scroll: &gtk::ScrolledWindow,
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
) {
    let Some(overlay) = crate::ui::modal::window_overlay(anchor) else {
        return;
    };
    if popover.parent().is_none() {
        overlay.add_overlay(popover);
    }
    let point = anchor
        .compute_point(&overlay, &gtk::graphene::Point::new(x as f32, y as f32))
        .unwrap_or(gtk::graphene::Point::new(x as f32, y as f32));
    let (position, max_content_height) =
        context_menu_placement(overlay.height(), f64::from(point.y()));
    popover.set_position(position);
    scroll.set_max_content_height(max_content_height);
    let click_y = point.y().round() as i32;
    let (_, content_height, _, _) = scroll.measure(gtk::Orientation::Vertical, -1);
    let anchor_y = shifted_anchor_y(position, overlay.height(), click_y, content_height.max(1));
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        point.x().round() as i32,
        anchor_y,
        1,
        1,
    )));
    popover.popup();
    popover.present();
}

pub(in crate::ui) fn install_folder_context_menu(
    state: &Rc<ViewState>,
    parent: &gtk::Widget,
    has_entries: Rc<dyn Fn() -> bool>,
    is_item_target: Rc<dyn Fn(&gtk::Widget) -> bool>,
    depth: usize,
    location: Location,
) {
    if !state.interactive {
        chooser_context::install_folder(state, parent, is_item_target, depth, location);
        return;
    }
    let content = crate::ui::accessibility::menu_box();
    content.add_css_class("folder-context-menu");
    let (popover, scroll) = context_menu_popover(&content);
    popover.add_css_class("folder-context-popover");
    bind_column_context_owner(state, &popover, depth);

    let new_folder = context_menu_option(
        crate::assets::icons::FOLDER_PLUS,
        "New Folder",
        "Ctrl+Shift+N",
    );
    let new_file = context_menu_option(crate::assets::icons::FILE_PLUS, "New File", "");
    let open_terminal =
        context_menu_option(crate::assets::icons::TERMINAL, "Open in Terminal", "Ctrl+T");
    let paste = context_menu_option(crate::assets::icons::CLIPBOARD_PASTE, "Paste", "Ctrl+V");
    let select_all = context_menu_option(crate::assets::icons::LIST_CHECKS, "Select All", "Ctrl+A");
    let refresh = context_menu_option(crate::assets::icons::REFRESH, "Refresh", "F5");
    let hidden_files_shown = state.browser.preferences().show_hidden;
    let (toggle_hidden, toggle_hidden_icon, toggle_hidden_label) = context_menu_toggle_option(
        if hidden_files_shown {
            crate::assets::icons::EYE
        } else {
            crate::assets::icons::EYE_OFF
        },
        if hidden_files_shown {
            "Hide Hidden Files"
        } else {
            "Show Hidden Files"
        },
        "Ctrl+H",
    );
    let customize = context_menu_option(crate::assets::icons::PALETTE, "Customize…", "");
    let properties = context_menu_option(crate::assets::icons::INFO, "Properties", "");
    let in_trash = is_trash_location(&location);
    customize.set_visible(!in_trash && location.native_path().is_some());
    new_folder.set_visible(!in_trash);
    new_file.set_visible(!in_trash);
    open_terminal.set_visible(!in_trash);
    paste.set_visible(!in_trash);
    content.append(&new_folder);
    content.append(&new_file);
    content.append(&open_terminal);
    if !in_trash {
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    }
    content.append(&paste);
    content.append(&select_all);
    content.append(&refresh);
    content.append(&toggle_hidden);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&customize);
    content.append(&properties);

    let pending_new_entry = Rc::new(Cell::new(None));
    let pending_for_click = pending_new_entry.clone();
    let new_folder_popover = popover.downgrade();
    new_folder.connect_clicked(move |_| {
        pending_for_click.set(Some(true));
        if let Some(popover) = new_folder_popover.upgrade() {
            popover.popdown();
        }
    });
    let pending_for_click = pending_new_entry.clone();
    let new_file_popover = popover.downgrade();
    new_file.connect_clicked(move |_| {
        pending_for_click.set(Some(false));
        if let Some(popover) = new_file_popover.upgrade() {
            popover.popdown();
        }
    });
    let weak = Rc::downgrade(state);
    let folder = location.clone();
    popover.connect_closed(move |popover| {
        popover.unparent();
        let Some(is_directory) = pending_new_entry.take() else {
            return;
        };
        let weak = weak.clone();
        let folder = folder.clone();
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.begin_new_entry(depth, folder, is_directory);
            }
        });
    });
    let weak = Rc::downgrade(state);
    let folder = location.clone();
    let paste_popover = popover.downgrade();
    paste.connect_clicked(move |_| {
        if let Some(popover) = paste_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.paste_into(folder.clone());
        }
    });
    let weak = Rc::downgrade(state);
    let select_popover = popover.downgrade();
    select_all.connect_clicked(move |_| {
        if let Some(popover) = select_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.select_all(depth);
        }
    });
    let weak = Rc::downgrade(state);
    let refresh_popover = popover.downgrade();
    refresh.connect_clicked(move |_| {
        if let Some(popover) = refresh_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.browser.retry_column(depth);
        }
    });
    let weak = Rc::downgrade(state);
    let toggle_hidden_popover = popover.downgrade();
    toggle_hidden.connect_clicked(move |_| {
        if let Some(popover) = toggle_hidden_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.browser.toggle_hidden();
        }
    });
    let weak = Rc::downgrade(state);
    let customize_popover = popover.downgrade();
    let customize_location = location.clone();
    customize.connect_clicked(move |_| {
        if let Some(popover) = customize_popover.upgrade() {
            popover.popdown();
        }
        let Some(state) = weak.upgrade() else {
            return;
        };
        let Some(path) = customize_location.native_path() else {
            return;
        };
        show_customize_modal(
            &state.overlay,
            path.to_path_buf(),
            true,
            crate::assets::icons::FOLDER,
        );
    });
    let weak = Rc::downgrade(state);
    let properties_popover = popover.downgrade();
    let properties_location = location.clone();
    properties.connect_clicked(move |_| {
        if let Some(popover) = properties_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            state.show_folder_properties(&properties_location);
        }
    });
    let weak = Rc::downgrade(state);
    let terminal_popover = popover.downgrade();
    let terminal_location = location.clone();
    open_terminal.connect_clicked(move |_| {
        if let Some(popover) = terminal_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            launch_terminal(&terminal_location, &state.overlay);
        }
    });

    let menu_click = gtk::GestureClick::new();
    menu_click.set_button(3);
    let popover_for_click = popover.clone();
    let browser_for_click = state.browser.clone();
    let scroll_for_click = scroll.clone();
    let weak_state = Rc::downgrade(state);
    menu_click.connect_pressed(move |gesture, _, x, y| {
        let over_item = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|picked| is_item_target(&picked));
        if over_item {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        paste.set_sensitive(gtk::gdk::Display::default().is_some_and(|display| {
            display
                .clipboard()
                .formats()
                .contains_type(gtk::gdk::FileList::static_type())
        }));
        select_all.set_sensitive(has_entries());
        open_terminal.set_sensitive(can_open_terminal(&location));
        let hidden_files_shown = browser_for_click.preferences().show_hidden;
        toggle_hidden_label.set_text(if hidden_files_shown {
            "Hide Hidden Files"
        } else {
            "Show Hidden Files"
        });
        crate::assets::set_primary_icon(
            &toggle_hidden_icon,
            if hidden_files_shown {
                crate::assets::icons::EYE
            } else {
                crate::assets::icons::EYE_OFF
            },
        );
        if let Some(anchor) = gesture.widget()
            && let Some(state) = weak_state.upgrade()
        {
            focus_context_column(&state, depth);
            show_context_popover(&popover_for_click, &scroll_for_click, &anchor, x, y);
        }
    });
    parent.add_controller(menu_click);
}

pub(in crate::ui) type ContextPickPosition = Rc<dyn Fn(&gtk::Widget) -> Option<u32>>;

pub(in crate::ui) type ContextSourcePosition = Rc<dyn Fn(u32) -> Option<usize>>;

pub(in crate::ui) type ContextTarget = (Option<usize>, FileEntry);
pub(in crate::ui) type ContextResolver = Rc<dyn Fn(&gtk::Widget) -> Option<ContextTarget>>;

const ITEM_CONTEXT_SUMMARY_MAX_CHARS: i32 = 60;

pub(in crate::ui) fn install_item_context_menu(
    state: &Rc<ViewState>,
    widget: &gtk::Widget,
    selection: &gtk::MultiSelection,
    pick_position: ContextPickPosition,
    source_position: ContextSourcePosition,
    clear_other_selections: Rc<dyn Fn()>,
    depth: usize,
) {
    let weak = Rc::downgrade(state);
    let selection = selection.clone();
    let resolve = Rc::new(move |picked: &gtk::Widget| {
        let position = pick_position(picked)?;
        let state = weak.upgrade()?;
        let columns = state.columns.borrow();
        let search = columns.get(depth).filter(|column| {
            state.mode_views.borrow().mode() == BrowserMode::Columns
                && column.search_handle.borrow().is_some()
        });
        let target = if let Some(column) = search {
            (
                None,
                super::search_result_entry(column.search_results.borrow().get(position as usize)?),
            )
        } else {
            let source = source_position(position)?;
            (Some(source), state.browser.entry_at(depth, source)?)
        };
        drop(columns);
        if !selection.is_selected(position) {
            clear_other_selections();
            selection.select_item(position, true);
        }
        Some(target)
    });
    install_resolved_item_context_menu(state, widget, resolve, depth);
}

pub(in crate::ui) fn install_resolved_item_context_menu(
    state: &Rc<ViewState>,
    widget: &gtk::Widget,
    resolve: ContextResolver,
    depth: usize,
) {
    if !state.interactive {
        chooser_context::install_item(state, widget, resolve, depth);
        return;
    }
    let in_trash = state
        .browser
        .location_at(depth)
        .as_ref()
        .is_some_and(is_trash_location);
    let content = crate::ui::accessibility::menu_box();
    content.add_css_class("item-context-menu");
    let header = gtk::Box::new(gtk::Orientation::Vertical, 2);
    header.add_css_class("item-context-header");
    let heading = gtk::Label::new(None);
    heading.add_css_class("item-context-title");
    heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
    heading.set_max_width_chars(ITEM_CONTEXT_SUMMARY_MAX_CHARS);
    heading.set_xalign(0.0);
    let summary = gtk::Label::new(None);
    summary.add_css_class("item-context-summary");
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    summary.set_max_width_chars(ITEM_CONTEXT_SUMMARY_MAX_CHARS);
    summary.set_xalign(0.0);
    header.append(&heading);
    header.append(&summary);
    content.append(&header);
    let header_separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    content.append(&header_separator);

    let single = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let open = item_context_option(crate::assets::icons::EXTERNAL_LINK, "Open", "↵");
    let open_with = item_context_option(crate::assets::icons::EXTERNAL_LINK, "Open With…", "");
    let open_terminal =
        item_context_option(crate::assets::icons::TERMINAL, "Open in Terminal", "Ctrl+T");
    let preview = item_context_option(crate::assets::icons::EYE, "Quick preview", "Space");
    let print = item_context_option(crate::assets::icons::PRINTER, "Print", "");
    let restore = item_context_option(crate::assets::icons::FOLDER, "Restore", "");
    restore.set_visible(in_trash);
    let pin = item_context_option(crate::assets::icons::PIN, "Pin to sidebar", "P");
    let copy = item_context_option(crate::assets::icons::COPY, "Copy", "Ctrl+C");
    let copy_path = item_context_option(crate::assets::icons::COPY, "Copy path", "Y");
    let copy_name = item_context_option(crate::assets::icons::COPY, "Copy name", "");
    let move_to = item_context_option(crate::assets::icons::FOLDER, "Move to…", "");
    let copy_to = item_context_option(crate::assets::icons::COPY, "Copy to…", "");
    let rename = item_context_option(crate::assets::icons::PENCIL, "Rename", "F2 / Ctrl+R");
    let cut = item_context_option(crate::assets::icons::SCISSORS, "Cut", "Ctrl+X");
    let delete_label = if in_trash {
        "Permanently delete"
    } else {
        "Move to Trash"
    };
    let move_to_trash = if in_trash {
        let option = item_context_danger_option(crate::assets::icons::TRASH, delete_label, "Del");
        option.add_css_class("danger");
        option
    } else {
        item_context_option(crate::assets::icons::TRASH, delete_label, "Del")
    };
    let permanent_delete = item_context_danger_option(
        crate::assets::icons::TRASH,
        "Permanently delete",
        "Shift+Del",
    );
    permanent_delete.add_css_class("danger");
    let properties = item_context_option(crate::assets::icons::INFO, "Properties", "Alt+Enter");
    let customize = item_context_option(crate::assets::icons::PALETTE, "Customize…", "");
    let compress = item_context_option(crate::assets::icons::FILE_ARCHIVE, "Compress…", "");
    let extract = item_context_option(crate::assets::icons::FILE_ARCHIVE, "Extract here", "");
    let extract_to = item_context_option(crate::assets::icons::FILE_ARCHIVE, "Extract to…", "");
    single.append(&open);
    single.append(&open_with);
    single.append(&open_terminal);
    single.append(&preview);
    single.append(&print);
    single.append(&restore);
    single.append(&extract);
    single.append(&extract_to);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&pin);
    single.append(&cut);
    single.append(&copy);
    single.append(&copy_path);
    single.append(&copy_name);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&move_to);
    single.append(&copy_to);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let archive_separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    single.append(&rename);
    single.append(&compress);
    single.append(&archive_separator);
    single.append(&customize);
    single.append(&properties);
    let delete_separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    single.append(&delete_separator);
    single.append(&move_to_trash);
    single.append(&permanent_delete);
    content.append(&single);

    let multiple = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let open_multiple = item_context_option(crate::assets::icons::EXTERNAL_LINK, "Open", "Enter");
    let open_with_multiple =
        item_context_option(crate::assets::icons::EXTERNAL_LINK, "Open With…", "");
    let restore_multiple = item_context_option(crate::assets::icons::FOLDER, "Restore items", "");
    restore_multiple.set_visible(in_trash);
    let copy_multiple = item_context_option(crate::assets::icons::COPY, "Copy", "Ctrl+C");
    let copy_paths = item_context_option(crate::assets::icons::COPY, "Copy paths", "Y");
    let copy_names_button = item_context_option(crate::assets::icons::COPY, "Copy names", "");
    let move_multiple = item_context_option(crate::assets::icons::FOLDER, "Move to…", "");
    let copy_to_multiple = item_context_option(crate::assets::icons::COPY, "Copy to…", "");
    let cut_multiple = item_context_option(crate::assets::icons::SCISSORS, "Cut", "Ctrl+X");
    let trash_multiple = if in_trash {
        let option = item_context_danger_option(crate::assets::icons::TRASH, delete_label, "Del");
        option.add_css_class("danger");
        option
    } else {
        item_context_option(crate::assets::icons::TRASH, delete_label, "Del")
    };
    let permanent_delete_multiple = item_context_danger_option(
        crate::assets::icons::TRASH,
        "Permanently delete",
        "Shift+Del",
    );
    permanent_delete_multiple.add_css_class("danger");
    let compress_multiple =
        item_context_option(crate::assets::icons::FILE_ARCHIVE, "Compress…", "");
    multiple.append(&open_multiple);
    multiple.append(&open_with_multiple);
    multiple.append(&restore_multiple);
    multiple.append(&cut_multiple);
    multiple.append(&copy_multiple);
    multiple.append(&copy_paths);
    multiple.append(&copy_names_button);
    multiple.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    multiple.append(&move_multiple);
    multiple.append(&copy_to_multiple);
    let multiple_transfer_separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    multiple.append(&multiple_transfer_separator);
    let multiple_archive_separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    multiple.append(&compress_multiple);
    multiple.append(&multiple_archive_separator);
    multiple.append(&trash_multiple);
    multiple.append(&permanent_delete_multiple);
    multiple.set_visible(false);
    content.append(&multiple);

    let (popover, scroll) = context_menu_popover(&content);
    popover.add_css_class("folder-context-popover");
    bind_column_context_owner(state, &popover, depth);

    let target = Rc::new(RefCell::new(None::<ContextTarget>));
    let open_with_selection = Rc::new(RefCell::new(None::<OpenWithSelection>));
    let open_with_generation = Rc::new(Cell::new(0_u64));
    let generation_for_close = open_with_generation.clone();
    popover.connect_closed(move |popover| {
        generation_for_close.set(generation_for_close.get().wrapping_add(1));
        popover.unparent();
    });
    let weak = Rc::downgrade(state);
    let open_target = target.clone();
    let open_popover = popover.downgrade();
    open.connect_clicked(move |_| {
        if let Some(popover) = open_popover.upgrade() {
            popover.popdown();
        }
        let Some((position, entry)) = open_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade() {
            if let Some(position) = current_context_position(&state, depth, position, &entry) {
                if state.mode_views.borrow().mode() == BrowserMode::Columns {
                    state.browser.activate(depth, position);
                } else {
                    state.browser.activate_in_place(depth, position);
                }
            } else if entry.is_directory() {
                state.browser.navigate(entry.location);
            } else {
                state.browser.open_location(entry.location);
            }
        }
    });
    let open_multiple_target = target.clone();
    let open_multiple_state = Rc::downgrade(state);
    let open_multiple_popover = popover.downgrade();
    let open_multiple_selection = open_with_selection.clone();
    open_multiple.connect_clicked(move |_| {
        if let Some(popover) = open_multiple_popover.upgrade() {
            popover.popdown();
        }
        let Some(state) = open_multiple_state.upgrade() else {
            return;
        };
        let Some(selection) = open_multiple_selection.borrow().clone() else {
            return;
        };
        if !selection.entries_match_target(&context_entries(&state, &open_multiple_target)) {
            return;
        }
        let Some(app) = selection.default else {
            return;
        };
        let context = state.overlay.display().app_launch_context();
        if let Err(error) = crate::ui::open_with::launch(&app, &selection.files, Some(&context)) {
            crate::ui::modal::show_error_dialog(
                &state.overlay,
                "Unable to open file",
                error.message(),
            );
        }
    });
    let open_with_target = target.clone();
    let open_with_state = Rc::downgrade(state);
    let open_with_popover = popover.downgrade();
    let open_with_selection_for_click = open_with_selection.clone();
    open_with.connect_clicked(move |_| {
        if let Some(popover) = open_with_popover.upgrade() {
            popover.popdown();
        }
        let Some(state) = open_with_state.upgrade() else {
            return;
        };
        let Some(selection) = open_with_selection_for_click.borrow().clone() else {
            return;
        };
        if selection.entries_match_target(&context_entries(&state, &open_with_target)) {
            let browser = Rc::downgrade(&state.browser);
            crate::ui::open_with::show(
                &state.overlay,
                selection.files,
                selection.apps,
                Rc::new(move || {
                    if let Some(browser) = browser.upgrade() {
                        browser.focus_active();
                    }
                }),
            );
        }
    });
    let open_with_multiple_target = target.clone();
    let open_with_multiple_state = Rc::downgrade(state);
    let open_with_multiple_popover = popover.downgrade();
    let open_with_multiple_selection = open_with_selection.clone();
    open_with_multiple.connect_clicked(move |_| {
        if let Some(popover) = open_with_multiple_popover.upgrade() {
            popover.popdown();
        }
        let Some(state) = open_with_multiple_state.upgrade() else {
            return;
        };
        let Some(selection) = open_with_multiple_selection.borrow().clone() else {
            return;
        };
        if selection.entries_match_target(&context_entries(&state, &open_with_multiple_target)) {
            let browser = Rc::downgrade(&state.browser);
            crate::ui::open_with::show(
                &state.overlay,
                selection.files,
                selection.apps,
                Rc::new(move || {
                    if let Some(browser) = browser.upgrade() {
                        browser.focus_active();
                    }
                }),
            );
        }
    });
    let weak = Rc::downgrade(state);
    let preview_target = target.clone();
    let preview_popover = popover.downgrade();
    preview.connect_clicked(move |_| {
        if let Some(popover) = preview_popover.upgrade() {
            popover.popdown();
        }
        let Some((position, entry)) = preview_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade()
            && !entry.is_directory()
        {
            preview_context_entry(&state, depth, position, entry);
        }
    });
    let weak = Rc::downgrade(state);
    let print_target = target.clone();
    let print_popover = popover.downgrade();
    print.connect_clicked(move |_| {
        if let Some(popover) = print_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = print_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade()
            && let Some(print) = state.print_handler.borrow().as_ref()
            && entry_supports_printing(&entry)
        {
            print(entry);
        }
    });
    let weak = Rc::downgrade(state);
    let terminal_target = target.clone();
    let terminal_popover = popover.downgrade();
    open_terminal.connect_clicked(move |_| {
        if let Some(popover) = terminal_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = terminal_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade() {
            launch_terminal(&entry.location, &state.overlay);
        }
    });
    let weak = Rc::downgrade(state);
    let pin_target = target.clone();
    let pin_popover = popover.downgrade();
    pin.connect_clicked(move |_| {
        if let Some(popover) = pin_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = pin_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade()
            && entry.is_directory()
            && let Some(handler) = state.pin_handler.borrow().as_ref()
        {
            handler(entry.location, entry.display_name);
        }
    });
    let weak = Rc::downgrade(state);
    let copy_target = target.clone();
    let copy_popover = popover.downgrade();
    copy_path.connect_clicked(move |_| {
        if let Some(popover) = copy_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = copy_target.borrow().clone() else {
            return;
        };
        if weak.upgrade().is_some() {
            copy_locations(&[entry]);
        }
    });
    let weak = Rc::downgrade(state);
    let copy_name_target = target.clone();
    let copy_name_popover = popover.downgrade();
    copy_name.connect_clicked(move |_| {
        if let Some(popover) = copy_name_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = copy_name_target.borrow().clone() else {
            return;
        };
        if weak.upgrade().is_some() {
            copy_names(&[entry]);
        }
    });
    let weak = Rc::downgrade(state);
    let rename_target = target.clone();
    let rename_popover = popover.downgrade();
    rename.connect_clicked(move |_| {
        if let Some(popover) = rename_popover.upgrade() {
            popover.popdown();
        }
        let Some((position, entry)) = rename_target.borrow().clone() else {
            return;
        };
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                rename_context_entry(&state, depth, position, entry);
            }
        });
    });
    for button in [&restore, &restore_multiple] {
        connect_selection_action(button, &popover, state, &target, |state, entries| {
            state.request_restore(entries);
        });
    }
    for (button, moving) in [
        (&move_to, true),
        (&copy_to, false),
        (&move_multiple, true),
        (&copy_to_multiple, false),
    ] {
        connect_selection_action(button, &popover, state, &target, move |state, entries| {
            state.show_transfer_dialog(entries, moving);
        });
    }
    for button in [&cut, &cut_multiple] {
        connect_selection_action(button, &popover, state, &target, |state, entries| {
            state.cut_entries(&entries);
        });
    }
    for button in [&copy, &copy_multiple] {
        connect_selection_action(button, &popover, state, &target, |state, entries| {
            state.copy_entries(&entries);
        });
    }
    for (button, permanent) in [
        (&move_to_trash, in_trash),
        (&trash_multiple, in_trash),
        (&permanent_delete, true),
        (&permanent_delete_multiple, true),
    ] {
        connect_selection_action(button, &popover, state, &target, move |state, entries| {
            state.request_delete(entries, permanent);
        });
    }
    for button in [&compress, &compress_multiple] {
        connect_selection_action(button, &popover, state, &target, |state, entries| {
            state.show_compress_dialog(entries);
        });
    }
    connect_context_extract(&extract, &popover, state, &target, false);
    connect_context_extract(&extract_to, &popover, state, &target, true);
    let weak = Rc::downgrade(state);
    let properties_target = target.clone();
    let properties_popover = popover.downgrade();
    properties.connect_clicked(move |_| {
        if let Some(popover) = properties_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = properties_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade() {
            state.show_entry_properties(entry);
        }
    });
    let weak = Rc::downgrade(state);
    let paths_target = target.clone();
    let paths_popover = popover.downgrade();
    copy_paths.connect_clicked(move |_| {
        if let Some(popover) = paths_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            copy_locations(&context_entries(&state, &paths_target));
        }
    });
    let weak = Rc::downgrade(state);
    let names_target = target.clone();
    let names_popover = popover.downgrade();
    copy_names_button.connect_clicked(move |_| {
        if let Some(popover) = names_popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            copy_names(&context_entries(&state, &names_target));
        }
    });
    let weak = Rc::downgrade(state);
    let customize_target = target.clone();
    let customize_popover = popover.downgrade();
    customize.connect_clicked(move |_| {
        if let Some(popover) = customize_popover.upgrade() {
            popover.popdown();
        }
        let Some((_, entry)) = customize_target.borrow().clone() else {
            return;
        };
        let Some(state) = weak.upgrade() else {
            return;
        };
        let Some(path) = entry.location.native_path() else {
            return;
        };
        show_customize_modal(
            &state.overlay,
            path.to_path_buf(),
            entry.is_directory(),
            entry_icon(&entry),
        );
    });

    let context_view = widget.downgrade();
    popover.connect_show(move |_| {
        if let Some(view) = context_view.upgrade() {
            view.add_css_class("context-selection");
        }
    });
    let context_view = widget.downgrade();
    popover.connect_closed(move |_| {
        if let Some(view) = context_view.upgrade() {
            view.remove_css_class("context-selection");
        }
    });

    let click = gtk::GestureClick::new();
    click.set_button(3);
    // Claim secondary clicks before ListView's row gestures consume them.
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_state = Rc::downgrade(state);
    let popover_for_reveal = popover.clone();
    let scroll_for_reveal = scroll.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(picked) = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
        else {
            return;
        };
        let Some(state) = weak_state.upgrade() else {
            return;
        };
        let Some((position, entry)) = resolve(&picked) else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        state.browser.set_active_column(depth);
        target.replace(Some((position, entry.clone())));
        let entries = context_entries(&state, &target);
        let open_with_entries = entries.clone();
        open_with.set_visible(open_with_entries.len() == 1);
        open_with_multiple.set_visible(open_with_entries.len() > 1);
        open_multiple.set_visible(false);
        for button in [&open_with, &open_with_multiple] {
            button.set_sensitive(false);
            set_open_with_explanation(button, Some("Looking for compatible applications…"));
        }
        open_with_selection.replace(None);
        let generation = open_with_generation.get().wrapping_add(1);
        open_with_generation.set(generation);
        prepare_open_with(
            open_with_entries,
            &open_with,
            &open_with_multiple,
            &open_multiple,
            &open_with_selection,
            &open_with_generation,
            generation,
        );
        let removable = entries
            .iter()
            .all(|entry| can_remove_location(&entry.location));
        let restorable = entries.iter().all(|entry| is_trash_item(&entry.location));
        restore.set_visible(restorable);
        restore_multiple.set_visible(restorable);
        for button in [&cut, &cut_multiple, &move_to, &move_multiple] {
            button.set_visible(removable);
        }
        let rename_visible = !is_trash_location(&entry.location);
        rename.set_visible(rename_visible);
        let can_compress = entries
            .iter()
            .all(|entry| entry.location.native_path().is_some());
        compress.set_visible(can_compress);
        compress_multiple.set_visible(can_compress);
        archive_separator.set_visible(rename_visible || can_compress);
        multiple_archive_separator.set_visible(can_compress);
        preview.set_visible(crate::ui::preview::entry_supports_quick_preview(&entry));
        print.set_visible(entry_supports_printing(&entry));
        open_terminal.set_visible(entry.is_directory() && can_open_terminal(&entry.location));
        let trash_visible =
            removable && move_to_trash_is_visible(in_trash, state.browser.can_trash_at(depth));
        move_to_trash.set_visible(trash_visible);
        trash_multiple.set_visible(trash_visible);
        let permanent_delete_visible =
            permanently_delete_is_visible(in_trash, state.browser.can_delete_at(depth));
        permanent_delete.set_visible(permanent_delete_visible);
        permanent_delete_multiple.set_visible(permanent_delete_visible);
        delete_separator.set_visible(trash_visible || permanent_delete_visible);
        multiple_transfer_separator
            .set_visible(can_compress || trash_visible || permanent_delete_visible);
        pin.set_visible(entry.is_directory() && !is_trash_location(&entry.location));
        pin.set_sensitive(
            state
                .pin_status_handler
                .borrow()
                .as_ref()
                .is_some_and(|handler| handler(&entry.location) == PinStatus::Available),
        );
        let can_extract = entry.location.native_path().is_some()
            && ArchiveFormat::from_extension(&entry.display_name).is_some();
        extract.set_visible(can_extract);
        extract_to.set_visible(can_extract);
        customize
            .set_visible(!in_trash && entries.len() == 1 && entry.location.native_path().is_some());
        if entries.len() > 1 {
            heading.set_text(&format!("{} items selected", entries.len()));
            summary.set_text(&selected_items_summary(&entries));
            single.set_visible(false);
            multiple.set_visible(true);
        } else {
            heading.set_text(&entry.display_name);
            summary.set_text(&compact_display_path(&entry.location));
            single.set_visible(true);
            multiple.set_visible(false);
        }
        let Some(anchor) = gesture.widget() else {
            return;
        };
        focus_context_column(&state, depth);
        show_context_popover(&popover_for_reveal, &scroll_for_reveal, &anchor, x, y);
    });
    widget.add_controller(click);
}

fn current_context_position(
    state: &ViewState,
    depth: usize,
    position: Option<usize>,
    entry: &FileEntry,
) -> Option<usize> {
    position.filter(|position| {
        state
            .browser
            .entry_at(depth, *position)
            .is_some_and(|current| current.location == entry.location)
    })
}

pub(super) fn preview_context_entry(
    state: &Rc<ViewState>,
    depth: usize,
    position: Option<usize>,
    entry: FileEntry,
) {
    if let Some(position) = current_context_position(state, depth, position, &entry) {
        state.browser.preview(depth, position);
    } else {
        state.browser.request_preview(entry);
    }
}

pub(super) fn rename_context_entry(
    state: &Rc<ViewState>,
    depth: usize,
    position: Option<usize>,
    entry: FileEntry,
) {
    if let Some(position) = current_context_position(state, depth, position, &entry) {
        state.browser.select(depth, position);
        let weak = Rc::downgrade(state);
        // Selection queues collection focus; enter the editor after it settles.
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.begin_rename();
            }
        });
        return;
    }
    use crate::ui::{
        controls::{form_entry, modal_layout},
        modal::{ModalHost, dismiss_modal_layer, modal_layer, submit_on_enter},
    };
    let Some(host) = ModalHost::blurred_for(&state.overlay) else {
        return;
    };
    let layout = modal_layout(
        crate::assets::icons::PENCIL,
        "Rename",
        &compact_display_path(&entry.location),
        "Rename",
    );
    let field = form_entry();
    field.set_text(&entry.display_name);
    super::super::accessibility::set_label(&field, "Name");
    layout.body.append(&field);
    let confirm = layout.confirm.downgrade();
    field.connect_changed(move |field| {
        if let Some(confirm) = confirm.upgrade() {
            confirm.set_sensitive(super::update_basename_validation(field));
        }
    });
    let layer = modal_layer(
        &layout.content,
        &host.overlay,
        host.blurred_root.clone(),
        None,
    );
    let weak_layer = layer.downgrade();
    let dismiss = Rc::new(move || {
        if let Some(layer) = weak_layer.upgrade() {
            dismiss_modal_layer(&layer, &host.overlay, host.blurred_root.as_ref());
        }
    });
    for button in [&layout.cancel, &layout.close] {
        let dismiss = dismiss.clone();
        button.connect_clicked(move |_| dismiss());
    }
    let escape = gtk::EventControllerKey::new();
    let dismiss_for_escape = dismiss.clone();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            dismiss_for_escape();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    submit_on_enter(&layout.body, &layout.confirm);
    let weak = Rc::downgrade(state);
    let field_for_submit = field.clone();
    layout.confirm.connect_clicked(move |_| {
        if super::update_basename_validation(&field_for_submit) {
            let name = field_for_submit.text().to_string();
            dismiss();
            if let Some(state) = weak.upgrade() {
                super::queue_rename(&state.browser, entry.clone(), name);
            }
        }
    });
    if let Some(overlay) = crate::ui::modal::window_overlay(&state.overlay) {
        overlay.add_overlay(&layer);
    }
    field.grab_focus();
    field.select_region(0, -1);
}

fn selected_items_summary(entries: &[FileEntry]) -> String {
    let mut names = entries
        .iter()
        .take(3)
        .map(|entry| entry.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if entries.len() > 3 {
        names.push_str(", …");
    }
    let max_chars = ITEM_CONTEXT_SUMMARY_MAX_CHARS as usize;
    if names.chars().count() > max_chars {
        names = names.chars().take(max_chars - 1).collect();
        names.push('…');
    }
    names
}

fn context_entries(state: &ViewState, target: &RefCell<Option<ContextTarget>>) -> Vec<FileEntry> {
    if let Some((None, entry)) = target.borrow().as_ref() {
        return vec![entry.clone()];
    }
    state.sync_mode_selection();
    let entries = state.browser.selected_entries();
    let target = target.borrow();
    let Some((_, target)) = target.as_ref() else {
        return entries;
    };
    if entries
        .iter()
        .any(|entry| entry.location == target.location)
    {
        entries
    } else {
        vec![target.clone()]
    }
}

fn connect_selection_action(
    button: &gtk::Button,
    popover: &gtk::Popover,
    state: &Rc<ViewState>,
    target: &Rc<RefCell<Option<ContextTarget>>>,
    run: impl Fn(&Rc<ViewState>, Vec<FileEntry>) + 'static,
) {
    let weak = Rc::downgrade(state);
    let target = target.clone();
    let popover = popover.downgrade();
    button.connect_clicked(move |_| {
        if let Some(popover) = popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            let entries = context_entries(&state, &target);
            run(&state, entries);
        }
    });
}

fn connect_context_extract(
    button: &gtk::Button,
    popover: &gtk::Popover,
    state: &Rc<ViewState>,
    target: &Rc<RefCell<Option<ContextTarget>>>,
    pick_destination: bool,
) {
    let weak = Rc::downgrade(state);
    let target = target.clone();
    let popover = popover.downgrade();
    button.connect_clicked(move |_| {
        if let Some(popover) = popover.upgrade() {
            popover.popdown();
        }
        if let Some(state) = weak.upgrade() {
            let Some((_, entry)) = target.borrow().clone() else {
                return;
            };
            if pick_destination {
                state.show_extract_to_dialog(entry);
            } else {
                state.extract_entry(entry);
            }
        }
    });
}

fn item_context_option(icon: &str, label: &str, accelerator: &str) -> gtk::Button {
    item_context_option_with_icon(crate::assets::primary_icon(icon, 15), label, accelerator)
}

fn item_context_danger_option(icon: &str, label: &str, accelerator: &str) -> gtk::Button {
    item_context_option_with_icon(crate::assets::danger_icon(icon, 15), label, accelerator)
}

fn item_context_option_with_icon(icon: gtk::Image, label: &str, accelerator: &str) -> gtk::Button {
    let button = crate::ui::accessibility::menu_item_button();
    crate::ui::accessibility::describe_menu_item(&button, label, accelerator);
    button.add_css_class("item-context-option");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    icon.add_css_class("item-context-icon");
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&icon);
    row.append(&title);
    if !accelerator.is_empty() {
        let shortcut = gtk::Label::new(Some(accelerator));
        shortcut.add_css_class("item-context-shortcut");
        row.append(&shortcut);
    }
    button.set_child(Some(&row));
    button
}

fn context_menu_row(
    icon: &str,
    label: &str,
    accelerator: &str,
) -> (gtk::Box, gtk::Image, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = crate::assets::primary_icon(icon, 15);
    icon.add_css_class("folder-context-icon");
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&icon);
    row.append(&title);
    if !accelerator.is_empty() {
        let shortcut = gtk::Label::new(Some(accelerator));
        shortcut.add_css_class("folder-context-shortcut");
        row.append(&shortcut);
    }
    (row, icon, title)
}

pub(super) fn context_menu_option(icon: &str, label: &str, accelerator: &str) -> gtk::Button {
    let (row, _, _) = context_menu_row(icon, label, accelerator);
    let button = crate::ui::accessibility::menu_item_button();
    crate::ui::accessibility::describe_menu_item(&button, label, accelerator);
    button.add_css_class("folder-context-option");
    button.set_child(Some(&row));
    button
}

fn context_menu_toggle_option(
    icon: &str,
    label: &str,
    accelerator: &str,
) -> (gtk::Button, gtk::Image, gtk::Label) {
    let (row, icon, title) = context_menu_row(icon, label, accelerator);
    let button = crate::ui::accessibility::menu_item_button();
    crate::ui::accessibility::describe_menu_item(&button, label, accelerator);
    button.add_css_class("folder-context-option");
    button.set_child(Some(&row));
    (button, icon, title)
}

/// In Trash this shared action deletes permanently, so `can_trash` is irrelevant.
/// Unknown capabilities retain the delete fallback (#179); callers exclude
/// nested Trash children, which GVfs cannot remove independently.
fn move_to_trash_is_visible(in_trash: bool, can_trash: Option<bool>) -> bool {
    in_trash || can_trash.unwrap_or(true)
}

/// Hidden in Trash, where the shared delete action is already permanent, or
/// when GIO confirms deletion is unsupported.
fn permanently_delete_is_visible(in_trash: bool, can_delete: Option<bool>) -> bool {
    !in_trash && can_delete.unwrap_or(true)
}

#[derive(Clone)]
struct OpenWithSelection {
    locations: Vec<Location>,
    files: Vec<gio::File>,
    apps: Vec<gio::AppInfo>,
    default: Option<gio::AppInfo>,
}

impl OpenWithSelection {
    fn entries_match_target(&self, entries: &[FileEntry]) -> bool {
        self.locations.len() == entries.len()
            && self
                .locations
                .iter()
                .zip(entries)
                .all(|(location, entry)| locations_equal(location, &entry.location))
    }
}

fn set_open_with_explanation(button: &gtk::Button, explanation: Option<&str>) {
    button.set_tooltip_text(explanation);
    button.update_property(&[gtk::accessible::Property::Description(
        explanation.unwrap_or(""),
    )]);
}

fn prepare_open_with(
    entries: Vec<FileEntry>,
    single_button: &gtk::Button,
    multiple_button: &gtk::Button,
    open_button: &gtk::Button,
    result: &Rc<RefCell<Option<OpenWithSelection>>>,
    generation: &Rc<Cell<u64>>,
    expected_generation: u64,
) {
    if entries.is_empty() {
        return;
    }
    let locations = entries
        .iter()
        .map(|entry| entry.location.clone())
        .collect::<Vec<_>>();
    let files = entries
        .iter()
        .map(|entry| gio_file_for_location(&entry.location))
        .collect::<Vec<_>>();
    let single_button = single_button.clone();
    let multiple_button = multiple_button.clone();
    let open_button = open_button.clone();
    let result = result.clone();
    let generation = generation.clone();
    glib::MainContext::default().spawn_local(async move {
        let unavailable = |reason: &str| {
            for button in [&single_button, &multiple_button] {
                button.set_sensitive(false);
                set_open_with_explanation(button, Some(reason));
            }
        };
        let mut content_types = Vec::<String>::new();
        for file in &files {
            if generation.get() != expected_generation {
                return;
            }
            let info = file
                .query_info_future(
                    "standard::type,standard::content-type",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await;
            if generation.get() != expected_generation {
                return;
            }
            let Ok(info) = info else {
                unavailable("Unable to read the selected file type");
                return;
            };
            if info.file_type() == gio::FileType::SymbolicLink {
                unavailable("Broken symbolic links cannot be opened with an application");
                return;
            }
            let Some(next_type) = info.content_type().map(|value| value.to_string()) else {
                unavailable("Unable to determine the selected file type");
                return;
            };
            if !content_types
                .iter()
                .any(|value| gio::content_type_equals(value, &next_type))
            {
                content_types.push(next_type);
            }
        }
        if generation.get() != expected_generation {
            return;
        }
        let requires_uris = files.iter().any(|file| !file.is_native());
        let (apps, default) = common_applications(&content_types, requires_uris);
        let available = !apps.is_empty();
        let explanation = if available {
            None
        } else if content_types.len() > 1 {
            Some("No application can open all selected file types")
        } else {
            Some("No compatible applications were found")
        };
        open_button.set_visible(default.is_some());
        result.replace(Some(OpenWithSelection {
            locations,
            files,
            apps,
            default,
        }));
        for button in [&single_button, &multiple_button] {
            button.set_sensitive(available);
            set_open_with_explanation(button, explanation);
        }
    });
}

fn common_applications(
    content_types: &[String],
    requires_uris: bool,
) -> (Vec<gio::AppInfo>, Option<gio::AppInfo>) {
    let Some(first) = content_types.first() else {
        return (vec![], None);
    };
    let mut apps = crate::ui::open_with::compatible_apps(first, requires_uris);
    let mut default = gio::AppInfo::default_for_type(first, requires_uris);
    for content_type in &content_types[1..] {
        let next = crate::ui::open_with::compatible_apps(content_type, requires_uris);
        apps.retain(|app| next.iter().any(|candidate| candidate.equal(app)));
        let next_default = gio::AppInfo::default_for_type(content_type, requires_uris);
        default = default.filter(|app| next_default.as_ref().is_some_and(|next| next.equal(app)));
    }
    (apps, default)
}

#[cfg(test)]
mod tests;
