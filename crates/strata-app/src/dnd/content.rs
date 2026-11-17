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
//! Image contents that are unreadable, with nothing else to fall back on, say why instead.

use std::{io::Cursor, path::PathBuf};

use rune::Tx;
use strata_components::contract::dnd::{DropOffer, Dropped, URI_LIST, names};
use strata_core::{i18n::tr, model::Location};
use strata_fs::services::{
    DroppedFile, MAX_DROPPED_FILE_BYTES, WebLink, has_web_scheme, io_error_message,
    save_dropped_files,
};

use crate::{
    App,
    error_dialog::{self, Error},
    shell, transfer,
};

#[cfg(test)]
mod tests;

/// A browser's dragged image bytes, with `;name="…"` (GTK has no reader for them).
const FILE_CONTENTS: &str = "application/octet-stream";

/// What every zone that takes files takes from another app besides, most wanted first: the
/// host's content-drops policy, as GTK's drop targets read file lists, textures and strings.
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

/// The types that carry text, most wanted first.
const TEXT_TYPES: &[&str] = &["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

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

/// What `dropped` saves, or `None` when it holds files, which go to the transfer.
pub(crate) fn read(dropped: &Dropped) -> Option<Content> {
    if dropped.data.local.is_some() {
        return None;
    }
    let uris = dropped.data.uris();
    let links = match web_links(&uris) {
        WebLinks::Files => return None,
        WebLinks::Refused => return Some(Content::Refused),
        WebLinks::Links(links) => links,
    };
    Some(saved(dropped, &links))
}

enum WebLinks {
    /// A file list, or none: `uris` empty gives no links.
    Links(Vec<WebLink>),
    /// Files only: the transfer's.
    Files,
    /// Addresses mixed with files, or one that cannot be a link.
    Refused,
}

fn web_links(uris: &[String]) -> WebLinks {
    if uris.is_empty() {
        return WebLinks::Links(Vec::new());
    }
    if !uris.iter().any(|uri| has_web_scheme(uri)) {
        return WebLinks::Files;
    }
    uris.iter()
        .map(|uri| WebLink::parse(uri))
        .collect::<Option<Vec<_>>>()
        .map_or(WebLinks::Refused, WebLinks::Links)
}

/// The image's own bytes, then its decoded data, then the addresses or text offered with it,
/// reporting the image's failure when nothing else is there (`DeferredContent::read`).
fn saved(dropped: &Dropped, links: &[WebLink]) -> Content {
    let mut failure = None;
    for read in [file_contents, image_data] {
        match read(dropped) {
            Some(Ok(file)) => return Content::Save(vec![file]),
            Some(Err(reason)) => {
                failure.get_or_insert(reason);
            }
            None => {}
        }
    }
    if !links.is_empty() {
        return Content::Save(links.iter().map(DroppedFile::link).collect());
    }
    if let Some(file) = text(dropped) {
        return Content::Save(vec![file]);
    }
    failure.map_or(Content::Refused, Content::Unreadable)
}

/// The first item whose type one of `listed` names, if within the size bound.
fn item<'a>(dropped: &'a Dropped, listed: &[&str]) -> Option<(&'a str, Result<&'a [u8], String>)> {
    let item = listed.iter().find_map(|mime| {
        dropped
            .data
            .items
            .iter()
            .find(|item| names(mime, &item.mime))
    })?;
    let bytes = if item.data.len() > MAX_DROPPED_FILE_BYTES {
        Err(tr("The dropped content is too large"))
    } else {
        Ok(item.data.as_slice())
    };
    Some((&item.mime, bytes))
}

fn file_contents(dropped: &Dropped) -> Option<Result<DroppedFile, String>> {
    let (mime, bytes) = item(dropped, &[FILE_CONTENTS])?;
    let name = supplied_name(mime);
    Some(bytes.and_then(|bytes| {
        DroppedFile::image(name.as_deref(), &tr("pasted_image.name"), bytes.to_vec())
            .ok_or_else(|| tr("Unsupported image format"))
    }))
}

fn image_data(dropped: &Dropped) -> Option<Result<DroppedFile, String>> {
    let images: Vec<&str> = CONTENT_TYPES
        .iter()
        .copied()
        .filter(|mime| mime.starts_with("image/"))
        .collect();
    let (_, bytes) = item(dropped, &images)?;
    Some(bytes.and_then(|bytes| {
        png_of(bytes)
            .map(|png| DroppedFile::png(&tr("pasted_image.name"), png))
            .ok_or_else(|| tr("Unsupported image format"))
    }))
}

fn text(dropped: &Dropped) -> Option<DroppedFile> {
    let (_, bytes) = item(dropped, TEXT_TYPES)?;
    let text = String::from_utf8_lossy(bytes.ok()?).into_owned();
    if text.is_empty() {
        return None;
    }
    Some(match WebLink::parse(&text) {
        Some(link) => DroppedFile::link(&link),
        None => DroppedFile::text(&tr("dropped_text.name"), text),
    })
}

/// An image's data as a PNG, as GTK saves a dropped texture.
fn png_of(bytes: &[u8]) -> Option<Vec<u8>> {
    if image::guess_format(bytes).ok()? == image::ImageFormat::Png {
        return Some(bytes.to_vec());
    }
    let decoded = image::load_from_memory(bytes).ok()?;
    let mut png = Cursor::new(Vec::new());
    decoded.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

/// The name in `application/octet-stream;name="…"`, its escapes undone.
fn supplied_name(mime: &str) -> Option<String> {
    let quoted = mime
        .strip_prefix(FILE_CONTENTS)?
        .strip_prefix(";name=\"")?
        .strip_suffix('"')?;
    let mut name = String::with_capacity(quoted.len());
    let mut escaped = false;
    for c in quoted.chars() {
        if escaped || c != '\\' {
            name.push(c);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    Some(name)
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
