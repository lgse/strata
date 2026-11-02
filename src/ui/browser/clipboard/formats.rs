// SPDX-License-Identifier: MIT

//! The clipboard formats Linux file managers exchange for copied files.
//!
//! GTK 4 readers take a `FileList`, KDE takes `text/uri-list` with its own cut
//! flag, and Nautilus, Nemo, Caja, Thunar and the GNOME desktop-icon
//! extensions paste only from the GNOME format. Offering all of them at once
//! lets every file manager paste what Strata copied, and reading them back
//! keeps a cut made elsewhere a move.

use crate::adapters::location_for_file;
use crate::model::Location;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

pub(crate) const URI_LIST: &str = "text/uri-list";
/// A `cut` or `copy` line followed by one URI per line, no trailing newline.
pub(crate) const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
/// `1` marks the accompanying `text/uri-list` as cut, `0` as copied.
pub(crate) const KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";

const READ_CHUNK: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClipboardFiles {
    pub(crate) sources: Vec<Location>,
    pub(crate) cut: bool,
}

pub(crate) fn file_content(files: &[gio::File], cut: bool) -> gdk::ContentProvider {
    let file_list = gdk::ContentProvider::for_value(&gdk::FileList::from_array(files).to_value());
    gdk::ContentProvider::new_union(&[
        file_list,
        uri_list_provider(files),
        bytes_provider(GNOME_COPIED_FILES, gnome_copied_files(files, cut)),
        bytes_provider(KDE_CUT_SELECTION, kde_cut_selection(cut)),
    ])
}

pub(crate) fn uri_list_provider(files: &[gio::File]) -> gdk::ContentProvider {
    bytes_provider(URI_LIST, uri_list(files))
}

pub(crate) fn offers_files(formats: &gdk::ContentFormats) -> bool {
    formats.contains_type(gdk::FileList::static_type())
        || formats.contain_mime_type(URI_LIST)
        || formats.contain_mime_type(GNOME_COPIED_FILES)
}

pub(crate) async fn read_files(clipboard: &gdk::Clipboard) -> Option<ClipboardFiles> {
    let formats = clipboard.formats();
    if formats.contain_mime_type(GNOME_COPIED_FILES)
        && let Some(files) = read_mime_bytes(clipboard, GNOME_COPIED_FILES)
            .await
            .and_then(|payload| parse_gnome_copied_files(&payload))
    {
        return Some(files);
    }
    let value = clipboard
        .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
        .await
        .ok()?;
    let sources = super::locations_from_file_list_value(&value)?;
    let cut = formats.contain_mime_type(KDE_CUT_SELECTION)
        && read_mime_bytes(clipboard, KDE_CUT_SELECTION)
            .await
            .is_some_and(|payload| kde_cut(&payload));
    Some(ClipboardFiles { sources, cut })
}

pub(crate) async fn read_mime_bytes(clipboard: &gdk::Clipboard, mime: &str) -> Option<Vec<u8>> {
    let (stream, _) = clipboard
        .read_future(&[mime], glib::Priority::DEFAULT)
        .await
        .ok()?;
    let mut payload = Vec::new();
    loop {
        let chunk = stream
            .read_bytes_future(READ_CHUNK, glib::Priority::DEFAULT)
            .await
            .ok()?;
        if chunk.is_empty() {
            return Some(payload);
        }
        payload.extend_from_slice(&chunk);
    }
}

pub(crate) fn parse_gnome_copied_files(payload: &[u8]) -> Option<ClipboardFiles> {
    let text = std::str::from_utf8(payload).ok()?;
    let mut lines = text.lines();
    let cut = match lines.next()? {
        "cut" => true,
        "copy" => false,
        _ => return None,
    };
    let sources = lines
        .filter(|line| !line.is_empty())
        .filter_map(|uri| location_for_file(&gio::File::for_uri(uri)))
        .collect::<Vec<_>>();
    (!sources.is_empty()).then_some(ClipboardFiles { sources, cut })
}

pub(crate) fn kde_cut(payload: &[u8]) -> bool {
    payload.first() == Some(&b'1')
}

pub(crate) fn gnome_copied_files(files: &[gio::File], cut: bool) -> String {
    let mut payload = String::from(if cut { "cut" } else { "copy" });
    for file in files {
        payload.push('\n');
        payload.push_str(&file.uri());
    }
    payload
}

fn kde_cut_selection(cut: bool) -> String {
    String::from(if cut { "1" } else { "0" })
}

pub(crate) fn uri_list(files: &[gio::File]) -> String {
    let mut payload = String::with_capacity(files.len() * 48);
    for file in files {
        payload.push_str(&file.uri());
        payload.push_str("\r\n");
    }
    payload
}

fn bytes_provider(mime: &str, payload: String) -> gdk::ContentProvider {
    gdk::ContentProvider::for_bytes(mime, &glib::Bytes::from_owned(payload.into_bytes()))
}

#[cfg(test)]
mod tests;
