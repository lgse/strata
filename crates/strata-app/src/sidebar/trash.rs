// SPDX-License-Identifier: MIT

//! What the Trash holds, as GTK's sidebar probes it (`refresh_trash_contents` and
//! `watch_trash` in `src/ui/window/sidebar_trash.rs`): whether `trash:///` has one item. The
//! Trash menu offers "Empty Trash…" only once it is known to hold something
//! (`trash_menu_visibility`).
//!
//! - **When.** As the sidebar starts, whenever its monitor of the Trash hears a change, after
//!   the browser's operations that can fill or drain it (`event_changes_trash_contents`), and
//!   as the Trash's menu opens. A probe asked for while one runs runs again after it: the
//!   running one may have read the Trash before the change.
//! - **An open Trash menu** takes the answer at once.
//!
//! GTK's sidebar draws the Trash row with one icon, full or not, so this interface does too.

use gio::prelude::*;
use strata_core::model::{LoadHandle, Location};
use strata_fs::{
    adapters::{Watched, gio_file_for_location, watch},
    app::BrowserEvent,
};

use super::menu;
use crate::{App, slots::Later, trash};

/// `TrashContents`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Contents {
    /// Not probed yet, or the probe failed.
    #[default]
    Unknown,
    Empty,
    NonEmpty,
}

/// The probe's state.
#[derive(Default)]
pub(super) struct Probe {
    pub(super) contents: Contents,
    running: bool,
    pending: bool,
    /// The monitor of the Trash, held while the sidebar shows.
    watch: Option<LoadHandle>,
}

/// `event_changes_trash_contents`.
pub(super) fn changes_contents(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::DeletionFinished { .. }
            | BrowserEvent::RestorationFinished { .. }
            | BrowserEvent::TransferFinished { .. }
            | BrowserEvent::OperationCompletedWithErrors { .. }
            | BrowserEvent::OperationCancelled { .. }
    )
}

/// Starts watching the Trash, and probes it.
pub(super) fn start(app: &mut App) {
    let later = app.sidebar.later.clone();
    let (changed, live) = (later.clone(), later);
    let handle = watch(
        &trash::empty::root(),
        Watched::Folder,
        move |_, _| changed.then(|app: &mut App, _| refresh(app)),
        move || live.then(|app: &mut App, _| refresh(app)),
    );
    app.sidebar.trash.watch = Some(handle);
    refresh(app);
}

/// `refresh_trash_contents`. Returns false: the answer, not the asking, changes what shows.
pub(super) fn refresh(app: &mut App) -> bool {
    let probe = &mut app.sidebar.trash;
    if probe.running {
        probe.pending = true;
        return false;
    }
    probe.running = true;
    let later = app.sidebar.later.clone();
    let root = trash::empty::root();
    glib::MainContext::default().spawn_local(async move {
        let found = has_items(&root).await;
        if let Err(error) = &found {
            tracing::warn!(%error, "unable to read trash contents");
        }
        answer(&later, found);
    });
    false
}

fn answer(later: &Later<App>, found: Result<bool, glib::Error>) {
    let contents = match found {
        Ok(false) => Contents::Empty,
        Ok(true) => Contents::NonEmpty,
        Err(_) => Contents::Unknown,
    };
    later.then(move |app: &mut App, _| {
        let probe = &mut app.sidebar.trash;
        probe.running = false;
        let changed = probe.contents != contents;
        probe.contents = contents;
        if std::mem::take(&mut probe.pending) {
            refresh(app);
        }
        if changed {
            menu::contents_changed(app);
        }
        changed
    })
}

/// `trash_has_items`: whether the Trash lists one child.
async fn has_items(root: &Location) -> Result<bool, glib::Error> {
    let enumerator = gio_file_for_location(root)
        .enumerate_children_future(
            gio::FILE_ATTRIBUTE_STANDARD_NAME,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let first = enumerator
        .next_files_future(1, glib::Priority::DEFAULT)
        .await?;
    Ok(!first.is_empty())
}
