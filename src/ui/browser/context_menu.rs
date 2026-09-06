// SPDX-License-Identifier: GPL-3.0-or-later

use super::chooser_context;
use crate::model::{FileEntry, Location};
use crate::services::ArchiveFormat;
use crate::ui::browser::clipboard::copy_locations;
use crate::ui::browser::customization::show_customize_modal;
use crate::ui::browser::desktop::{can_open_terminal, launch_terminal};
use crate::ui::browser::entry::{entry_icon, entry_supports_printing};
use crate::ui::browser::paths::{compact_display_path, is_trash_location};
use crate::ui::browser::{PinStatus, ViewState};
use crate::ui::browser_modes::BrowserMode;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CONTEXT_MENU_EDGE_MARGIN: i32 = 24;

fn context_menu_placement(anchor_height: i32, click_y: f64) -> (gtk::PositionType, i32) {
    let click_y = click_y.round() as i32;
    let above = click_y.clamp(0, anchor_height);
    let below = anchor_height.saturating_sub(above);
    let (position, available_height) = if below >= above {
        (gtk::PositionType::Bottom, below)
    } else {
        (gtk::PositionType::Top, above)
    };

    (
        position,
        available_height
            .saturating_sub(CONTEXT_MENU_EDGE_MARGIN)
            .max(1),
    )
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
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        point.x().round() as i32,
        point.y().round() as i32,
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
    let properties = context_menu_option(crate::assets::icons::INFO, "Properties", "");
    content.append(&new_folder);
    content.append(&new_file);
    content.append(&open_terminal);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&paste);
    content.append(&select_all);
    content.append(&refresh);
    content.append(&toggle_hidden);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
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
        if let Some(anchor) = gesture.widget() {
            show_context_popover(&popover_for_click, &scroll_for_click, &anchor, x, y);
        }
    });
    parent.add_controller(menu_click);
}

pub(in crate::ui) type ContextPickPosition = Rc<dyn Fn(&gtk::Widget) -> Option<u32>>;

pub(in crate::ui) type ContextSourcePosition = Rc<dyn Fn(u32) -> Option<usize>>;

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
    if !state.interactive {
        chooser_context::install_item(
            state,
            widget,
            selection,
            pick_position,
            source_position,
            clear_other_selections,
            depth,
        );
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
    let open_terminal =
        item_context_option(crate::assets::icons::TERMINAL, "Open in Terminal", "Ctrl+T");
    let preview = item_context_option(crate::assets::icons::EYE, "Quick preview", "Space");
    let print = item_context_option(crate::assets::icons::PRINTER, "Print", "");
    let restore = item_context_option(crate::assets::icons::FOLDER, "Restore", "");
    restore.set_visible(in_trash);
    let pin = item_context_option(crate::assets::icons::PIN, "Pin to sidebar", "P");
    let copy = item_context_option(crate::assets::icons::COPY, "Copy", "Ctrl+C");
    let copy_path = item_context_option(crate::assets::icons::COPY, "Copy path", "Y");
    let move_to = item_context_option(crate::assets::icons::FOLDER, "Move to…", "");
    let copy_to = item_context_option(crate::assets::icons::COPY, "Copy to…", "");
    let rename = item_context_option(crate::assets::icons::PENCIL, "Rename", "F2");
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
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&move_to);
    single.append(&copy_to);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&rename);
    single.append(&compress);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&customize);
    single.append(&properties);
    single.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    single.append(&move_to_trash);
    single.append(&permanent_delete);
    content.append(&single);

    let multiple = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let restore_multiple = item_context_option(crate::assets::icons::FOLDER, "Restore items", "");
    restore_multiple.set_visible(in_trash);
    let copy_multiple = item_context_option(crate::assets::icons::COPY, "Copy", "Ctrl+C");
    let copy_paths = item_context_option(crate::assets::icons::COPY, "Copy paths", "Y");
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
    multiple.append(&restore_multiple);
    multiple.append(&cut_multiple);
    multiple.append(&copy_multiple);
    multiple.append(&copy_paths);
    multiple.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    multiple.append(&move_multiple);
    multiple.append(&copy_to_multiple);
    multiple.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    multiple.append(&compress_multiple);
    multiple.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    multiple.append(&trash_multiple);
    multiple.append(&permanent_delete_multiple);
    multiple.set_visible(false);
    content.append(&multiple);

    let (popover, scroll) = context_menu_popover(&content);
    popover.add_css_class("folder-context-popover");
    popover.connect_closed(|popover| popover.unparent());

    let target = Rc::new(RefCell::new(None::<(usize, FileEntry)>));
    let weak = Rc::downgrade(state);
    let open_target = target.clone();
    let open_popover = popover.downgrade();
    open.connect_clicked(move |_| {
        if let Some(popover) = open_popover.upgrade() {
            popover.popdown();
        }
        let Some((position, _)) = open_target.borrow().clone() else {
            return;
        };
        if let Some(state) = weak.upgrade() {
            if state.mode_views.borrow().mode() == BrowserMode::Columns {
                state.browser.activate(depth, position);
            } else {
                state.browser.activate_in_place(depth, position);
            }
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
            state.browser.preview(depth, position);
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
    let rename_target = target.clone();
    let rename_popover = popover.downgrade();
    rename.connect_clicked(move |_| {
        if let Some(popover) = rename_popover.upgrade() {
            popover.popdown();
        }
        let Some((position, _)) = rename_target.borrow().clone() else {
            return;
        };
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.browser.select(depth, position);
                state.begin_rename();
            }
        });
    });
    for button in [&restore, &restore_multiple] {
        connect_selection_action(button, &popover, state, &target, |state, entries| {
            state.browser.restore(entries);
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

    let click = gtk::GestureClick::new();
    click.set_button(3);
    let weak_state = Rc::downgrade(state);
    let popover_for_reveal = popover.clone();
    let scroll_for_reveal = scroll.clone();
    let selection = selection.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(picked) = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
        else {
            return;
        };
        let Some(filtered_position) = pick_position(&picked) else {
            return;
        };
        let Some(resolved_position) = source_position(filtered_position) else {
            return;
        };
        let Some(state) = weak_state.upgrade() else {
            return;
        };
        let Some(entry) = state.browser.entry_at(depth, resolved_position) else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        state.browser.set_active_column(depth);
        if !selection.is_selected(filtered_position) {
            clear_other_selections();
            selection.select_item(filtered_position, true);
        }
        target.replace(Some((resolved_position, entry.clone())));
        let entries = context_entries(&state, &target);
        preview.set_visible(crate::ui::preview::entry_supports_quick_preview(&entry));
        print.set_visible(entry_supports_printing(&entry));
        open_terminal.set_visible(entry.is_directory() && can_open_terminal(&entry.location));
        let trash_visible = move_to_trash_is_visible(in_trash, state.browser.can_trash_at(depth));
        move_to_trash.set_visible(trash_visible);
        trash_multiple.set_visible(trash_visible);
        let permanent_delete_visible =
            permanently_delete_is_visible(in_trash, state.browser.can_delete_at(depth));
        permanent_delete.set_visible(permanent_delete_visible);
        permanent_delete_multiple.set_visible(permanent_delete_visible);
        pin.set_visible(entry.is_directory() && !is_trash_location(&entry.location));
        pin.set_sensitive(
            state
                .pin_status_handler
                .borrow()
                .as_ref()
                .is_some_and(|handler| handler(&entry.location) == PinStatus::Available),
        );
        extract.set_visible(ArchiveFormat::from_extension(&entry.display_name).is_some());
        extract_to.set_visible(ArchiveFormat::from_extension(&entry.display_name).is_some());
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
        show_context_popover(&popover_for_reveal, &scroll_for_reveal, &anchor, x, y);
    });
    widget.add_controller(click);
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

fn context_entries(
    state: &ViewState,
    target: &RefCell<Option<(usize, FileEntry)>>,
) -> Vec<FileEntry> {
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
    target: &Rc<RefCell<Option<(usize, FileEntry)>>>,
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
    target: &Rc<RefCell<Option<(usize, FileEntry)>>>,
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

/// Whether the "Move to Trash" context-menu option should be shown (issue #284).
///
/// Always visible while already browsing Trash, where it's really "Permanently
/// delete" under a shared label -- `can_trash` describes an ordinary location's
/// Trash support and has no bearing there. Otherwise, visible unless the
/// location's `access::can-trash` check came back a definite `Some(false)`;
/// `None` (not yet resolved, or the check itself couldn't be answered) defaults
/// to visible, since offering Trash and letting the operation fail is the
/// existing, safer fallback (issue #179) rather than ever hiding the only
/// delete option this menu has.
fn move_to_trash_is_visible(in_trash: bool, can_trash: Option<bool>) -> bool {
    in_trash || can_trash.unwrap_or(true)
}

/// Hidden in Trash, where the shared delete action is already permanent, or
/// when GIO confirms deletion is unsupported.
fn permanently_delete_is_visible(in_trash: bool, can_delete: Option<bool>) -> bool {
    !in_trash && can_delete.unwrap_or(true)
}

#[cfg(test)]
mod tests;
