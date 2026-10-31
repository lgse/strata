// SPDX-License-Identifier: MIT

use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HiddenName {
    Probe,
    Copy,
    Extraction,
    Replacement,
    Compression,
    Replaced,
    Rename,
    Merge,
    Originals,
}

impl HiddenName {
    pub(crate) const fn prefix(self) -> &'static str {
        match self {
            Self::Probe => ".strata-probe-",
            Self::Copy => ".strata-copy-",
            Self::Extraction => ".strata-extraction-",
            Self::Replacement => ".strata-replacement-",
            Self::Compression => ".strata-compression-",
            Self::Replaced => ".strata-replaced-",
            Self::Rename => ".strata-rename-",
            Self::Merge => ".strata-merge-",
            Self::Originals => ".strata-originals-",
        }
    }

    pub(crate) fn fresh(self) -> OsString {
        OsString::from(format!(
            "{}{}",
            self.prefix(),
            gio::glib::uuid_string_random()
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Leftover {
    Probe,
    IncompleteCopy,
    MaybeOriginal,
    Original,
    Originals,
    MergeOriginals,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ForeignLeftover {
    Nfs,
    Fuse,
    Smb,
    Ntfs3gRenameOver,
}

#[derive(Clone, Copy)]
enum Suffix {
    Uuid,
    Tempfile,
    LowerHex(usize),
    Counter,
    Tail { tag: &'static str, digits: usize },
}

const STRATA: &[u8] = b".strata-";
const LEGACY_SYMLINK: &str = ".strata-symlink-";

const LEFTOVERS: [(&str, Suffix, Leftover); 12] = [
    (HiddenName::Probe.prefix(), Suffix::Uuid, Leftover::Probe),
    (
        HiddenName::Copy.prefix(),
        Suffix::Uuid,
        Leftover::IncompleteCopy,
    ),
    (
        HiddenName::Extraction.prefix(),
        Suffix::Uuid,
        Leftover::IncompleteCopy,
    ),
    (LEGACY_SYMLINK, Suffix::Uuid, Leftover::IncompleteCopy),
    (
        HiddenName::Compression.prefix(),
        Suffix::Tempfile,
        Leftover::IncompleteCopy,
    ),
    (
        HiddenName::Compression.prefix(),
        Suffix::Uuid,
        Leftover::MaybeOriginal,
    ),
    (
        HiddenName::Replacement.prefix(),
        Suffix::Uuid,
        Leftover::MaybeOriginal,
    ),
    (
        HiddenName::Replacement.prefix(),
        Suffix::Tempfile,
        Leftover::MaybeOriginal,
    ),
    (
        HiddenName::Replaced.prefix(),
        Suffix::Uuid,
        Leftover::Original,
    ),
    (
        HiddenName::Rename.prefix(),
        Suffix::Uuid,
        Leftover::Original,
    ),
    (
        HiddenName::Originals.prefix(),
        Suffix::Uuid,
        Leftover::Originals,
    ),
    (
        HiddenName::Merge.prefix(),
        Suffix::Uuid,
        Leftover::MergeOriginals,
    ),
];

const FOREIGN_LEFTOVERS: [(&str, Suffix, ForeignLeftover); 4] = [
    (".nfs", Suffix::LowerHex(24), ForeignLeftover::Nfs),
    (".fuse_hidden", Suffix::LowerHex(16), ForeignLeftover::Fuse),
    (".__smbfile_silly", Suffix::Counter, ForeignLeftover::Smb),
    (
        "",
        Suffix::Tail {
            tag: ".ntfs-3g-",
            digits: 10,
        },
        ForeignLeftover::Ntfs3gRenameOver,
    ),
];

pub(crate) fn classify(name: &OsStr) -> Option<Leftover> {
    let name = name.as_bytes();
    if !name.starts_with(STRATA) {
        return None;
    }
    find(&LEFTOVERS, name)
}

pub(crate) fn foreign_leftover(name: &OsStr) -> Option<ForeignLeftover> {
    find(&FOREIGN_LEFTOVERS, name.as_bytes())
}

fn find<T: Copy>(table: &[(&str, Suffix, T)], name: &[u8]) -> Option<T> {
    table.iter().find_map(|&(prefix, suffix, found)| {
        name.strip_prefix(prefix.as_bytes())
            .is_some_and(|rest| suffix.matches(rest))
            .then_some(found)
    })
}

impl Suffix {
    fn matches(self, rest: &[u8]) -> bool {
        let lower_hex = |byte: &u8| matches!(byte, b'0'..=b'9' | b'a'..=b'f');
        match self {
            Self::Uuid => {
                rest.len() == 36
                    && rest.iter().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            *byte == b'-'
                        } else {
                            byte.is_ascii_hexdigit()
                        }
                    })
            }
            Self::Tempfile => rest.len() == 6 && rest.iter().all(u8::is_ascii_alphanumeric),
            Self::LowerHex(digits) => rest.len() == digits && rest.iter().all(lower_hex),
            Self::Counter => !rest.is_empty() && rest.iter().all(lower_hex),
            Self::Tail { tag, digits } => {
                rest.len() > tag.len() + digits && {
                    let (tagged, number) = rest.split_at(rest.len() - digits);
                    tagged.ends_with(tag.as_bytes()) && number.iter().all(u8::is_ascii_digit)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
