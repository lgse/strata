// SPDX-License-Identifier: MIT

//! Whether local files named as audio or video hold media. A name alone decides nothing:
//! the preview reads a file's leading bytes before choosing a player, so a media sequence
//! that trusted names would count, and land on, files that preview as text.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use super::MediaFamily;
use crate::{
    adapters::mount_lanes::{self, Class, Responsiveness, Unanswered},
    model::{FileEntry, Location, MetadataValue},
};

/// The nearest file on each side answers first; later chunks grow to bound refreshes.
const FIRST_CHUNK: usize = 2;
const MAX_CHUNK: usize = 256;
/// How long a read waits for a mount that is busy with other work, not stalled.
const BUSY_RETRY: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Fingerprint {
    size: MetadataValue<u64>,
    modified: MetadataValue<i64>,
}

impl Fingerprint {
    fn of(entry: &FileEntry) -> Self {
        Self {
            size: entry.size.clone(),
            modified: entry.modified_unix_seconds.clone(),
        }
    }

    /// Listings learn sizes and dates late, so only two known values that differ
    /// mean the file changed.
    fn same_file(&self, other: &Self) -> bool {
        fn agree<T: PartialEq>(left: &MetadataValue<T>, right: &MetadataValue<T>) -> bool {
            match (left, right) {
                (MetadataValue::Known(left), MetadataValue::Known(right)) => left == right,
                _ => true,
            }
        }
        agree(&self.size, &other.size) && agree(&self.modified, &other.modified)
    }

    fn learn(&mut self, other: &Self) {
        if matches!(other.size, MetadataValue::Known(_)) {
            self.size = other.size.clone();
        }
        if matches!(other.modified, MetadataValue::Known(_)) {
            self.modified = other.modified.clone();
        }
    }
}

/// Only answered reads: a failed or abandoned read is asked again later.
type Answers = Rc<RefCell<HashMap<PathBuf, (Fingerprint, bool)>>>;

/// One preview's answers for the listing it walks, and the reads that fill them.
#[derive(Default)]
pub(super) struct MediaChecks {
    answers: Answers,
    reads: Option<Reads>,
    #[cfg(test)]
    queued: usize,
}

/// Unanswered files of one listing, by their place in it.
pub(super) struct Unread {
    around: usize,
    files: Vec<(usize, PathBuf, Fingerprint)>,
}

/// Reads for one shown file; dropping them stops reading before the next file.
struct Reads {
    anchor: Location,
    covered: HashSet<PathBuf>,
    cancellable: gio::Cancellable,
}

impl Drop for Reads {
    fn drop(&mut self) {
        self.cancellable.cancel();
    }
}

impl MediaChecks {
    /// Whether `entry`, named as `family`, holds media; `None` until its bytes are read.
    /// Route files and remote locations go by their name.
    pub(super) fn answer(&self, entry: &FileEntry, family: MediaFamily) -> Option<bool> {
        if family == MediaFamily::Route || probed_media(entry, family) {
            return Some(true);
        }
        let Some(path) = entry.location.native_path() else {
            return Some(true);
        };
        let fingerprint = Fingerprint::of(entry);
        let mut answers = self.answers.borrow_mut();
        let (known, media) = answers.get_mut(path)?;
        known.same_file(&fingerprint).then(|| {
            known.learn(&fingerprint);
            *media
        })
    }

    /// Forgets answers for files no longer in the walked listing.
    pub(super) fn keep_only(&self, listing: &[FileEntry]) {
        let keep: HashSet<&Path> = listing
            .iter()
            .filter_map(|entry| entry.location.native_path())
            .collect();
        self.answers
            .borrow_mut()
            .retain(|path, _| keep.contains(path.as_path()));
    }

    /// Reads `unread` around `anchor` unless reads for it already took them up, so a
    /// walk they triggered, or a failed read, does not start them again. `answered`
    /// runs after each chunk that answered something and returns whether to go on.
    pub(super) fn read(
        &mut self,
        anchor: &Location,
        unread: Unread,
        answered: impl Fn() -> bool + 'static,
    ) {
        let covered = self.reads.as_ref().is_some_and(|reads| {
            reads.anchor == *anchor
                && unread
                    .files
                    .iter()
                    .all(|(_, path, _)| reads.covered.contains(path))
        });
        if unread.files.is_empty() || covered {
            return;
        }
        let files = unread.nearest_first();
        #[cfg(test)]
        {
            self.queued += files.len();
        }
        let cancellable = gio::Cancellable::new();
        self.reads = Some(Reads {
            anchor: anchor.clone(),
            covered: files.iter().map(|(path, _)| path.clone()).collect(),
            cancellable: cancellable.clone(),
        });
        let answers = self.answers.clone();
        glib::MainContext::default().spawn_local(async move {
            let mut queue = VecDeque::from(files);
            let mut size = FIRST_CHUNK;
            while !queue.is_empty() && !cancellable.is_cancelled() {
                let chunk: Vec<_> = queue.drain(..size.min(queue.len())).collect();
                size = (size * 2).min(MAX_CHUNK);
                let learned = read_chunk(chunk, &cancellable).await;
                if learned.is_empty() {
                    continue;
                }
                answers.borrow_mut().extend(
                    learned
                        .into_iter()
                        .map(|(path, fingerprint, media)| (path, (fingerprint, media))),
                );
                if !cancellable.is_cancelled() && !answered() {
                    break;
                }
            }
        });
    }

    pub(super) fn stop(&mut self) {
        self.reads = None;
    }

    /// Files queued for reading so far, to prove a walk converges.
    #[cfg(test)]
    pub(super) fn queued(&self) -> usize {
        self.queued
    }
}

/// A metadata probe only runs on bytes that already back the media name.
fn probed_media(entry: &FileEntry, family: MediaFamily) -> bool {
    matches!(entry.duration_seconds, MetadataValue::Known(_))
        || (family == MediaFamily::Video
            && matches!(entry.image_dimensions, MetadataValue::Known(_)))
}

impl Unread {
    /// `around` is the shown file's place in the listing.
    pub(super) fn new(around: usize) -> Self {
        Self {
            around,
            files: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, index: usize, entry: &FileEntry) {
        if let Some(path) = entry.location.native_path() {
            self.files
                .push((index, path.to_path_buf(), Fingerprint::of(entry)));
        }
    }

    /// Outward from the shown file, the next one before the previous at each distance.
    pub(super) fn nearest_first(mut self) -> Vec<(PathBuf, Fingerprint)> {
        let around = self.around;
        self.files
            .sort_by_key(|(index, ..)| (index.abs_diff(around), *index < around));
        self.files
            .into_iter()
            .map(|(_, path, fingerprint)| (path, fingerprint))
            .collect()
    }
}

/// Reads one chunk folder by folder; files whose read gets no answer are left out.
async fn read_chunk(
    chunk: Vec<(PathBuf, Fingerprint)>,
    cancellable: &gio::Cancellable,
) -> Vec<(PathBuf, Fingerprint, bool)> {
    let mut folders: Vec<(PathBuf, Vec<(PathBuf, Fingerprint)>)> = Vec::new();
    for (path, fingerprint) in chunk {
        let folder = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        match folders.iter_mut().find(|(known, _)| *known == folder) {
            Some((_, files)) => files.push((path, fingerprint)),
            None => folders.push((folder, vec![(path, fingerprint)])),
        }
    }
    let mut answers = Vec::new();
    for (folder, files) in folders {
        let paths: Vec<PathBuf> = files.iter().map(|(path, _)| path.clone()).collect();
        let Some(read) = read_folder(&folder, paths, cancellable).await else {
            continue;
        };
        answers.extend(
            files
                .into_iter()
                .zip(read)
                .filter_map(|((path, fingerprint), media)| Some((path, fingerprint, media?))),
        );
    }
    answers
}

/// Background work through the folder's mount lane: it waits its turn while the mount
/// is busy with other requests and gives up once the mount has stalled. Each read
/// follows symbolic links through `reach`, so a dead link target trips only itself.
async fn read_folder(
    folder: &Path,
    paths: Vec<PathBuf>,
    cancellable: &gio::Cancellable,
) -> Option<Vec<Option<bool>>> {
    loop {
        if cancellable.is_cancelled() {
            return None;
        }
        let paths = paths.clone();
        let stop = cancellable.clone();
        let job = mount_lanes::run(folder, Class::Background, move |progress| {
            paths
                .iter()
                .map(|path| {
                    if stop.is_cancelled() {
                        return None;
                    }
                    progress
                        .reach(path, None, || {
                            crate::services::read_supports_named_family(path)
                        })
                        .and_then(Result::ok)
                })
                .collect::<Vec<_>>()
        });
        match job.await {
            Ok(read) => return Some(read),
            Err(Unanswered::Skipped)
                if mount_lanes::responsiveness(folder) == Responsiveness::Answering =>
            {
                glib::timeout_future(BUSY_RETRY).await;
            }
            Err(_) => return None,
        }
    }
}
