// SPDX-License-Identifier: MIT

use super::item_options::{CreationOptions, DeleteOptions, ItemMenuOptions, OpenInRow};
use super::open_with::{
    OpenWithButtons, OpenWithLookup, OpenWithSelection, prepare_open_with,
    set_open_with_explanation,
};
use super::{
    ContextPickPosition, ContextResolver, ContextSourcePosition, ContextTarget,
    ResolvedContextTrigger, actions, bind_column_context_owner, context_entries,
    context_filter_or_search_active, context_search_active, current_context_position,
    focus_context_entry, install_secondary_release_retarget, move_to_trash_is_visible,
    permanently_delete_is_visible, preview_context_entry, rename_context_entry, schedule_new_entry,
    selected_items_summary,
};
use crate::adapters::gio_file_for_location;
use crate::model::{FileEntry, Location};
use crate::services::ArchiveFormat;
use crate::ui::browser::chooser_context;
use crate::ui::browser::clipboard::{copy_locations, copy_names};
use crate::ui::browser::customization::show_customize_modal;
use crate::ui::browser::desktop::{can_open_terminal, launch_terminal};
use crate::ui::browser::entry::entry_icon;
use crate::ui::browser::paths::{
    can_remove_location, compact_display_path, is_trash_item, is_trash_location,
};
use crate::ui::browser::{PinStatus, ViewState};
use crate::ui::browser_modes::BrowserMode;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

pub(in crate::ui) fn install_item_context_menu(
    state: &Rc<ViewState>,
    widget: &gtk::Widget,
    selection: &gtk::MultiSelection,
    pick_position: ContextPickPosition,
    source_position: ContextSourcePosition,
    clear_other_selections: Rc<dyn Fn()>,
    depth: usize,
) -> Rc<dyn Fn(f64, f64)> {
    let weak = Rc::downgrade(state);
    let selection = selection.clone();
    let resolve = Rc::new(move |picked: &gtk::Widget| {
        let position = pick_position(picked)?;
        let state = weak.upgrade()?;
        let target = picked_context_target(&state, depth, position, &source_position)?;
        if !selection.is_selected(position) {
            clear_other_selections();
            selection.select_item(position, true);
        }
        Some(target)
    });
    install_resolved_item_context_menu(state, widget, resolve, depth)
}

/// A recursive search lists its own results; otherwise the row maps to the listing.
fn picked_context_target(
    state: &ViewState,
    depth: usize,
    position: u32,
    source_position: &ContextSourcePosition,
) -> Option<ContextTarget> {
    let columns = state.columns.borrow();
    let search = columns.get(depth).filter(|column| {
        state.mode_views.borrow().mode() == BrowserMode::Columns
            && column.recursive_search_active.get()
    });
    if let Some(column) = search {
        let results = column.search_results.borrow();
        let result = results.get(position as usize)?;
        return Some((None, crate::ui::browser::search_result_entry(result)));
    }
    let source = source_position(position)?;
    Some((Some(source), state.browser.entry_at(depth, source)?))
}

/// What every item-menu handler needs: the view, its popover, and the clicked target.
#[derive(Clone)]
struct ItemMenu {
    state: Weak<ViewState>,
    popover: glib::WeakRef<gtk::Popover>,
    target: Rc<RefCell<Option<ContextTarget>>>,
    depth: usize,
}

impl ItemMenu {
    fn popdown(&self) {
        if let Some(popover) = self.popover.upgrade() {
            popover.popdown();
        }
    }

    /// Closes the menu, then runs `run` on the clicked item while the view is alive.
    fn on_entry(
        &self,
        button: &gtk::Button,
        run: impl Fn(&Rc<ViewState>, Option<usize>, FileEntry) + 'static,
    ) {
        let menu = self.clone();
        button.connect_clicked(move |_| {
            menu.popdown();
            let Some((position, entry)) = menu.target.borrow().clone() else {
                return;
            };
            if let Some(state) = menu.state.upgrade() {
                run(&state, position, entry);
            }
        });
    }

    /// Closes the menu, then runs `run` on the selection the menu applies to.
    fn on_selection(
        &self,
        button: &gtk::Button,
        run: impl Fn(&Rc<ViewState>, Vec<FileEntry>) + 'static,
    ) {
        let menu = self.clone();
        button.connect_clicked(move |_| {
            menu.popdown();
            if let Some(state) = menu.state.upgrade() {
                let entries = context_entries(&state, &menu.target);
                run(&state, entries);
            }
        });
    }

    /// Runs `run` with the resolved applications only while they still describe
    /// the menu's selection.
    fn on_open_with(
        &self,
        button: &gtk::Button,
        lookup: &OpenWithLookup,
        run: fn(&Rc<ViewState>, OpenWithSelection),
    ) {
        let menu = self.clone();
        let selection = lookup.selection.clone();
        button.connect_clicked(move |_| {
            menu.popdown();
            let Some(state) = menu.state.upgrade() else {
                return;
            };
            let Some(selection) = selection.borrow().clone() else {
                return;
            };
            if selection.entries_match_target(&context_entries(&state, &menu.target)) {
                run(&state, selection);
            }
        });
    }

    fn connect_open_in(&self, open_in: &OpenInRow) {
        for (button, action) in [
            (&open_in.tab, "win.open-tab-at"),
            (&open_in.window, "win.open-window-at"),
        ] {
            self.on_selection(button, move |state, entries| {
                open_entries_in_action(state, entries, action);
            });
        }
    }

    fn connect_delete(&self, delete: &DeleteOptions, in_trash: bool) {
        for (button, permanent) in [
            (&delete.trash, in_trash),
            (&delete.permanent, true),
            (&delete.without_trash, true),
        ] {
            self.on_selection(button, move |state, entries| {
                state.request_delete(entries, permanent);
            });
        }
    }

    fn connect_preview(&self, button: &gtk::Button) {
        let menu = self.clone();
        button.connect_clicked(move |_| {
            let target = menu.target.borrow().clone();
            let menu = menu.clone();
            // Dismiss after the click finishes so it cannot select a result beneath the menu.
            glib::idle_add_local_once(move || {
                menu.popdown();
                let Some((position, entry)) = target else {
                    return;
                };
                glib::idle_add_local_once(move || {
                    if let Some(state) = menu.state.upgrade()
                        && !entry.is_directory()
                    {
                        focus_context_entry(&state, menu.depth, position, &entry);
                        preview_context_entry(&state, menu.depth, position, entry);
                    }
                });
            });
        });
    }

    fn connect_open_rows(&self, options: &ItemMenuOptions, lookup: &OpenWithLookup) {
        let single = &options.single_open;
        let multiple = &options.multiple_open;
        let depth = self.depth;
        self.on_entry(&single.open, move |state, position, entry| {
            open_context_entry(state, depth, position, entry);
        });
        self.connect_open_in(&single.open_in);
        self.connect_open_in(&multiple.open_in);
        self.on_entry(&single.open_file_location, |state, _, entry| {
            if entry.is_directory() {
                return;
            }
            if let Some(parent) = entry.location.parent() {
                state.reveal_locations(parent, vec![entry.location], false);
            }
        });
        self.on_selection(&single.run, |state, entries| {
            if let [entry] = entries.as_slice()
                && crate::ui::browser::desktop::entry_is_regular_executable(entry)
            {
                crate::ui::browser::desktop::confirm_run_program(&entry.location, &state.overlay);
            }
        });
        self.on_open_with(&multiple.open, lookup, open_with_default_app);
        self.on_open_with(&single.open_with, lookup, show_open_with);
        self.on_open_with(&multiple.open_with, lookup, show_open_with);
        self.connect_preview(&single.preview);
        self.on_entry(&single.open_terminal, |state, _, entry| {
            launch_terminal(&entry.location, &state.overlay);
        });
    }

    fn connect_item_rows(&self, options: &ItemMenuOptions) {
        let single = &options.single;
        let depth = self.depth;
        self.on_entry(&single.pin, |state, _, entry| {
            if entry.is_directory()
                && let Some(handler) = state.pin_handler.borrow().as_ref()
            {
                handler(entry.location, entry.display_name, Box::new(|_| {}));
            }
        });
        self.on_entry(&single.copy_path, |_, _, entry| copy_locations(&[entry]));
        self.on_entry(&single.copy_name, |_, _, entry| copy_names(&[entry]));
        self.on_entry(&single.rename, move |state, position, entry| {
            let weak = Rc::downgrade(state);
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    rename_context_entry(&state, depth, position, entry);
                }
            });
        });
        self.on_entry(&single.properties, move |state, _, entry| {
            state.show_entry_properties_at(entry, depth);
        });
        self.on_selection(&options.multiple.properties, |state, entries| {
            state.show_selection_properties(entries);
        });
        self.on_selection(&options.multiple.copy_paths, |_, entries| {
            copy_locations(&entries);
        });
        self.on_selection(&options.multiple.copy_names, |_, entries| {
            copy_names(&entries)
        });
        self.on_entry(&single.customize, |state, _, entry| {
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
    }

    fn connect_creation_rows(&self, creation: &CreationOptions) {
        let depth = self.depth;
        for (button, is_directory) in [(&creation.new_folder, true), (&creation.new_file, false)] {
            self.on_entry(button, move |state, _, _| {
                if let Some(location) = state.browser.location_at(depth) {
                    schedule_new_entry(state, depth, location, is_directory);
                }
            });
        }
    }

    fn connect_selection_rows(&self, options: &ItemMenuOptions) {
        let (single_open, multiple_open) = (&options.single_open, &options.multiple_open);
        let (single, multiple) = (&options.single, &options.multiple);
        let depth = self.depth;
        for button in [&single_open.restore, &multiple_open.restore] {
            self.on_selection(button, move |state, entries| {
                restore_context_entries(state, depth, entries);
            });
        }
        for button in [
            &single_open.remove_from_recent,
            &multiple_open.remove_from_recent,
        ] {
            self.on_selection(button, |state, entries| {
                let uris = entries.into_iter().filter_map(|entry| entry.recent_uri);
                crate::adapters::recent_remove_entries(&state.recent_removal, uris);
            });
        }
        for (button, moving) in [
            (&single.move_to, true),
            (&single.copy_to, false),
            (&multiple.move_to, true),
            (&multiple.copy_to, false),
        ] {
            self.on_selection(button, move |state, entries| {
                state.show_transfer_dialog(entries, moving);
            });
        }
        for (button, moving) in [
            (&single.other_pane.moving, true),
            (&single.other_pane.copy, false),
            (&multiple.other_pane.moving, true),
            (&multiple.other_pane.copy, false),
        ] {
            self.on_selection(button, move |state, entries| {
                state.transfer_to_other_pane(entries, moving);
            });
        }
        self.connect_clipboard_rows(options);
        for button in [&single_open.group, &multiple_open.group] {
            self.on_selection(button, move |state, entries| {
                state.new_folder_with_selection(depth, &entries);
            });
        }
        self.connect_delete(&single.delete, options.in_trash);
        self.connect_delete(&multiple.delete, options.in_trash);
        for button in [&single.compress, &multiple.compress] {
            self.on_selection(button, |state, entries| {
                state.show_compress_dialog(entries);
            });
        }
        self.on_entry(&single_open.extract, |state, _, entry| {
            state.extract_entry(entry)
        });
        self.on_entry(&single_open.extract_to, |state, _, entry| {
            state.show_extract_to_dialog(entry);
        });
    }

    fn connect_clipboard_rows(&self, options: &ItemMenuOptions) {
        let (single, multiple) = (&options.single, &options.multiple);
        for button in [&single.cut, &multiple.cut] {
            self.on_selection(button, |state, entries| {
                state.cut_entries(&entries);
            });
        }
        for button in [&single.copy, &multiple.copy] {
            self.on_selection(button, |state, entries| {
                state.copy_entries(&entries);
            });
        }
        for button in [&single.duplicate, &multiple.duplicate] {
            self.on_selection(button, |state, entries| {
                state.duplicate_entries(&entries);
            });
        }
    }
}

fn open_context_entry(
    state: &Rc<ViewState>,
    depth: usize,
    position: Option<usize>,
    entry: FileEntry,
) {
    if let Some(position) = current_context_position(state, depth, position, &entry) {
        if state.mode_views.borrow().mode() == BrowserMode::Columns {
            state.activate_column_entry(depth, position);
        } else {
            state.browser.activate_in_place(depth, position);
        }
    } else if entry.is_directory() {
        state.browser.navigate(entry.location);
    } else {
        state.browser.open_location(entry.location);
    }
}

fn restore_context_entries(state: &Rc<ViewState>, depth: usize, entries: Vec<FileEntry>) {
    if let [entry] = entries.as_slice()
        && let Some(kind) = state.leftover_kind(depth, entry)
    {
        state.restore_leftover(depth, entry.clone(), kind);
    } else {
        state.request_restore(entries);
    }
}

fn open_entries_in_action(state: &Rc<ViewState>, entries: Vec<FileEntry>, action: &str) {
    for entry in entries {
        if !entry.is_directory() {
            continue;
        }
        let uri = gio_file_for_location(&entry.location).uri().to_variant();
        if let Err(error) = state.overlay.activate_action(action, Some(&uri)) {
            tracing::warn!(%error, action, "open-in-new location action unavailable");
        }
    }
}

fn open_with_default_app(state: &Rc<ViewState>, selection: OpenWithSelection) {
    if crate::ui::open_with::uses_portal() {
        crate::ui::open_with::launch_with_portal(&state.overlay, selection.files, false);
        return;
    }
    let Some(app) = selection.default else {
        return;
    };
    let context = crate::child_environment::launch_context(&state.overlay.display());
    if let Err(error) = crate::ui::open_with::launch(
        &app,
        &selection.files,
        &selection.content_types,
        Some(&context),
    ) {
        crate::ui::modal::show_error_dialog(
            &state.overlay,
            &crate::i18n::tr("Unable to open file"),
            error.message(),
        );
    }
}

fn show_open_with(state: &Rc<ViewState>, selection: OpenWithSelection) {
    let browser = Rc::downgrade(&state.browser);
    crate::ui::open_with::show(
        &state.overlay,
        selection.files,
        selection.content_types,
        selection.choices,
        crate::ui::open_with::OpenWithContext::Explicit,
        Rc::new(move || {
            if let Some(browser) = browser.upgrade() {
                browser.focus_active();
            }
        }),
    );
}

/// The item the menu opened on and the selection it applies to.
struct MenuTarget<'a> {
    state: &'a Rc<ViewState>,
    depth: usize,
    entry: &'a FileEntry,
    entries: &'a [FileEntry],
}

impl MenuTarget<'_> {
    fn removable(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| can_remove_location(&entry.location))
    }

    /// Whether the pane lists a real folder that new items can appear in.
    fn can_create_in_pane(&self) -> bool {
        !context_search_active(self.state, self.depth)
            && self
                .state
                .browser
                .location_at(self.depth)
                .is_some_and(|location| {
                    !is_trash_location(&location) && !location.is_recent_location()
                })
    }

    fn groupable(&self, removable: bool) -> bool {
        removable && self.can_create_in_pane()
    }

    fn shows_file_location(&self, in_trash: bool) -> bool {
        let entry = self.entry;
        if in_trash || entry.is_directory() {
            return false;
        }
        if entry.location.parent().is_none() {
            return false;
        }
        let in_different_folder =
            self.state.browser.location_at(self.depth).as_ref() != entry.location.parent().as_ref();
        context_filter_or_search_active(self.state, self.depth) || in_different_folder
    }

    fn recent_removable(&self, in_recent: bool) -> bool {
        in_recent
            && !self.entries.is_empty()
            && self.entries.iter().all(|entry| entry.recent_uri.is_some())
    }

    fn pin_available(&self) -> bool {
        self.state
            .pin_status_handler
            .borrow()
            .as_ref()
            .is_some_and(|handler| handler(&self.entry.location) == PinStatus::Available)
    }
}

impl ItemMenuOptions {
    fn open_with_buttons(&self) -> OpenWithButtons {
        OpenWithButtons {
            single: self.single_open.open_with.clone(),
            multiple: self.multiple_open.open_with.clone(),
            open: self.multiple_open.open.clone(),
        }
    }

    fn refresh_open_with(&self, target: &MenuTarget, lookup: &OpenWithLookup) {
        let entries = target.entries;
        // See `desktop::ADMINISTRATOR_OPEN_REFUSAL`.
        let other_apps = !entries
            .iter()
            .any(|entry| entry.location.is_administrator());
        let buttons = self.open_with_buttons();
        buttons.single.set_visible(other_apps && entries.len() == 1);
        buttons
            .multiple
            .set_visible(other_apps && entries.len() > 1);
        buttons.open.set_visible(false);
        for button in [&buttons.single, &buttons.multiple] {
            button.set_sensitive(false);
            set_open_with_explanation(button, Some("Looking for compatible applications…"));
        }
        let generation = lookup.restart();
        prepare_open_with(entries.to_vec(), &buttons, lookup, generation);
    }

    fn refresh_restore(&self, target: &MenuTarget) {
        let entries = target.entries;
        let restorable = entries.iter().all(|entry| is_trash_item(&entry.location));
        let leftover = entries.len() == 1
            && target
                .state
                .leftover_kind(target.depth, target.entry)
                .is_some();
        self.single_open.restore.set_visible(restorable || leftover);
        self.multiple_open.restore.set_visible(restorable);
        let recent_removable = target.recent_removable(self.in_recent);
        self.single_open
            .remove_from_recent
            .set_visible(recent_removable);
        self.multiple_open
            .remove_from_recent
            .set_visible(recent_removable);
    }

    fn refresh_open_rows(&self, target: &MenuTarget) {
        let (entry, entries) = (target.entry, target.entries);
        let single = &self.single_open;
        single
            .preview
            .set_visible(crate::ui::preview::entry_supports_quick_preview(entry));
        single.open_in.row.set_visible(entry.is_directory());
        self.multiple_open
            .open_in
            .row
            .set_visible(entries.len() > 1 && entries.iter().any(|entry| entry.is_directory()));
        single
            .open_terminal
            .set_visible(entry.is_directory() && can_open_terminal(&entry.location));
        single
            .open_file_location
            .set_visible(target.shows_file_location(self.in_trash));
        let can_extract = ArchiveFormat::for_entry(entry).is_some();
        single.extract.set_visible(can_extract);
        single.extract_to.set_visible(can_extract);
    }

    fn refresh_transfer_rows(&self, target: &MenuTarget, removable: bool) {
        for button in [
            &self.single.cut,
            &self.multiple.cut,
            &self.single.move_to,
            &self.multiple.move_to,
        ] {
            button.set_visible(removable);
        }
        let other_pane = target.state.other_pane_location().is_some();
        for options in [&self.single.other_pane, &self.multiple.other_pane] {
            options.copy.set_visible(other_pane);
            options.moving.set_visible(other_pane && removable);
        }
        let groupable = target.groupable(removable);
        self.single_open.group.set_visible(groupable);
        self.multiple_open.group.set_visible(groupable);
        self.single_open.group_separator.set_visible(groupable);
        self.multiple_open.group_separator.set_visible(groupable);
        self.single
            .rename
            .set_visible(!is_trash_location(&target.entry.location));
        let can_compress = target
            .entries
            .iter()
            .all(|entry| entry.location.native_path().is_some());
        self.single.compress.set_visible(can_compress);
        self.multiple.compress.set_visible(can_compress);
    }

    fn refresh_delete_rows(&self, target: &MenuTarget, removable: bool) {
        let browser = &target.state.browser;
        let trash_visible = removable
            && move_to_trash_is_visible(self.in_trash, browser.can_trash_at(target.depth));
        let permanent_visible =
            permanently_delete_is_visible(self.in_trash, browser.can_delete_at(target.depth));
        let remote = permanent_visible
            && crate::ui::browser::trash::deletion_reach(target.entries)
                == crate::ui::browser::trash::DeletionReach::Remote;
        for delete in [&self.single.delete, &self.multiple.delete] {
            delete.trash.set_visible(trash_visible);
            delete.permanent.set_visible(permanent_visible && !remote);
            delete.without_trash.set_visible(remote);
        }
    }

    fn refresh_item_rows(&self, target: &MenuTarget) {
        let entry = target.entry;
        self.single
            .pin
            .set_visible(entry.is_directory() && !is_trash_location(&entry.location));
        self.single.pin.set_sensitive(target.pin_available());
        self.single.customize.set_visible(
            !self.in_trash && target.entries.len() == 1 && entry.location.native_path().is_some(),
        );
    }

    fn refresh_header(&self, target: &MenuTarget) {
        let (entry, entries) = (target.entry, target.entries);
        let header = &self.header;
        let several = entries.len() > 1;
        if several {
            header.heading.set_text(&rust_i18n::t!(
                "%{items} selected",
                items = crate::i18n::count("items", entries.len())
            ));
            header.summary.set_text(&selected_items_summary(entries));
        } else {
            header.heading.set_text(&entry.display_name);
            header
                .summary
                .set_text(&compact_display_path(&entry.location));
        }
        self.single.container.set_visible(!several);
        self.multiple.container.set_visible(several);
    }

    fn refresh(&self, target: &MenuTarget, lookup: &OpenWithLookup) {
        self.single_open.run.set_visible(
            !self.in_trash
                && target.entries.len() == 1
                && crate::ui::browser::desktop::entry_is_regular_executable(target.entry),
        );
        self.creation
            .container
            .set_visible(target.can_create_in_pane());
        self.refresh_open_with(target, lookup);
        let removable = target.removable();
        self.refresh_restore(target);
        self.refresh_transfer_rows(target, removable);
        self.refresh_open_rows(target);
        self.refresh_delete_rows(target, removable);
        self.refresh_item_rows(target);
        self.refresh_header(target);
    }
}

fn mark_context_selection(popover: &gtk::Popover, widget: &gtk::Widget) {
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
}

fn location_path(state: &ViewState, depth: usize) -> Option<PathBuf> {
    state
        .browser
        .location_at(depth)
        .and_then(|location| location.native_path().map(PathBuf::from))
}

/// Resolves the row under a secondary click and opens the menu on it.
struct ItemMenuTrigger {
    widget: glib::WeakRef<gtk::Widget>,
    menu: ItemMenu,
    resolve: ContextResolver,
    options: ItemMenuOptions,
    action_section: Rc<actions::ActionMenuSection>,
    lookup: OpenWithLookup,
}

impl ItemMenuTrigger {
    fn open(&self, x: f64, y: f64, gesture: Option<&gtk::GestureClick>) -> bool {
        let Some(widget) = self.widget.upgrade() else {
            return false;
        };
        let Some(picked) = widget.pick(x, y, gtk::PickFlags::DEFAULT) else {
            return false;
        };
        let Some(state) = self.menu.state.upgrade() else {
            return false;
        };
        let Some((position, entry)) = (self.resolve)(&picked) else {
            return false;
        };
        let depth = self.menu.depth;
        focus_context_entry(&state, depth, position, &entry);
        self.menu.target.replace(Some((position, entry.clone())));
        let entries = context_entries(&state, &self.menu.target);
        self.action_section
            .rebuild_for_selection(&state, &entries, location_path(&state, depth));
        let target = MenuTarget {
            state: &state,
            depth,
            entry: &entry,
            entries: &entries,
        };
        self.options.refresh(&target, &self.lookup);
        // Claim before popup() changes GTK's grab and pointer-focus state.
        if let Some(gesture) = gesture {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
        self.action_section.show(&widget, x, y);
        true
    }
}

fn install_item_menu_click(widget: &gtk::Widget, open_at_resolved: ResolvedContextTrigger) {
    let click = gtk::GestureClick::new();
    click.set_button(3);
    // Claim secondary clicks before ListView's row gestures consume them.
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    click.connect_pressed(move |gesture, _, x, y| {
        open_at_resolved(x, y, Some(gesture));
    });
    widget.add_controller(click);
}

fn location_is(state: &ViewState, depth: usize, test: impl FnOnce(&Location) -> bool) -> bool {
    state.browser.location_at(depth).as_ref().is_some_and(test)
}

pub(in crate::ui) fn install_resolved_item_context_menu(
    state: &Rc<ViewState>,
    widget: &gtk::Widget,
    resolve: ContextResolver,
    depth: usize,
) -> Rc<dyn Fn(f64, f64)> {
    if !state.interactive {
        return chooser_context::install_item(state, widget, resolve, depth);
    }
    let options = ItemMenuOptions::build(
        location_is(state, depth, is_trash_location),
        location_is(state, depth, Location::is_recent_location),
    );
    let action_section = actions::ActionMenuSection::new(
        &options.content,
        &options.remaining,
        Some(options.header.header.upcast_ref()),
        state.overlay.upcast_ref(),
        Some([
            options.single.copy_to.clone(),
            options.multiple.copy_to.clone(),
        ]),
    );
    let popover = action_section.popover();
    bind_column_context_owner(state, &popover, depth);

    let menu = ItemMenu {
        state: Rc::downgrade(state),
        popover: popover.downgrade(),
        target: Rc::new(RefCell::new(None)),
        depth,
    };
    let lookup = OpenWithLookup::default();
    let closed_lookup = lookup.clone();
    popover.connect_closed(move |_| closed_lookup.invalidate());
    menu.connect_creation_rows(&options.creation);
    menu.connect_open_rows(&options, &lookup);
    menu.connect_item_rows(&options);
    menu.connect_selection_rows(&options);
    mark_context_selection(&popover, widget);

    let trigger = ItemMenuTrigger {
        widget: widget.downgrade(),
        menu,
        resolve,
        options,
        action_section,
        lookup,
    };
    let open_at_resolved: ResolvedContextTrigger =
        Rc::new(move |x: f64, y: f64, gesture: Option<&gtk::GestureClick>| {
            trigger.open(x, y, gesture)
        });

    install_secondary_release_retarget(&popover, widget, open_at_resolved.clone());

    let open_for_trigger = open_at_resolved.clone();
    let open_at: Rc<dyn Fn(f64, f64)> = Rc::new(move |x, y| {
        open_for_trigger(x, y, None);
    });
    install_item_menu_click(widget, open_at_resolved);
    open_at
}
