// SPDX-License-Identifier: MIT

use std::{ffi::OsString, os::unix::ffi::OsStringExt};

use super::*;

const UUID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

fn named(name: &str) -> OsString {
    OsString::from(name.replace("<uuid>", UUID))
}

#[test]
fn leftovers_are_named_by_prefix_and_suffix() {
    let mut rows: Vec<(OsString, Option<Leftover>, Option<ForeignLeftover>)> = [
        (HiddenName::Probe, Leftover::Probe),
        (HiddenName::Copy, Leftover::IncompleteCopy),
        (HiddenName::Extraction, Leftover::IncompleteCopy),
        (HiddenName::Replacement, Leftover::MaybeOriginal),
        (HiddenName::Compression, Leftover::MaybeOriginal),
        (HiddenName::Replaced, Leftover::Original),
        (HiddenName::Rename, Leftover::Original),
        (HiddenName::Merge, Leftover::MergeOriginals),
        (HiddenName::Originals, Leftover::Originals),
    ]
    .into_iter()
    .map(|(name, class)| (name.fresh(), Some(class), None))
    .collect();
    rows.extend(
        [
            (
                ".strata-symlink-<uuid>",
                Some(Leftover::IncompleteCopy),
                None,
            ),
            (
                ".strata-replacement-aB3xY9",
                Some(Leftover::MaybeOriginal),
                None,
            ),
            (
                ".strata-compression-Qw12eR",
                Some(Leftover::IncompleteCopy),
                None,
            ),
            (
                ".strata-copy-0F8FAD5B-D9CB-469F-A165-70867728950E",
                Some(Leftover::IncompleteCopy),
                None,
            ),
            (".strata-copy-notes", None, None),
            (".strata-x-<uuid>", None, None),
            (".strata-copy-", None, None),
            (".strata-", None, None),
            (".strata-copy-<uuid>.txt", None, None),
            (
                ".strata-copy-0f8fad5bd9cb-469f-a165-70867728950e0",
                None,
                None,
            ),
            (
                ".strata-copy-0f8fad5g-d9cb-469f-a165-70867728950e",
                None,
                None,
            ),
            (".strata-replaced-aB3xY9", None, None),
            (".strata-compression-aB3xY", None, None),
            (".strata-compression-aB3_Y9", None, None),
            (".strata-write-123-4.tmp", None, None),
            ("notes.strata-copy-<uuid>", None, None),
            (".Strata-copy-<uuid>", None, None),
            (
                ".nfs000000000000005e000000de",
                None,
                Some(ForeignLeftover::Nfs),
            ),
            (
                ".fuse_hidden0000005f00000001",
                None,
                Some(ForeignLeftover::Fuse),
            ),
            (".__smbfile_sillye6", None, Some(ForeignLeftover::Smb)),
            (".nfs000000000000005e000000d", None, None),
            (".nfs000000000000005E000000DE", None, None),
            (".nfsnotes", None, None),
            (".fuse_hidden0000005f0000001", None, None),
            (".__smbfile_silly", None, None),
            (".__smbfile_sillyxyz", None, None),
            (
                "notes.txt.ntfs-3g-0000000001",
                None,
                Some(ForeignLeftover::Ntfs3gRenameOver),
            ),
            ("notes.ntfs-3g-123", None, None),
            ("notes.ntfs-3g-00000000001", None, None),
            ("notes.ntfs-3g-000000000a", None, None),
            (".ntfs-3g-0000000001", None, None),
        ]
        .into_iter()
        .map(|(name, class, foreign)| (named(name), class, foreign)),
    );
    let mut cut = named(".strata-copy-<uuid>").into_vec();
    cut[20] = 0xff;
    rows.push((OsString::from_vec(cut), None, None));

    for (name, class, foreign) in rows {
        assert_eq!(classify(&name), class, "{name:?}");
        assert_eq!(foreign_leftover(&name), foreign, "{name:?}");
    }
}
