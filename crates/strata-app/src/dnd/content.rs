// SPDX-License-Identifier: MIT

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

const FILE_CONTENTS: &str = "application/octet-stream";

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

pub(crate) fn offered(offer: &DropOffer) -> bool {
    !offer.local && !offer.offers(URI_LIST) && CONTENT_TYPES.iter().any(|mime| offer.offers(mime))
}

pub(crate) fn zone_takes<S: 'static>(zone: &DropZone<S>) -> bool {
    CONTENT_TYPES.iter().any(|mime| zone.reads(mime))
}

pub(crate) fn lists_the_policy<S: 'static>(zone: &DropZone<S>) -> bool {
    let listed = zone.preferred_mime_types().iter().map(AsRef::as_ref);
    listed.eq(std::iter::once(URI_LIST).chain(CONTENT_TYPES.iter().copied()))
}

pub(crate) fn takes(destination: &Location, content: &Content) -> bool {
    match content {
        Content::Refused => false,
        Content::Unreadable(_) => true,
        Content::Save(_) => destination.native_path().is_some(),
    }
}

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
