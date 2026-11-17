// SPDX-License-Identifier: MIT

//! What a drop from another app holds, in GTK's order, and which of its types to read for it.
//!
//! One walk decides both: while the drop is read ([`next_read`]), the first type it needs and
//! has not read yet is the next to read; once read ([`read`]), what it saves. GTK reads only
//! the type it uses and falls back where `DeferredContent::read` does: from image bytes that
//! are no image, or image data that cannot be read, to the addresses or text offered with
//! them. So a drop reads one type of each kind at most, in that order, and stops at the first
//! that settles it.

use strata_components::contract::dnd::{Dropped, ReadFailure, URI_LIST, parse_uri_list};
use strata_core::{error_text::UNEXPECTED_SYSTEM_ERROR, i18n::tr};
use strata_fs::services::{
    DroppedFile, MAX_DROPPED_FILE_BYTES, WebLink, has_web_scheme, io_error_message,
};

use super::{Content, FILE_CONTENTS, decode};

/// What a drop holds of one offered type, as far as it was read.
pub(crate) enum Got<'a> {
    Bytes(&'a [u8]),
    Failed(ReadFailure),
    /// Not read yet: it may still be.
    Unread,
    /// Not read, and it will not be.
    Absent,
}

/// A drop's offered types and what was read of them.
pub(crate) trait Reads {
    /// The types offered that the zone takes, in the source's order.
    fn offered(&self) -> Vec<&str>;
    /// What was read of the offered type `mime`.
    fn got(&self, mime: &str) -> Got<'_>;
    /// The drag comes from this process.
    fn local(&self) -> bool;
}

impl Reads for Dropped {
    fn offered(&self) -> Vec<&str> {
        self.offer.mime_types.iter().map(AsRef::as_ref).collect()
    }

    fn got(&self, mime: &str) -> Got<'_> {
        let item = self.data.items.iter().find(|item| item.mime == mime);
        let failed = self.data.failed.iter().find(|(m, _)| m == mime);
        match (item, failed) {
            (Some(item), _) if item.data.len() > MAX_DROPPED_FILE_BYTES => {
                Got::Failed(ReadFailure::TooLarge)
            }
            (Some(item), _) => Got::Bytes(&item.data),
            (None, Some((_, failure))) => Got::Failed(failure.clone()),
            (None, None) => Got::Absent,
        }
    }

    fn local(&self) -> bool {
        self.data.local.is_some()
    }
}

/// What `dropped` saves, or `None` when it holds files, which go to the transfer.
pub(crate) fn read(dropped: &Dropped) -> Option<Content> {
    classify(dropped).ok().flatten()
}

/// The offered type to read next for what `reads` holds, or `None` when what it read settles
/// the drop (P1's `DropTarget::read_next`).
pub(crate) fn next_read(reads: &impl Reads) -> Option<String> {
    classify(reads).err()
}

/// A type the walk needs read before it can go on.
type Needs = String;

/// What the drop saves (`None` for files), or the type to read before that is known.
fn classify(reads: &impl Reads) -> Result<Option<Content>, Needs> {
    if reads.local() {
        return Ok(None);
    }
    let offered = reads.offered();
    let mut walk = Walk {
        reads,
        failure: None,
    };
    let links = match walk.file_list(&offered)? {
        WebLinks::Files => return Ok(None),
        WebLinks::Refused => return Ok(Some(Content::Refused)),
        WebLinks::Links(links) => links,
    };
    for image in [Walk::file_contents, Walk::image_data] {
        if let Some(file) = image(&mut walk, &offered)? {
            return Ok(Some(Content::Save(vec![file])));
        }
    }
    if !links.is_empty() {
        return Ok(Some(Content::Save(
            links.iter().map(DroppedFile::link).collect(),
        )));
    }
    if let Some(file) = walk.text(&offered)? {
        return Ok(Some(Content::Save(vec![file])));
    }
    Ok(Some(
        walk.failure.map_or(Content::Refused, Content::Unreadable),
    ))
}

enum WebLinks {
    /// Web addresses only, or no file list: none gives no links.
    Links(Vec<WebLink>),
    /// Files only: the transfer's.
    Files,
    /// Addresses mixed with files, or one that cannot be a link.
    Refused,
}

/// A walk through a drop's types, keeping the first failure to report when nothing else is
/// there.
struct Walk<'a, R> {
    reads: &'a R,
    failure: Option<String>,
}

impl<'a, R: Reads> Walk<'a, R> {
    /// The bytes of the first offered type `wanted` takes, `None` when none is offered or its
    /// read failed (noted, if `report` says so), or the type to read first.
    fn bytes(
        &mut self,
        offered: &[&str],
        wanted: impl Fn(&str) -> bool,
        report: impl Fn(&ReadFailure) -> bool,
    ) -> Result<Option<(String, &'a [u8])>, Needs> {
        let Some(&mime) = offered.iter().find(|m| wanted(m)) else {
            return Ok(None);
        };
        match self.reads.got(mime) {
            Got::Bytes(bytes) => Ok(Some((mime.to_owned(), bytes))),
            Got::Unread => Err(mime.to_owned()),
            Got::Absent => Ok(None),
            Got::Failed(failure) => {
                if report(&failure) {
                    self.fail(reason(&failure));
                }
                Ok(None)
            }
        }
    }

    fn fail(&mut self, reason: String) {
        self.failure.get_or_insert(reason);
    }

    /// GTK reads a file list silently: only a list past the read's bound, which GTK has not,
    /// is reported.
    fn file_list(&mut self, offered: &[&str]) -> Result<WebLinks, Needs> {
        let list = self.bytes(offered, |m| m == URI_LIST, too_large)?;
        Ok(list.map_or(WebLinks::Links(Vec::new()), |(_, bytes)| {
            web_links(&parse_uri_list(bytes))
        }))
    }

    /// A browser's image bytes, under the name it gives.
    fn file_contents(&mut self, offered: &[&str]) -> Result<Option<DroppedFile>, Needs> {
        let Some((mime, bytes)) = self.bytes(offered, is_file_contents, any)? else {
            return Ok(None);
        };
        let name = decode::supplied_name(&mime);
        let image = DroppedFile::image(name.as_deref(), &tr("pasted_image.name"), bytes.to_vec());
        if image.is_none() {
            self.fail(tr("Unsupported image format"));
        }
        Ok(image)
    }

    /// Image data, saved as a PNG: the first image type offered, as GDK reads one texture.
    fn image_data(&mut self, offered: &[&str]) -> Result<Option<DroppedFile>, Needs> {
        let Some((_, bytes)) = self.bytes(offered, |m| m.starts_with("image/"), any)? else {
            return Ok(None);
        };
        let png = decode::png_of(bytes).map(|png| DroppedFile::png(&tr("pasted_image.name"), png));
        if png.is_none() {
            self.fail(tr("Unsupported image format"));
        }
        Ok(png)
    }

    /// The first text type offered, in its charset: a link when it is one address, else a
    /// text file. GTK reads one string and reports nothing when it is no use.
    fn text(&mut self, offered: &[&str]) -> Result<Option<DroppedFile>, Needs> {
        let Some((mime, bytes)) = self.bytes(offered, decode::is_text, too_large)? else {
            return Ok(None);
        };
        let text = decode::text(&mime, bytes).filter(|text| !text.is_empty());
        Ok(text.map(|text| match WebLink::parse(&text) {
            Some(link) => DroppedFile::link(&link),
            None => DroppedFile::text(&tr("dropped_text.name"), text),
        }))
    }
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

/// `application/octet-stream`, bare or with the name a browser gives (`file_contents_mime`).
fn is_file_contents(mime: &str) -> bool {
    mime.strip_prefix(FILE_CONTENTS)
        .is_some_and(|parameters| parameters.is_empty() || parameters.starts_with(";name=\""))
}

fn any(_: &ReadFailure) -> bool {
    true
}

fn too_large(failure: &ReadFailure) -> bool {
    *failure == ReadFailure::TooLarge
}

/// Why a read failed, to show: GTK's "too large", a timeout, or a system error.
fn reason(failure: &ReadFailure) -> String {
    match failure {
        ReadFailure::TooLarge => tr("The dropped content is too large"),
        ReadFailure::TimedOut => io_error_message(&std::io::ErrorKind::TimedOut.into()),
        ReadFailure::Failed(error) => {
            tracing::warn!(%error, "unable to read dropped content");
            tr(UNEXPECTED_SYSTEM_ERROR)
        }
    }
}
