// SPDX-License-Identifier: MIT

//! The floating destination chooser that the entry menu's "Copy to…", "Move to…" and
//! "Extract to…" open (`present_destination_chooser` in `src/ui/chooser/destination.rs`,
//! `show_transfer_dialog` in `src/ui/browser/transfer/destination_dialog.rs` and
//! `show_extract_to_dialog` in `src/ui/browser/archive.rs`).
//!
//! The window asking for one queues a [`FloatingRequest`] here. The host takes it ([`take`])
//! and opens the chooser as a toplevel of its own, over an `App` of its own: the folder chooser
//! of the portal, choosing one existing folder. Its answer comes back through the asking
//! window's [`Later`], which then transfers or extracts into the folder chosen.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use rune::Tx;
use strata_core::{
    i18n::tr,
    model::{ArchiveFormat, FileEntry, Location},
};
use strata_fs::{adapters::probes, services::LocationValidationError};

use super::{
    source::ChooserFileSource,
    state::{DestinationHost, PendingValidation},
};
use crate::{
    App,
    clipboard::paths,
    commands::{Call, On, Outcome},
    menus,
    portal::{ChooserKind, ChooserRequest, Completion, PortalError, SelectedResult},
    slots::Later,
};

/// A floating chooser for the host to open.
pub(crate) struct FloatingRequest {
    pub(crate) request: ChooserRequest,
    pub(crate) source: Rc<ChooserFileSource>,
    pub(crate) destination: DestinationHost,
    pub(crate) completion: Completion,
}

/// What the asking window keeps of its floating chooser.
#[derive(Default)]
pub(crate) struct Floating {
    pending: Option<FloatingRequest>,
    /// A chooser is open over the window: GTK keeps one per window (`DestinationLease`).
    open: bool,
    opened: u64,
}

/// What the folder chosen is for.
enum Destined {
    Transfer {
        sources: Vec<Location>,
        move_sources: bool,
    },
    Extract(Box<FileEntry>),
}

impl Destined {
    fn finish(self, app: &mut App, folder: PathBuf, tx: &mut Tx) {
        let destination = Location::local(folder);
        match self {
            Self::Transfer {
                sources,
                move_sources,
            } => crate::transfer::transfer_to_chosen(app, destination, sources, move_sources),
            Self::Extract(entry) => crate::archive::extract_to_chosen(app, *entry, destination, tx),
        }
    }
}

/// The floating chooser the window asked for, once.
pub(crate) fn take(app: &mut App) -> Option<FloatingRequest> {
    app.chooser.floating.pending.take()
}

/// Whether the window asked for a floating chooser that no host has taken yet.
#[cfg(feature = "qa")]
pub(crate) fn has_pending(app: &App) -> bool {
    app.chooser.floating.pending.is_some()
}

/// "Copy to…" on the entry menu.
pub(crate) fn copy_to(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    transfer_to(app, call, false)
}

/// "Move to…" on the entry menu.
pub(crate) fn move_to(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    transfer_to(app, call, true)
}

/// "Extract to…" on an archive's entry menu.
pub(crate) fn extract_to(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let On::Row((depth, id)) = call.on else {
        return Outcome::Declined;
    };
    let archive = menus::context_entries(app, depth, id)
        .map(|(entry, _)| entry)
        .filter(|entry| ArchiveFormat::for_entry(entry).is_some());
    let Some(entry) = archive.filter(|entry| entry.location.native_path().is_some()) else {
        return Outcome::Ran;
    };
    let base = entry
        .location
        .parent()
        .and_then(|parent| parent.native_path().map(Path::to_path_buf))
        .unwrap_or_else(crate::launch::home_directory);
    let labels = (tr("Extract to"), tr("Extract here"));
    open(app, labels, base, Destined::Extract(Box::new(entry)));
    Outcome::Ran
}

fn transfer_to(app: &mut App, call: &Call<'_>, move_sources: bool) -> Outcome {
    let On::Row((depth, id)) = call.on else {
        return Outcome::Declined;
    };
    let entries = menus::context_entries(app, depth, id)
        .map(|(_, entries)| entries)
        .unwrap_or_default();
    let sources: Vec<Location> = entries.into_iter().map(|entry| entry.location).collect();
    if !transferable(&sources, move_sources) {
        return Outcome::Ran;
    }
    let base = app
        .browser
        .active_location()
        .and_then(|location| location.native_path().map(Path::to_path_buf))
        .unwrap_or_else(crate::launch::home_directory);
    let labels = if move_sources {
        (tr("Move to"), tr("Move here"))
    } else {
        (tr("Copy to"), tr("Copy here"))
    };
    let destined = Destined::Transfer {
        sources,
        move_sources,
    };
    open(app, labels, base, destined);
    Outcome::Ran
}

/// Something to transfer, and, for a move, nothing GVfs cannot remove from where it is.
fn transferable(sources: &[Location], move_sources: bool) -> bool {
    !sources.is_empty() && (!move_sources || sources.iter().all(paths::can_remove))
}

fn open(app: &mut App, (title, accept_label): (String, String), base: PathBuf, destined: Destined) {
    let floating = &mut app.chooser.floating;
    if floating.open {
        return;
    }
    floating.open = true;
    floating.opened += 1;
    let request = ChooserRequest {
        token: format!("floating-{}", floating.opened),
        title,
        accept_label,
        modal: true,
        parent: None,
        parent_size_hint: None,
        initial_directory: base,
        kind: ChooserKind::Open {
            directory: true,
            multiple: false,
        },
        filters: Vec::new(),
        current_filter: None,
        choices: Vec::new(),
    };
    let completion = answered(app.chooser.later.clone(), destined);
    floating.pending = Some(FloatingRequest {
        request,
        source: ChooserFileSource::new(None),
        destination: DestinationHost {
            allow_create: true,
            confined: false,
            validate: Rc::new(existing_folder),
        },
        completion,
    });
}

/// The chooser's answer, applied by the asking window.
fn answered(later: Later<App>, destined: Destined) -> Completion {
    Box::new(move |result| {
        let folder = chosen_folder(result);
        later.then(move |app: &mut App, tx: &mut Tx| {
            app.chooser.floating.open = false;
            if let Some(folder) = folder {
                destined.finish(app, folder, tx);
            }
            true
        });
    })
}

/// The folder the chooser answered with; none when it was cancelled.
fn chosen_folder(result: Result<SelectedResult, PortalError>) -> Option<PathBuf> {
    let uri = result.ok()?.uris.into_iter().next()?;
    gio::prelude::FileExt::path(&gio::File::for_uri(&uri))
}

/// Accepts an existing folder only (`existing_folder` in `src/ui/browser/destination.rs`).
fn existing_folder(path: &Path) -> PendingValidation {
    let path = path.to_path_buf();
    Box::pin(async move {
        match probes::destination_folder(&path).await {
            Ok(_) => Ok(path),
            Err(LocationValidationError::Missing | LocationValidationError::NotDirectory) => {
                Err(tr("Choose an existing folder."))
            }
            Err(error) => Err(error.message()),
        }
    })
}
