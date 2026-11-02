// SPDX-License-Identifier: MIT

use std::{
    error::Error,
    ffi::OsStr,
    fs,
    os::unix::ffi::OsStrExt,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize},
};

use super::super::{
    ArchiveError, ArchiveOutcome, ExtractionSession, MemberContent, MemberMetadata,
};
use super::harness::Outcome;
use crate::{
    model::Location,
    services::{RenameCause, RenamedItem, SkippedItem, SpecialKind, TransferReport},
};

#[derive(Clone, Copy)]
pub(super) enum Member<'a> {
    Folder,
    File,
    Link(&'a [u8]),
    Hard(&'a str),
}

pub(super) type Entry<'a> = (&'a [u8], Member<'a>);

/// What an extraction of fixed members ended as, and what it reported.
pub(super) struct Extracted {
    pub(super) outcome: Outcome,
    pub(super) report: TransferReport,
}

impl Extracted {
    /// The name a completed extraction published.
    pub(super) fn completed(&self) -> &str {
        match &self.outcome {
            Ok(ArchiveOutcome::Completed(Some(name))) => name,
            other => panic!("expected a completed extraction with a name, got {other:?}"),
        }
    }

    /// The links below `root` the report says were left out.
    pub(super) fn assert_skipped_links(&self, root: &Path, names: &[&str]) {
        assert_eq!(self.report.skipped, skipped_links(root, names));
    }
}

pub(super) fn skipped_links(root: &Path, names: &[&str]) -> Vec<SkippedItem> {
    names
        .iter()
        .map(|name| SkippedItem {
            location: Location::local(root.join(name)),
            kind: SpecialKind::Symlink,
        })
        .collect()
}

/// Extracts `members` from `archive` into `destination`, recording hard-link targets,
/// and ends the session with `ending` unless a member failed first.
pub(super) fn extract_members(
    destination: &Path,
    archive: &str,
    members: &[Entry<'_>],
    ending: Result<(), ArchiveError>,
) -> Extracted {
    let progress = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let mut report = TransferReport::default();
    let outcome = ExtractionSession::open(destination, archive, &progress, &cancelled, &mut report)
        .and_then(|mut session| {
            session.record_hard_link_targets();
            let result = members.iter().try_for_each(|(name, member)| {
                let content = match member {
                    Member::Folder => MemberContent::Directory,
                    Member::File => MemberContent::File(&mut &b"data"[..], Some(4)),
                    Member::Link(target) => MemberContent::Symlink(target),
                    Member::Hard(target) => MemberContent::HardLink(Path::new(target)),
                };
                session.extract_member(OsStr::from_bytes(name), content, MemberMetadata::NONE)
            });
            session.finish(result.and(ending), || {
                vec![format!("10:00/later:x{}.d/next.txt", "a".repeat(300))]
            })
        });
    Extracted { outcome, report }
}

/// Every entry below `folder`, sorted, with directories marked by a trailing slash.
pub(super) fn entries(folder: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut entries = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            let mut name = path.strip_prefix(folder)?.to_string_lossy().into_owned();
            if fs::symlink_metadata(&path)?.is_dir() {
                name.push('/');
                pending.push(path);
            }
            entries.push(name);
        }
    }
    entries.sort();
    Ok(entries)
}

pub(super) fn renamed<B: AsRef<[u8]>, S: AsRef<str>>(
    root: &Path,
    entries: &[(B, S, RenameCause)],
) -> Vec<RenamedItem> {
    entries
        .iter()
        .map(|(requested, saved_as, cause)| RenamedItem {
            requested: Location::local(root.join(OsStr::from_bytes(requested.as_ref()))),
            saved_as: Location::local(root.join(saved_as.as_ref())),
            cause: *cause,
        })
        .collect()
}
