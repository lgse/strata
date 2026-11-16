// SPDX-License-Identifier: MIT

//! Preview session state transitions: Space toggle, Appearance panel toggle, 75 ms focus
//! debounce, empty-selection placeholders, and child-column lending in Columns.
//!
//! Mirrors GTK's `src/ui/preview/session.rs` and `src/ui/preview/lifecycle.rs`.

use std::time::Duration;

use rune::Tx;
use strata_components::preview_drawer::HeaderInfo;
use strata_core::model::FileEntry;

use super::{load, preview_target, viewer::Viewer};
use crate::{App, filter, shell};

const FOCUS_PREVIEW_DELAY: Duration = Duration::from_millis(75);

pub(super) fn browsing_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

pub(super) fn browsing_icons(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("icons"))
}

pub(super) fn in_band(app: &App) -> bool {
    if app.filter.pane.in_band() {
        return true;
    }
    if browsing_columns(app) {
        app.columns.in_band()
    } else if browsing_icons(app) {
        app.icons.single.in_band()
    } else {
        app.list.single.in_band()
    }
}

pub(super) fn selected_entry(app: &App) -> (Option<usize>, Option<FileEntry>) {
    if let Some((depth, entry)) = filter::targets::preview_result(app) {
        return (Some(depth), Some(entry));
    }
    if let Some((depth, _, entry)) = app.browser.focused_item() {
        return (Some(depth), Some(entry));
    }
    (app.browser.active_depth(), app.browser.focused_entry())
}

pub(super) fn cancel_focus_timer(app: &mut App) {
    if let Some(timer) = app.preview.focus_timer.borrow_mut().take() {
        timer.remove();
    }
}

pub(super) fn toggle_panel(
    app: &mut App,
    entry: Option<FileEntry>,
    depth: Option<usize>,
    tx: &mut Tx,
) {
    let active = app.preview.enabled || app.preview.reserves_column_space(app);
    app.preview.reserve_columns = !active;
    if active {
        app.preview.dismissed = true;
        close(app, tx);
    } else {
        toggle(app, entry, depth, tx);
    }
}

pub(super) fn toggle(app: &mut App, entry: Option<FileEntry>, depth: Option<usize>, tx: &mut Tx) {
    if app.preview.enabled {
        close(app, tx);
        return;
    }
    app.preview.reserve_columns = true;
    app.preview.dismissed = false;
    app.preview.enabled = true;
    let folder = entry.as_ref().is_some_and(FileEntry::is_directory);
    if let Some(entry) = preview_target(entry) {
        show(app, entry, depth, tx);
    } else {
        app.preview.child_pane = folder && browsing_columns(app);
        clear_target(app, tx);
    }
}

pub(super) fn show(app: &mut App, entry: FileEntry, depth: Option<usize>, tx: &mut Tx) {
    let other_file = app
        .preview
        .current
        .as_ref()
        .is_none_or(|current| current.location != entry.location);
    if other_file {
        app.preview.drawer.reset_find();
        app.preview.viewer.stop();
        app.preview.viewer = Viewer::Empty;
        app.preview.content_owns_focus = 0;
    }
    cancel_focus_timer(app);
    let was_open = app.preview.drawer.is_open();
    let already_showing = app.preview.current.as_ref() == Some(&entry)
        && (app.preview.loader.is_active() || !app.preview.viewer.is_empty());
    app.preview.current_depth = depth;
    app.preview.current = Some(entry.clone());
    app.preview.reserve_columns = true;
    app.preview.dismissed = false;
    app.preview.child_pane = false;
    app.preview.enabled = true;
    sync_reservation(app);
    app.preview.drawer.open(&app.look, tx);
    if !was_open && browsing_columns(app) {
        app.columns.reveal_active(&app.look, tx);
    }
    if !was_open || !already_showing {
        load::start(app, entry);
    }
}

pub(super) fn show_after_focus_change(
    app: &mut App,
    entry: FileEntry,
    depth: Option<usize>,
    tx: &mut Tx,
) {
    cancel_focus_timer(app);
    let already_showing = app.preview.current.as_ref() == Some(&entry)
        && (app.preview.loader.is_active() || !app.preview.viewer.is_empty());
    if already_showing {
        return;
    }
    if !app.preview.drawer.is_open() {
        show(app, entry, depth, tx);
        return;
    }
    // Keep the displayed target during the 75 ms focus debounce so split sizing does not treat
    // rapid cursor navigation as a closed drawer.
    app.preview.loader.cancel();
    let later = app.preview.loader.later.clone();
    let slot = app.preview.focus_timer.clone();
    let source = glib::timeout_add_local_once(FOCUS_PREVIEW_DELAY, move || {
        slot.borrow_mut().take();
        later.then(move |app: &mut App, tx: &mut Tx| {
            if app.preview.enabled {
                show(app, entry, depth, tx);
                return true;
            }
            false
        });
    });
    app.preview.focus_timer.replace(Some(source));
}

/// Mirroring also raises focus for the child beyond the active depth. A folder a click is
/// opening becomes the focused column, so lending to it, or to its column, would only flicker
/// (`lend_slot_to_child` and `pointer_opens` in `src/ui/preview/lifecycle.rs`).
pub(super) fn lend_slot_to_child(app: &mut App, depth: usize, entry: Option<&FileEntry>) {
    let active = app.browser.active_depth();
    let opening = click_opens(app, depth, entry);
    app.preview.child_pane = browsing_columns(app)
        && !opening
        && (entry.is_some_and(FileEntry::is_directory) || active.is_some_and(|a| depth > a));
}

/// Whether the focus at `depth` on `entry` is the folder a click is opening, or its column's.
fn click_opens(app: &App, depth: usize, entry: Option<&FileEntry>) -> bool {
    let Some(folder) = app.preview.click_opening.as_ref() else {
        return false;
    };
    match entry {
        Some(entry) => &entry.location == folder,
        None => app.browser.location_at(depth).as_ref() == Some(folder),
    }
}

/// The click's folder is open once its column has loaded, or failed to.
pub(super) fn click_opened(app: &mut App, depth: usize) {
    if app.preview.click_opening.as_ref() == app.browser.location_at(depth).as_ref() {
        app.preview.click_opening = None;
    }
}

/// The view shown changed with no file previewed. A view switch lends no slot it did not
/// (`view_mode_changed` in `src/ui/preview/layout.rs`); another tab's folder may want it lent.
pub(super) fn show_empty_slot_after_switch(app: &mut App, tab_shown: bool, tx: &mut Tx) {
    let (_, entry) = selected_entry(app);
    let lend = browsing_columns(app) && entry.as_ref().is_some_and(FileEntry::is_directory);
    app.preview.child_pane = lend && (tab_shown || app.preview.child_pane);
    if app.preview.reserves_empty_preview(app) {
        app.preview.drawer.open(&app.look, tx);
    } else {
        app.preview.drawer.close(&app.look, tx);
    }
}

pub(super) fn clear_target(app: &mut App, tx: &mut Tx) {
    cancel_focus_timer(app);
    app.preview.loader.cancel();
    app.preview.raw.clear();
    app.preview.drawer.reset_find();
    app.preview.current = None;
    app.preview.current_depth = None;
    app.preview.header_source = None;
    app.preview.content_owns_focus = 0;
    app.preview.viewer.stop();
    app.preview.viewer = Viewer::Empty;
    app.preview.header = HeaderInfo::default();
    sync_reservation(app);
    if app.preview.reserves_empty_preview(app) {
        app.preview.drawer.open(&app.look, tx);
    } else {
        app.preview.drawer.close(&app.look, tx);
    }
}

pub(super) fn close(app: &mut App, tx: &mut Tx) {
    if app.preview.enabled {
        app.preview.dismissed = true;
    }
    app.preview.enabled = false;
    app.preview.child_pane = false;
    clear_target(app, tx);
}

pub(super) fn sync_reservation(app: &mut App) {
    let reserved = app.preview.reserves_column_space(app);
    app.preview.drawer.reserve(reserved);
}
