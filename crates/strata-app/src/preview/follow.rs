// SPDX-License-Identifier: MIT

//! What the preview follows: `Browser` focus, selection and removal events
//! (`PreviewDrawer::handle_browser_event` in `src/ui/preview/drawer.rs`), and the moves the
//! views make without the browser's cursor moving, which GTK's views raise `FocusChanged` for.

use rune::Tx;
use strata_core::model::FileEntry;
use strata_fs::app::{Browser, BrowserEvent, EntrySplice};

use super::{preview_target, session};
use crate::{App, filter, slots::Heard};

pub(super) fn hear(event: &BrowserEvent, browser: &Browser, heard: &mut Heard<'_, App>) {
    match event {
        BrowserEvent::PreviewRequested { entry, automatic } => {
            hear_request(entry.clone(), *automatic, browser, heard);
        }
        BrowserEvent::FocusChanged {
            depth,
            position: Some(position),
            ..
        }
        | BrowserEvent::SelectionSynced {
            depth,
            focused: Some(position),
        }
        | BrowserEvent::SelectionSetChanged {
            depth,
            focused: position,
            ..
        } => {
            let depth = *depth;
            let entry = browser.entry_at(depth, *position);
            heard.then(move |app: &mut App, tx: &mut Tx| follow_focus(app, depth, entry, tx));
        }
        BrowserEvent::FocusChanged {
            depth,
            position: None,
            ..
        }
        | BrowserEvent::SelectionSynced {
            depth,
            focused: None,
        } => {
            let depth = *depth;
            heard.then(move |app: &mut App, tx: &mut Tx| follow_focus(app, depth, None, tx));
        }
        BrowserEvent::LoadFinished { depth, .. } | BrowserEvent::LoadFailed { depth, .. } => {
            let depth = *depth;
            heard.then(move |app: &mut App, _: &mut Tx| {
                session::click_opened(app, depth);
                false
            });
        }
        BrowserEvent::EntriesSpliced { depth, splices } => hear_splices(*depth, splices, heard),
        _ => {}
    }
}

fn hear_request(entry: FileEntry, automatic: bool, browser: &Browser, heard: &mut Heard<'_, App>) {
    let active_depth = browser.active_depth();
    heard.then(move |app: &mut App, tx: &mut Tx| {
        if automatic && app.preview.dismissed {
            return false;
        }
        session::show(app, entry, active_depth, tx);
        true
    });
}

fn hear_splices(depth: usize, splices: &[EntrySplice], heard: &mut Heard<'_, App>) {
    if !splices.iter().any(|s: &EntrySplice| s.removed > 0) {
        return;
    }
    heard.then(move |app: &mut App, tx: &mut Tx| follow_removal(app, depth, tx));
}

/// The preview follows `entry`, focused at `depth`: it shows a file it can preview, and
/// otherwise clears, lending its slot to a folder's column.
pub(super) fn follow_focus(
    app: &mut App,
    depth: usize,
    entry: Option<FileEntry>,
    tx: &mut Tx,
) -> bool {
    if !app.preview.enabled || session::in_band(app) {
        return false;
    }
    if let Some(target) = preview_target(entry.clone()) {
        session::show_after_focus_change(app, target, Some(depth), tx);
    } else {
        session::lend_slot_to_child(app, depth, entry.as_ref());
        session::clear_target(app, tx);
    }
    true
}

fn follow_removal(app: &mut App, depth: usize, tx: &mut Tx) -> bool {
    if !app.preview.enabled || app.preview.current_depth != Some(depth) {
        return false;
    }
    let Some(current) = app.preview.current.clone() else {
        return false;
    };
    let still_present = (0..)
        .map_while(|pos| app.browser.entry_at(depth, pos))
        .any(|entry| entry.location == current.location);
    if !still_present {
        session::clear_target(app, tx);
        return true;
    }
    false
}

/// Updates an already-open preview when the user moves the selection among filtered results.
pub(crate) fn follow_filter_pick(app: &mut App) {
    if !app.preview.enabled {
        return;
    }
    let later = app.preview.loader.later.clone();
    later.then(
        |app: &mut App, tx: &mut Tx| match filter::targets::preview_result(app) {
            Some((depth, entry)) => follow_focus(app, depth, Some(entry), tx),
            None => false,
        },
    );
}

/// The Columns view moved the keyboard to another column itself (Left, Right into an open
/// column, a click on a column's header, peek or reopened folder), so the browser's cursor did
/// not move: GTK's `focus_parent`, `focus_child` and `focus_active` raise `FocusChanged` there,
/// and the preview follows the active column's cursor.
pub(crate) fn follow_active_column(app: &mut App, tx: &mut Tx) {
    let Some(depth) = app.browser.active_depth() else {
        return;
    };
    let entry = app.browser.cursor_entry(depth);
    follow_focus(app, depth, entry, tx);
}

/// A click in Columns is opening the folder at `position` of column `depth`: until its column
/// has loaded, the focus on it and in its column keeps the preview's slot (GTK's
/// `TabLocationHold::opening`, which `lend_slot_to_child` reads). A folder whose column is
/// open already loads nothing, and GTK's hold ends with the click.
pub(crate) fn note_click_opening(app: &mut App, depth: usize, position: usize) {
    app.preview.click_opening = app
        .browser
        .entry_at(depth, position)
        .filter(FileEntry::is_directory)
        .map(|entry| entry.location)
        .filter(|folder| !app.browser.is_open_child(depth, folder))
        .map(|folder| super::ClickOpening { depth, folder });
}

/// A closed filter puts the listing's cursor back without the browser's moving: the preview
/// leaves the result it showed for that cursor, as GTK's does.
pub(crate) fn follow_filter_close(app: &mut App) {
    if !app.preview.enabled {
        return;
    }
    let later = app.preview.loader.later.clone();
    later.then(|app: &mut App, tx: &mut Tx| {
        let (depth, entry) = session::selected_entry(app);
        follow_focus(app, depth.unwrap_or(0), entry, tx)
    });
}
