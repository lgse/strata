// SPDX-License-Identifier: GPL-3.0-or-later

use crate::adapters::gio_file_for_location;
use crate::adapters::location_for_file;
use crate::model::{FileEntry, Location};
use crate::ui::browser::ViewState;
use crate::ui::browser::columns::set_cut_path_style;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
use std::rc::{Rc, Weak};

pub(super) fn install_directory_drop_target(
    state: &Rc<ViewState>,
    widget: &impl IsA<gtk::Widget>,
    destination: Location,
) {
    widget.add_css_class("file-drop-zone");
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );
    drop.connect_enter(|target, _, _| file_drop_action(target));
    drop.connect_motion(|target, _, _| file_drop_action(target));
    let weak = Rc::downgrade(state);
    drop.connect_drop(move |target, value, _, _| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        transfer_dropped_files(&state, target, value, destination.clone())
    });
    widget.add_controller(drop);
}

fn transfer_dropped_files(
    state: &Rc<ViewState>,
    target: &gtk::DropTarget,
    value: &glib::Value,
    destination: Location,
) -> bool {
    let Some(sources) = locations_from_file_list_value(value) else {
        return false;
    };
    if sources.is_empty() {
        return false;
    }
    let move_sources = file_drop_action(target) == gtk::gdk::DragAction::MOVE;
    state.start_transfer(destination, sources, move_sources);
    true
}

pub(crate) fn file_drop_action(target: &gtk::DropTarget) -> gtk::gdk::DragAction {
    let Some(drop) = target.current_drop() else {
        return gtk::gdk::DragAction::empty();
    };
    preferred_file_drop_action(drop.actions(), drop.drag().is_some())
}

fn preferred_file_drop_action(actions: gtk::gdk::DragAction, local: bool) -> gtk::gdk::DragAction {
    if actions.contains(gtk::gdk::DragAction::MOVE)
        && (local || !actions.contains(gtk::gdk::DragAction::COPY))
    {
        gtk::gdk::DragAction::MOVE
    } else if actions.contains(gtk::gdk::DragAction::COPY) {
        gtk::gdk::DragAction::COPY
    } else {
        gtk::gdk::DragAction::empty()
    }
}

pub(crate) fn locations_from_file_list_value(value: &glib::Value) -> Option<Vec<Location>> {
    let files = value.get::<gtk::gdk::FileList>().ok()?;
    let locations = files
        .files()
        .iter()
        .filter_map(location_for_file)
        .collect::<Vec<_>>();
    (!locations.is_empty()).then_some(locations)
}

pub(in crate::ui) fn file_drag_content(entries: &[FileEntry]) -> Option<gtk::gdk::ContentProvider> {
    let files = entries
        .iter()
        .map(|entry| gio_file_for_location(&entry.location))
        .collect::<Vec<_>>();
    if files.is_empty() {
        return None;
    }
    let file_list =
        gtk::gdk::ContentProvider::for_value(&gtk::gdk::FileList::from_array(&files).to_value());
    let uri_list = files
        .iter()
        .map(|file| file.uri())
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n";
    let uri_list = gtk::gdk::ContentProvider::for_bytes(
        "text/uri-list",
        &glib::Bytes::from_owned(uri_list.into_bytes()),
    );
    Some(gtk::gdk::ContentProvider::new_union(&[file_list, uri_list]))
}

pub(super) fn copy_locations(entries: &[FileEntry]) {
    let text = entries
        .iter()
        .map(|entry| copy_path_text(&entry.location, entry.is_directory()))
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(&text);
    }
}

pub(super) fn copy_path_text(location: &Location, is_directory: bool) -> String {
    match location.native_path() {
        Some(path) => {
            let mut path = shell_escape_path(path);
            if is_directory && !path.ends_with(std::path::MAIN_SEPARATOR) {
                path.push(std::path::MAIN_SEPARATOR);
            }
            path
        }
        None => location.display_path(),
    }
}

fn shell_escape_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if path.contains('\n') {
        return format!("'{}'", path.replace('\'', "'\\''"));
    }

    let mut escaped = String::new();
    for c in path.chars() {
        if needs_shell_escape(c) {
            escaped.push('\\');
            escaped.push(c);
        } else {
            escaped.push(c);
        }
    }
    escaped
}

fn needs_shell_escape(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '"' | '\''
                | '\\'
                | '$'
                | '`'
                | '!'
                | '#'
                | '&'
                | '*'
                | ';'
                | '<'
                | '>'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '('
                | ')'
                | '|'
                | '~'
        )
}

// Process-wide cut intent shared by every window. The GDK clipboard only
// carries a `FileList` with no cut marker, so this thread-local (GTK stays on
// the main thread) is the source of truth for both paste behavior and styling.
thread_local! {
    static SHARED_CUT_LOCATIONS: RefCell<Vec<Location>> = const { RefCell::new(Vec::new()) };
    static CUT_VIEWS: RefCell<Vec<Weak<ViewState>>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn register_cut_view(state: &Rc<ViewState>) {
    CUT_VIEWS.with(|views| views.borrow_mut().push(Rc::downgrade(state)));
    state.refresh_cut_rows();
}

fn refresh_cut_views() {
    let views = CUT_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let live = views.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
        views.retain(|view| view.strong_count() > 0);
        live
    });
    for view in views {
        view.refresh_cut_rows();
    }
}

pub(super) fn shared_cut_locations() -> Vec<Location> {
    SHARED_CUT_LOCATIONS.with(|cut| cut.borrow().clone())
}

fn set_shared_cut(locations: &[Location]) {
    SHARED_CUT_LOCATIONS.with(|cut| cut.replace(locations.to_vec()));
    refresh_cut_views();
}

fn clear_shared_cut() {
    SHARED_CUT_LOCATIONS.with(|cut| cut.borrow_mut().clear());
    refresh_cut_views();
}

fn retain_shared_untransferred(transferred: &[Location]) {
    SHARED_CUT_LOCATIONS.with(|cut| retain_untransferred(&mut cut.borrow_mut(), transferred));
    refresh_cut_views();
}

fn is_cut_match(sources: &[Location]) -> bool {
    same_locations(sources, &shared_cut_locations())
}

fn set_files_clipboard(entries: &[FileEntry]) -> bool {
    set_location_files_clipboard(
        &entries
            .iter()
            .map(|entry| entry.location.clone())
            .collect::<Vec<_>>(),
    )
}

fn set_location_files_clipboard(locations: &[Location]) -> bool {
    let files = locations
        .iter()
        .map(gio_file_for_location)
        .collect::<Vec<_>>();
    if files.is_empty() {
        return false;
    }
    gtk::gdk::Display::default().is_some_and(|display| {
        display
            .clipboard()
            .set_content(Some(&gtk::gdk::ContentProvider::for_value(
                &gtk::gdk::FileList::from_array(&files).to_value(),
            )))
            .is_ok()
    })
}

/// Location equality that also accepts GIO-level equivalence (URI
/// normalization, `file://` vs native path for the same file). Mounts such as
/// NFS can round-trip through the clipboard with a different but equivalent
/// representation, and strict `PathBuf` equality alone would degrade a cut to
/// a copy.
pub(super) fn locations_equal(left: &Location, right: &Location) -> bool {
    left == right || gio_file_for_location(left).equal(&gio_file_for_location(right))
}

fn same_locations(left: &[Location], right: &[Location]) -> bool {
    if left.is_empty() || left.len() != right.len() {
        return false;
    }
    let left_set: HashSet<_> = left.iter().collect();
    let right_set: HashSet<_> = right.iter().collect();
    if left_set.len() == right_set.len() && left_set == right_set {
        return true;
    }
    let mut used = vec![false; right.len()];
    left.iter().all(|location| {
        let Some((index, _)) = right
            .iter()
            .enumerate()
            .find(|(index, candidate)| !used[*index] && locations_equal(location, candidate))
        else {
            return false;
        };
        used[index] = true;
        true
    })
}

fn retain_untransferred(cut: &mut Vec<Location>, transferred: &[Location]) {
    cut.retain(|location| {
        !transferred
            .iter()
            .any(|moved| locations_equal(location, moved))
    });
}

impl ViewState {
    pub(super) fn copy_entries(&self, entries: &[FileEntry]) {
        if set_files_clipboard(entries) {
            self.clear_cut();
        }
    }

    pub(super) fn cut_entries(&self, entries: &[FileEntry]) {
        if set_files_clipboard(entries) {
            let locations: Vec<Location> =
                entries.iter().map(|entry| entry.location.clone()).collect();
            set_shared_cut(&locations);
        }
    }

    fn clear_cut(&self) {
        clear_shared_cut();
    }

    pub(super) fn complete_cut_transfer(&self, transferred: &[Location]) {
        retain_shared_untransferred(transferred);
        let remaining = shared_cut_locations();
        if remaining.is_empty() {
            if let Some(display) = gtk::gdk::Display::default() {
                let _result = display
                    .clipboard()
                    .set_content(None::<&gtk::gdk::ContentProvider>);
            }
        } else {
            let _set = set_location_files_clipboard(&remaining);
        }
    }

    fn refresh_cut_rows(&self) {
        let cut = shared_cut_locations();
        self.mode_views.borrow().set_cut_locations(&cut);
        let cut_lookup: HashSet<_> = cut.iter().collect();
        for (depth, column) in self.columns.borrow().iter().enumerate() {
            column.bound_rows.borrow_mut().retain(|bound| {
                let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                    return false;
                };
                let is_cut = column
                    .map
                    .source_position(item.position())
                    .and_then(|position| self.browser.entry_at(depth, position))
                    .is_some_and(|entry| cut_lookup.contains(&entry.location));
                set_cut_path_style(&row, is_cut);
                true
            });
        }
    }

    pub(super) fn paste_into(self: &Rc<Self>, destination: Location) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let clipboard = display.clipboard();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = clipboard
                .read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT)
                .await;
            let files = match result {
                Ok(value) => match value.get::<gtk::gdk::FileList>() {
                    Ok(files) => files.files(),
                    Err(_) => return,
                },
                Err(_) => return,
            };
            let sources = files
                .into_iter()
                .filter_map(|file| location_for_file(&file))
                .collect::<Vec<_>>();
            if let Some(state) = weak.upgrade() {
                let move_sources = is_cut_match(&sources);
                state.start_transfer(destination, sources, move_sources);
            }
        });
    }
}

#[cfg(test)]
mod tests;
