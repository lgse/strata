// SPDX-License-Identifier: MIT

use strata_components::contract::dnd::{Dropped, ReadFailure, URI_LIST, names, parse_uri_list};
use strata_core::{error_text::UNEXPECTED_SYSTEM_ERROR, i18n::tr};
use strata_fs::services::{DroppedFile, WebLink, has_web_scheme, io_error_message};

use super::{CONTENT_TYPES, Content, FILE_CONTENTS, decode};

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

/// Whether a content zone takes the offered type `mime`.
pub(crate) fn content_zone_takes(mime: &str) -> bool {
    std::iter::once(URI_LIST)
        .chain(CONTENT_TYPES.iter().copied())
        .any(|listed| names(listed, mime))
}

impl Reads for Dropped {
    fn offered(&self) -> Vec<&str> {
        let types = self.offer.mime_types.iter().map(AsRef::as_ref);
        types.filter(|mime| content_zone_takes(mime)).collect()
    }

    fn got(&self, mime: &str) -> Got<'_> {
        if let Some(item) = self.data.items.iter().find(|item| item.mime == mime) {
            return Got::Bytes(&item.data);
        }
        match self.data.failed.iter().find(|(m, _)| m == mime) {
            Some((_, failure)) => Got::Failed(failure.clone()),
            None => Got::Absent,
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

struct Walk<'a, R> {
    reads: &'a R,
    failure: Option<String>,
}

impl<'a, R: Reads> Walk<'a, R> {
    fn bytes(
        &mut self,
        offered: &[&str],
        wanted: impl Fn(&str) -> bool,
        report: impl Fn(&ReadFailure) -> bool,
    ) -> Result<Option<(String, &'a [u8])>, Needs> {
        for &mime in offered.iter().filter(|m| wanted(m)) {
            match self.reads.got(mime) {
                Got::Bytes(bytes) => return Ok(Some((mime.to_owned(), bytes))),
                Got::Unread => return Err(mime.to_owned()),
                Got::Absent => {}
                Got::Failed(failure) => {
                    if report(&failure) {
                        self.fail(reason(&failure));
                    }
                    return Ok(None);
                }
            }
        }
        Ok(None)
    }

    fn fail(&mut self, reason: String) {
        self.failure.get_or_insert(reason);
    }

    fn file_list(&mut self, offered: &[&str]) -> Result<WebLinks, Needs> {
        let list = self.bytes(offered, |m| m == URI_LIST, too_large)?;
        Ok(list.map_or(WebLinks::Links(Vec::new()), |(_, bytes)| {
            web_links(&parse_uri_list(bytes))
        }))
    }

    /// A browser's image bytes, under the name it gives.
    fn file_contents(&mut self, offered: &[&str]) -> Result<Option<DroppedFile>, Needs> {
        self.image(offered, is_file_contents, |mime, bytes| {
            let name = decode::supplied_name(mime);
            DroppedFile::image(name.as_deref(), &tr("pasted_image.name"), bytes.to_vec())
        })
    }

    /// Image data, saved as a PNG: the first image type offered, as GDK reads one texture.
    fn image_data(&mut self, offered: &[&str]) -> Result<Option<DroppedFile>, Needs> {
        self.image(
            offered,
            |m| m.starts_with("image/"),
            |_, bytes| {
                decode::png_of(bytes).map(|png| DroppedFile::png(&tr("pasted_image.name"), png))
            },
        )
    }

    fn image(
        &mut self,
        offered: &[&str],
        wanted: impl Fn(&str) -> bool,
        save: impl FnOnce(&str, &[u8]) -> Option<DroppedFile>,
    ) -> Result<Option<DroppedFile>, Needs> {
        let Some((mime, bytes)) = self.bytes(offered, wanted, any)? else {
            return Ok(None);
        };
        let image = save(&mime, bytes);
        if image.is_none() {
            self.fail(tr("Unsupported image format"));
        }
        Ok(image)
    }

    fn text(&mut self, offered: &[&str]) -> Result<Option<DroppedFile>, Needs> {
        let mut forms: Vec<&str> = offered
            .iter()
            .copied()
            .filter(|m| decode::is_text(m))
            .collect();
        while let Some((mime, bytes)) = self.bytes(&forms, |_| true, too_large)? {
            if let Some(text) = decode::text(&mime, bytes).filter(|text| !text.is_empty()) {
                return Ok(Some(match WebLink::parse(&text) {
                    Some(link) => DroppedFile::link(&link),
                    None => DroppedFile::text(&tr("dropped_text.name"), text),
                }));
            }
            forms.retain(|form| *form != mime);
        }
        Ok(None)
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
