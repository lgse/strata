//! What a copy offers: any MIME types, each with its bytes or a lazy writer.

use std::time::Duration;

use crate::transfer::{SourceSpec, Writer, mime};

use super::files::Files;

/// The names text goes by: `text/plain;charset=utf-8` and `text/plain` for Wayland and newer
/// X11 apps, `UTF8_STRING` for X11 ones, and ICCCM's `TEXT` and `STRING` (ISO Latin-1).
pub const TEXT_TYPES: &[&str] = &[
    mime::TEXT_UTF8,
    "UTF8_STRING",
    "text/plain",
    "TEXT",
    "STRING",
];

/// How long the app's exit waits for a clipboard manager to take a copy, by default (X11).
pub const KEEP_AFTER_EXIT: Duration = Duration::from_secs(2);

/// What a copy offers: MIME types (or X11 target names), in order, each with its bytes or a
/// lazy writer. A type offered twice keeps its first data.
///
/// A lazy writer runs on a worker thread when an app first asks for its type, at most once
/// per copy; its bytes are kept for the copy's life (Nautilus asks six times for one paste).
/// It may run late, or never, and it can't see the app's state: give it what it needs.
///
/// ```ignore
/// Content::new()
///     .files(&Files::from_paths(&selected, cut))
///     .text(&paths_as_text)
///     .lazy("image/png", move || encode_png(&pixels))
/// ```
#[derive(Clone)]
pub struct Content {
    spec: SourceSpec,
    keep: Option<Duration>,
    /// The first X11 meta target asked for, which no copy can offer.
    reserved: Option<String>,
}

impl Default for Content {
    fn default() -> Self {
        Content::new()
    }
}

impl Content {
    pub fn new() -> Self {
        Content {
            spec: SourceSpec::new(),
            keep: Some(KEEP_AFTER_EXIT),
            reserved: None,
        }
    }

    /// Adds `mime` with bytes at hand. An X11 target that names a request, not a data type
    /// (`TARGETS`, `SAVE_TARGETS`, `DELETE`…), can't be offered: [`Clipboard::copy`] of such a
    /// content is refused.
    ///
    /// [`Clipboard::copy`]: super::Clipboard::copy
    pub fn with(self, mime: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        self.add(mime.into(), Writer::bytes(bytes))
    }

    /// Adds `mime`, made by `make` when first asked for (see the type's docs).
    pub fn lazy(
        self,
        mime: impl Into<String>,
        make: impl FnOnce() -> Vec<u8> + Send + 'static,
    ) -> Self {
        self.add(mime.into(), Writer::lazy(make))
    }

    fn add(mut self, mime: String, writer: Writer) -> Self {
        if mime::is_x11_meta(&mime) {
            self.reserved.get_or_insert(mime);
        } else {
            self.spec = self.spec.with(mime, writer);
        }
        self
    }

    /// Adds text under every name in [`TEXT_TYPES`]. `TEXT` and `STRING` are ISO Latin-1, so
    /// they are offered only when the text fits it.
    pub fn text(self, text: &str) -> Self {
        let utf8 = text.as_bytes().to_vec();
        let mut c = self
            .with(mime::TEXT_UTF8, utf8.clone())
            .with("UTF8_STRING", utf8.clone())
            .with("text/plain", utf8);
        if let Some(latin1) = latin1(text) {
            c = c.with("TEXT", latin1.clone()).with("STRING", latin1);
        }
        c
    }

    /// Adds a PNG image as `image/png`.
    pub fn png(self, png: impl Into<Vec<u8>>) -> Self {
        self.with("image/png", png)
    }

    /// Adds Rune's clipboard image (a pasted one, say) as `image/png`.
    pub fn image(self, image: &rune::ClipboardImage) -> Self {
        self.png(image.png())
    }

    /// Adds a file list in the formats file managers exchange ([`Files`]).
    pub fn files(self, files: &Files) -> Self {
        files.add_to(self)
    }

    /// How long the app's exit waits for a clipboard manager to take this copy, every type of
    /// it, when it still holds the clipboard then (X11; see the module docs). `None`: don't
    /// hand it over, so it goes with the app. [`KEEP_AFTER_EXIT`] by default.
    pub fn keep_after_exit(mut self, wait: Option<Duration>) -> Self {
        self.keep = wait;
        self
    }

    /// The types offered, in order.
    pub fn mimes(&self) -> impl Iterator<Item = &str> {
        self.spec.mimes()
    }

    pub fn is_empty(&self) -> bool {
        self.spec.is_empty()
    }

    pub(super) fn into_parts(self) -> (SourceSpec, Option<Duration>) {
        (self.spec, self.keep)
    }

    /// The first X11 meta target asked for, if any.
    pub(super) fn reserved(&self) -> Option<&str> {
        self.reserved.as_deref()
    }
}

/// `text` in ISO Latin-1, if every character is in it.
fn latin1(text: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).ok())
        .collect()
}

#[cfg(test)]
mod tests;
