// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    error::Error,
    fs, io,
    io::Read,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::super::{
    ArchiveError, ArchiveOutcome, ExtractionSession, MemberContent, MemberMetadata,
};
use crate::{model::Location, services::TransferReport};

pub(super) const ARCHIVE: &str = "archive.zip";

pub(super) struct TestReader<F>(pub(super) F);

impl<F: FnMut(&mut [u8]) -> io::Result<usize>> Read for TestReader<F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        (self.0)(buffer)
    }
}

/// A member the session must refuse before reading it.
pub(super) fn unread(
    reason: &'static str,
) -> TestReader<impl FnMut(&mut [u8]) -> io::Result<usize>> {
    TestReader(move |_: &mut [u8]| panic!("{reason}"))
}

/// What a session's destination holds: directories, file contents and link targets.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Node {
    Dir,
    File(Vec<u8>),
    Link(PathBuf),
}

pub(super) fn file(contents: &[u8]) -> Node {
    Node::File(contents.to_vec())
}

pub(super) fn tree(folder: &Path) -> Result<BTreeMap<String, Node>, Box<dyn Error>> {
    let mut nodes = BTreeMap::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            let name = path.strip_prefix(folder)?.to_string_lossy().into_owned();
            let metadata = fs::symlink_metadata(&path)?;
            let node = if metadata.is_dir() {
                pending.push(path);
                Node::Dir
            } else if metadata.is_symlink() {
                Node::Link(fs::read_link(&path)?)
            } else {
                Node::File(fs::read(&path)?)
            };
            nodes.insert(name, node);
        }
    }
    Ok(nodes)
}

/// The listed entries, whose parents are implied directories.
fn expected_tree<const N: usize>(entries: [(&str, Node); N]) -> BTreeMap<String, Node> {
    let mut nodes = BTreeMap::new();
    for (name, node) in entries {
        for parent in Path::new(name).ancestors().skip(1) {
            if !parent.as_os_str().is_empty() {
                nodes
                    .entry(parent.to_string_lossy().into_owned())
                    .or_insert(Node::Dir);
            }
        }
        nodes.insert(name.to_owned(), node);
    }
    nodes
}

/// `folder` holds exactly `entries` and their parents.
pub(super) fn assert_tree<const N: usize>(
    folder: &Path,
    entries: [(&str, Node); N],
) -> Result<(), Box<dyn Error>> {
    assert_eq!(tree(folder)?, expected_tree(entries));
    Ok(())
}

pub(super) fn assert_same_inode(first: &Path, second: &Path) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::MetadataExt;
    assert_eq!(fs::metadata(first)?.ino(), fs::metadata(second)?.ino());
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Cancelled {
    pub(super) completed: Vec<Location>,
    pub(super) failed: Vec<Location>,
    pub(super) not_attempted: Vec<Location>,
}

/// Owns what an [`ExtractionSession`] borrows for a destination folder.
pub(super) struct Harness {
    root: tempfile::TempDir,
    destination: PathBuf,
    progress: AtomicUsize,
    cancelled: AtomicBool,
    report: TransferReport,
}

impl Harness {
    pub(super) fn new() -> Result<Self, Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        let destination = root.path().to_path_buf();
        Ok(Self {
            root,
            destination,
            progress: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
            report: TransferReport::default(),
        })
    }

    /// Extracts into a folder below the temporary root.
    pub(super) fn in_folder(name: &str) -> Result<Self, Box<dyn Error>> {
        let mut harness = Self::new()?;
        harness.destination = harness.root.path().join(name);
        fs::create_dir(&harness.destination)?;
        Ok(harness)
    }

    pub(super) fn path(&self) -> &Path {
        &self.destination
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub(super) fn open(&mut self) -> Result<Session<'_>, ArchiveError> {
        self.open_named(ARCHIVE)
    }

    pub(super) fn open_named(
        &mut self,
        archive: &'static str,
    ) -> Result<Session<'_>, ArchiveError> {
        let inner = ExtractionSession::open(
            &self.destination,
            archive,
            &self.progress,
            &self.cancelled,
            &mut self.report,
        )?;
        Ok(Session {
            inner: Some(inner),
            root: self.root.path(),
            destination: &self.destination,
            progress: &self.progress,
            cancelled: &self.cancelled,
        })
    }

    pub(super) fn open_with_available_bytes(
        &mut self,
        available: Option<u64>,
    ) -> Result<Session<'_>, ArchiveError> {
        let inner = ExtractionSession::open_with_available_bytes(
            &self.destination,
            ARCHIVE,
            &self.progress,
            &self.cancelled,
            available,
            &mut self.report,
        )?;
        Ok(Session {
            inner: Some(inner),
            root: self.root.path(),
            destination: &self.destination,
            progress: &self.progress,
            cancelled: &self.cancelled,
        })
    }
}

pub(super) type Outcome = Result<ArchiveOutcome<Option<String>>, ArchiveError>;

/// An open session that keeps its destination and counters readable once finished.
pub(super) struct Session<'a> {
    inner: Option<ExtractionSession<'a>>,
    root: &'a Path,
    destination: &'a Path,
    progress: &'a AtomicUsize,
    cancelled: &'a AtomicBool,
}

impl<'a> Deref for Session<'a> {
    type Target = ExtractionSession<'a>;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().expect("the session was finished")
    }
}

impl DerefMut for Session<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().expect("the session was finished")
    }
}

impl<'a> Session<'a> {
    /// The folder the archive is extracted into.
    pub(super) fn path(&self) -> &'a Path {
        self.destination
    }

    /// The temporary folder, which holds the destination.
    pub(super) fn root(&self) -> &'a Path {
        self.root
    }

    pub(super) fn cancelled(&self) -> &'a AtomicBool {
        self.cancelled
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub(super) fn progress(&self) -> usize {
        self.progress.load(Ordering::Relaxed)
    }

    pub(super) fn location(&self, name: &str) -> Location {
        Location::local(self.destination.join(name))
    }

    pub(super) fn locations(&self, names: &[&str]) -> Vec<Location> {
        names.iter().map(|name| self.location(name)).collect()
    }

    pub(super) fn extract_dir(&mut self, name: &str) -> Result<(), ArchiveError> {
        self.extract_member(name, MemberContent::Directory, MemberMetadata::NONE)
    }

    /// A file member whose declared size matches its contents.
    pub(super) fn extract_file(&mut self, name: &str, contents: &[u8]) -> Result<(), ArchiveError> {
        let declared = Some(contents.len() as u64);
        self.extract_member(
            name,
            MemberContent::File(&mut &contents[..], declared),
            MemberMetadata::NONE,
        )
    }

    /// Extracts a member the session must refuse as cancelled.
    pub(super) fn extract_cancelled(
        &mut self,
        name: &str,
        content: MemberContent<'_>,
    ) -> Result<(), ArchiveError> {
        let result = self.extract_member(name, content, MemberMetadata::NONE);
        assert_eq!(result, Err(ArchiveError::Cancelled));
        result
    }

    pub(super) fn finish(
        &mut self,
        result: Result<(), ArchiveError>,
        remaining: impl FnOnce() -> Vec<String>,
    ) -> Outcome {
        self.inner
            .take()
            .expect("the session was finished")
            .finish(result, remaining)
    }

    /// The first name a completed extraction published.
    pub(super) fn finish_completed(
        &mut self,
        result: Result<(), ArchiveError>,
        remaining: impl FnOnce() -> Vec<String>,
    ) -> Result<Option<String>, Box<dyn Error>> {
        match self.finish(result, remaining)? {
            ArchiveOutcome::Completed(name) => Ok(name),
            other => Err(format!("expected a completed extraction, got {other:?}").into()),
        }
    }

    pub(super) fn finish_cancelled(
        &mut self,
        result: Result<(), ArchiveError>,
        remaining: impl FnOnce() -> Vec<String>,
    ) -> Result<Cancelled, Box<dyn Error>> {
        match self.finish(result, remaining)? {
            ArchiveOutcome::Cancelled {
                completed,
                failed,
                not_attempted,
            } => Ok(Cancelled {
                completed,
                failed,
                not_attempted,
            }),
            other => Err(format!("expected a cancelled extraction, got {other:?}").into()),
        }
    }

    /// The message of a failed member, whose failure must not enumerate the rest.
    pub(super) fn finish_failed(&mut self, result: Result<(), ArchiveError>) -> String {
        assert!(
            matches!(result, Err(ArchiveError::Failed(_))),
            "expected a failed member, got {result:?}"
        );
        match self.finish(result, || {
            panic!("failure must not enumerate remaining members")
        }) {
            Err(ArchiveError::Failed(message)) => message,
            other => panic!("expected a failed extraction, got {other:?}"),
        }
    }

    /// `progress` members were extracted, leaving exactly `entries` at the destination.
    pub(super) fn assert_extracted<const N: usize>(
        &self,
        progress: usize,
        entries: [(&str, Node); N],
    ) -> Result<(), Box<dyn Error>> {
        assert_eq!(self.progress(), progress);
        assert_tree(self.destination, entries)
    }
}
