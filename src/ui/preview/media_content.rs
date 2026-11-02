// SPDX-License-Identifier: MIT

//! Whether local files named as audio or video hold media. A name alone decides nothing:
//! the preview reads a file's leading bytes before choosing a player, so a media sequence
//! that trusted names would count, and land on, files that preview as text.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};

use gtk::{gio, glib, prelude::*};

use super::MediaFamily;
use crate::{
    adapters::mount_lanes::{self, Class},
    model::{FileEntry, Location, MetadataValue},
};

/// Answers outside the listing being walked are forgotten past this many.
const MAX_REMEMBERED: usize = if cfg!(test) { 4 } else { 8192 };
/// The nearest file on each side answers first; later chunks grow to bound refreshes.
const FIRST_CHUNK: usize = 2;
const MAX_CHUNK: usize = 256;

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

thread_local! {
    /// Only answered reads: a failed or skipped read is asked again later.
    static ANSWERS: RefCell<HashMap<PathBuf, (Fingerprint, bool)>> =
        RefCell::new(HashMap::new());
}

#[cfg(test)]
thread_local! {
    static QUEUED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Files queued for reading so far, to prove a walk converges.
#[cfg(test)]
pub(super) fn queued_reads() -> usize {
    QUEUED.get()
}

/// Whether `entry`, named as `family`, holds media; `None` until its bytes are read.
/// Route files and remote locations go by their name.
pub(super) fn answer(entry: &FileEntry, family: MediaFamily) -> Option<bool> {
    if family == MediaFamily::Route || probed_media(entry, family) {
        return Some(true);
    }
    let Some(path) = entry.location.native_path() else {
        return Some(true);
    };
    let fingerprint = Fingerprint::of(entry);
    ANSWERS.with_borrow_mut(|answers| {
        let (known, media) = answers.get_mut(path)?;
        known.same_file(&fingerprint).then(|| {
            known.learn(&fingerprint);
            *media
        })
    })
}

/// A metadata probe only runs on bytes that already back the media name.
fn probed_media(entry: &FileEntry, family: MediaFamily) -> bool {
    matches!(entry.duration_seconds, MetadataValue::Known(_))
        || (family == MediaFamily::Video
            && matches!(entry.image_dimensions, MetadataValue::Known(_)))
}

/// Keeps every answer for `listing`, however many, so walking it again never reads
/// its files again; answers for other folders go once there are too many.
pub(super) fn forget_all_but(listing: &[FileEntry]) {
    ANSWERS.with_borrow_mut(|answers| {
        if answers.len() <= MAX_REMEMBERED {
            return;
        }
        let keep: HashSet<&Path> = listing
            .iter()
            .filter_map(|entry| entry.location.native_path())
            .collect();
        answers.retain(|path, _| keep.contains(path.as_path()));
    });
}

/// Unanswered files of one listing, by their place in it.
pub(super) struct Unread {
    around: usize,
    files: Vec<(usize, PathBuf, Fingerprint)>,
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

    pub(super) fn is_empty(&self) -> bool {
        self.files.is_empty()
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

/// One preview's reads; dropping them stops reading before the next chunk.
pub(super) struct Reads {
    anchor: Location,
    covered: HashSet<PathBuf>,
    cancellable: gio::Cancellable,
}

impl Drop for Reads {
    fn drop(&mut self) {
        self.cancellable.cancel();
    }
}

impl Reads {
    /// Whether these reads already took up `unread` for the same shown file, so a walk
    /// they triggered, or a failed read, does not start them again.
    pub(super) fn covers(&self, anchor: &Location, unread: &Unread) -> bool {
        self.anchor == *anchor
            && unread
                .files
                .iter()
                .all(|(_, path, _)| self.covered.contains(path))
    }

    /// Reads `unread` in growing chunks, nearest first, and calls `answered` after each
    /// chunk that answered something; it returns whether reading should go on.
    pub(super) fn start(
        anchor: Location,
        unread: Unread,
        answered: impl Fn() -> bool + 'static,
    ) -> Self {
        let files = unread.nearest_first();
        #[cfg(test)]
        QUEUED.set(QUEUED.get() + files.len());
        let covered = files.iter().map(|(path, _)| path.clone()).collect();
        let cancellable = gio::Cancellable::new();
        let reading = cancellable.clone();
        glib::MainContext::default().spawn_local(async move {
            let mut queue = VecDeque::from(files);
            let mut size = FIRST_CHUNK;
            while !queue.is_empty() && !reading.is_cancelled() {
                let chunk: Vec<_> = queue.drain(..size.min(queue.len())).collect();
                size = (size * 2).min(MAX_CHUNK);
                let learned = remember(read_chunk(chunk, &reading).await);
                if learned && !reading.is_cancelled() && !answered() {
                    break;
                }
            }
        });
        Self {
            anchor,
            covered,
            cancellable,
        }
    }
}

/// Reads one chunk folder by folder, each through its mount's lane as background work:
/// a stalled mount skips its files, which stay unanswered.
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
        if cancellable.is_cancelled() {
            break;
        }
        let paths: Vec<PathBuf> = files.iter().map(|(path, _)| path.clone()).collect();
        let stop = cancellable.clone();
        let job = mount_lanes::run(&folder, Class::Background, move |progress| {
            paths
                .iter()
                .map(|path| {
                    if stop.is_cancelled() {
                        return None;
                    }
                    let answer = crate::services::read_supports_named_family(path);
                    progress.answered(&answer);
                    answer.ok()
                })
                .collect::<Vec<_>>()
        });
        let Ok(read) = job.await else {
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

/// Whether there was anything to remember.
fn remember(answers: Vec<(PathBuf, Fingerprint, bool)>) -> bool {
    let learned = !answers.is_empty();
    ANSWERS.with_borrow_mut(|known| {
        for (path, fingerprint, media) in answers {
            known.insert(path, (fingerprint, media));
        }
    });
    learned
}
