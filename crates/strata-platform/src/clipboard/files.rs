//! The formats Linux file managers exchange for copied and cut files, written and read
//! together: `text/uri-list` (RFC 2483, which GTK readers and Dolphin take),
//! `x-special/gnome-copied-files` (Nautilus, Nemo, Caja, Thunar and the GNOME desktop
//! extensions paste only from it) and `application/x-kde-cutselection` (Dolphin's cut flag).
//!
//! The encoders write what GTK writes, byte for byte: `file://` URIs escaped as GLib
//! escapes them (a path that isn't UTF-8 still gets one), each `text/uri-list` line ending in
//! CRLF, the GNOME payload `copy` or `cut` and then one URI per line with no trailing newline,
//! and the KDE flag `1` for a cut, `0` for a copy.
//!
//! Reading follows GTK and Nautilus: a GNOME payload whose first line is `cut` or `copy`
//! decides; without a valid one, `text/uri-list` decides, cut when the KDE flag is `1`.
//! CRLF, a trailing newline and NUL padding read the same as plain newlines.
//!
//! Not here: the portal types (`application/vnd.portal.filetransfer`,
//! `application/vnd.portal.files`), through which sandboxed (Flatpak) apps exchange files.
//! They need the Documents portal over D-Bus; a follow-up.

use std::path::{Path, PathBuf};

use crate::transfer::mime;
pub use crate::transfer::mime::{
    FileUri, GNOME_COPIED_FILES, KDE_CUT_SELECTION, URI_LIST, UriError, file_path, file_uri,
};

use super::content::Content;

/// Files on the clipboard: their URIs, and whether they were cut (to be moved by the paste)
/// rather than copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Files {
    pub uris: Vec<String>,
    pub cut: bool,
}

impl Files {
    /// The types a file list goes by, in the order they are offered and read.
    pub const TYPES: [&'static str; 3] = [GNOME_COPIED_FILES, URI_LIST, KDE_CUT_SELECTION];

    pub fn new<S: Into<String>>(uris: impl IntoIterator<Item = S>, cut: bool) -> Self {
        Files {
            uris: uris.into_iter().map(Into::into).collect(),
            cut,
        }
    }

    /// Files from absolute paths, as `file://` URIs ([`file_uri`]).
    pub fn from_paths<P: AsRef<Path>>(paths: impl IntoIterator<Item = P>, cut: bool) -> Self {
        Files::new(paths.into_iter().map(|p| file_uri(p.as_ref())), cut)
    }

    /// The local paths among the URIs; other schemes and other hosts are left out.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.uris
            .iter()
            .filter_map(|u| match file_path(u) {
                Ok(FileUri::Local(p)) => Some(p),
                _ => None,
            })
            .collect()
    }

    /// `content` with the three formats added.
    pub fn add_to(&self, content: Content) -> Content {
        content
            .with(
                GNOME_COPIED_FILES,
                mime::encode_gnome_copied_files(self.cut, &self.uris),
            )
            .with(URI_LIST, mime::encode_uri_list(&self.uris))
            .with(KDE_CUT_SELECTION, mime::encode_kde_cut(self.cut))
    }

    /// Whether a selection offering `mimes` holds files.
    pub fn offered<S: AsRef<str>>(mimes: &[S]) -> bool {
        mimes
            .iter()
            .any(|m| [GNOME_COPIED_FILES, URI_LIST].contains(&m.as_ref()))
    }

    /// The files in the payloads read, by the rule in the module docs. None when neither the
    /// GNOME payload nor `text/uri-list` names a file.
    pub fn decode(
        gnome: Option<&[u8]>,
        uri_list: Option<&[u8]>,
        kde: Option<&[u8]>,
    ) -> Option<Files> {
        if let Some((cut, uris)) = gnome.and_then(mime::decode_gnome_copied_files) {
            return Some(Files { uris, cut });
        }
        let uris = mime::decode_uri_list(uri_list?);
        (!uris.is_empty()).then(|| Files {
            uris,
            cut: kde.is_some_and(mime::decode_kde_cut),
        })
    }
}

#[cfg(test)]
mod tests;
