// SPDX-License-Identifier: MIT

//! Drops of content rather than files, from another application: images, selected text and
//! web addresses, saved as new files in local folders (`src/ui/browser/content_drop.rs`).
//! Nothing here touches the network: an address becomes a link file, never a download.
//!
//! What a drop holds, in GTK's order:
//!
//! - **Files** (a `text/uri-list` of local or remote files, or this process's drag) go to the
//!   transfer, as before: not content. A list mixing files with web addresses is refused.
//! - **A browser's file contents** (`application/octet-stream;name="…"`) are saved under the
//!   name it gives, when they are an image; else what else the drop offers is tried.
//! - **Image data** (`image/*`) is saved as a PNG named for a pasted image.
//! - **Web addresses** (a list of them only) become `.desktop` links.
//! - **Text** becomes a link when it is one address, else a text file.
//!
//! Image contents that are unreadable, with nothing else to fall back on, say why instead, as
//! does a read past its bound ("The dropped content is too large"). A drop reads only the
//! types this order needs, one at a time (`classify`).

use std::path::PathBuf;

use rune::Tx;
use strata_components::contract::dnd::{DropOffer, DropZone, URI_LIST};
use strata_core::{i18n::tr, model::Location};
use strata_fs::services::{DroppedFile, io_error_message, save_dropped_files};

use crate::{
    App,
    error_dialog::{self, Error},
    shell, transfer,
};

mod classify;
mod decode;
#[cfg(test)]
mod tests;

pub(crate) use classify::{Got, Reads, content_zone_takes, next_read, read};

/// A browser's dragged image bytes, with `;name="…"` (GTK has no reader for them).
const FILE_CONTENTS: &str = "application/octet-stream";

/// What every zone that takes files takes from another app besides, most wanted first: the
/// host's content-drops policy, as GTK's drop targets read file lists, textures and strings.
/// `text/plain` takes it in any charset.
pub(crate) const CONTENT_TYPES: &[&str] = &[
    FILE_CONTENTS,
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/bmp",
    "image/tiff",
    "image/webp",
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
];

/// What a drop that holds no files saves.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Content {
    /// The files to save.
    Save(Vec<DroppedFile>),
    /// Content taken but not readable, with the reason to show.
    Unreadable(String),
    /// Nothing to save: a list mixing files with addresses, or no content at all.
    Refused,
}

/// Whether `offer` is content rather than files: from another app, offering no file list but
/// a type of [`CONTENT_TYPES`]. A file list that holds only web addresses is content too, but
/// that shows only once its URIs are read.
pub(crate) fn offered(offer: &DropOffer) -> bool {
    !offer.local && !offer.offers(URI_LIST) && CONTENT_TYPES.iter().any(|mime| offer.offers(mime))
}

/// Whether `zone` takes content besides files ([`DropZone::takes_content`]): its drops are
/// read one type at a time ([`next_read`]).
pub(crate) fn zone_takes<S: 'static>(zone: &DropZone<S>) -> bool {
    CONTENT_TYPES.iter().any(|mime| zone.reads(mime))
}

/// Whether `zone` lists exactly what the policy reads: the file list, then [`CONTENT_TYPES`],
/// as `DropZone::takes_content(CONTENT_TYPES)` makes it.
pub(crate) fn lists_the_policy<S: 'static>(zone: &DropZone<S>) -> bool {
    let listed = zone.preferred_mime_types().iter().map(AsRef::as_ref);
    listed.eq(std::iter::once(URI_LIST).chain(CONTENT_TYPES.iter().copied()))
}

/// Whether a drop of `content` onto `destination` is taken, so what runs before a drop commits
/// (a tab drop selects its tab) should run.
pub(crate) fn takes(destination: &Location, content: &Content) -> bool {
    match content {
        Content::Refused => false,
        Content::Unreadable(_) => true,
        Content::Save(_) => destination.native_path().is_some(),
    }
}

/// Saves `content` dropped on `destination`, a local folder, under names no entry has; then
/// shows the saved files there when "Open folder after drop" is on, unless the user went
/// elsewhere meanwhile, or says why nothing was saved. Returns whether the drop was taken.
pub(crate) fn commit(app: &mut App, destination: Location, content: Content, tx: &mut Tx) -> bool {
    match content {
        Content::Refused => false,
        Content::Unreadable(reason) => {
            unsaved(app, reason);
            true
        }
        Content::Save(files) => {
            let Some(directory) = destination.native_path().map(PathBuf::from) else {
                return false;
            };
            save(app, (destination, directory), files);
            shell::settle(app, tx);
            true
        }
    }
}

fn save(app: &App, (destination, directory): (Location, PathBuf), files: Vec<DroppedFile>) {
    let reveal = shell::preferences(app).open_folder_after_drop;
    let navigation = app.browser.navigation_generation();
    let later = transfer::later(app);
    glib::MainContext::default().spawn_local(async move {
        let saved = gio::spawn_blocking(move || save_dropped_files(&directory, &files))
            .await
            .unwrap_or_else(|_| Err(std::io::Error::other("the save was interrupted")));
        later.then(move |app, tx| {
            match saved {
                // Navigating away while saving outranks the preference.
                Ok(paths) if reveal && app.browser.navigation_generation() == navigation => {
                    shell::publish(app);
                    let locations = paths.into_iter().map(Location::local).collect();
                    transfer::reveal::apply(app, destination, locations, tx);
                }
                Ok(_) => {}
                Err(error) => unsaved(app, io_error_message(&error)),
            }
            shell::settle(app, tx);
            true
        });
    });
}

fn unsaved(app: &mut App, reason: String) {
    error_dialog::show(
        app,
        Error::new(tr("Unable to save dropped content"), reason),
    );
}
