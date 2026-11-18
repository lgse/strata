// SPDX-License-Identifier: MIT

//! Which files a player steps through, and its place among them (GTK's `family_listing`,
//! `media_sequence` and `step_media` in `src/ui/preview/media_session.rs`, and
//! `src/ui/preview/media_content.rs`).
//!
//! A name alone decides nothing: a file named as video whose metadata probe found no media
//! drops out at once, and one not probed yet joins only once its leading bytes are read and
//! back its name. The shown file always counts. The reads run off the UI thread through the
//! folder's mount lane, nearest file first, and each chunk's answers reach Rune through
//! `Later`, which refreshes the shown player's position.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gio::prelude::CancellableExt;
use rune::Tx;
use strata_core::model::{FileEntry, Location, MetadataValue};
use strata_fs::{
    adapters::mount_lanes::{self, Class, Responsiveness, Unanswered},
    services::{guess_content_type, read_supports_named_family},
};

use super::Family;
use crate::{App, shell, slots::Later};

/// The nearest file on each side answers first; later chunks grow to bound refreshes.
const FIRST_CHUNK: usize = 2;
const MAX_CHUNK: usize = 256;
/// How often a read asks again while a mount is busy with other work, not stalled.
const BUSY_RETRY: Duration = Duration::from_millis(100);

/// What a file's size and date were when its bytes were read.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
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

    /// Listings learn sizes and dates late, so only two known values that differ mean the
    /// file changed.
    fn same_file(&self, other: &Self) -> bool {
        fn agree<T: PartialEq>(left: &MetadataValue<T>, right: &MetadataValue<T>) -> bool {
            match (left, right) {
                (MetadataValue::Known(left), MetadataValue::Known(right)) => left == right,
                _ => true,
            }
        }
        agree(&self.size, &other.size) && agree(&self.modified, &other.modified)
    }
}

/// The answers for the listing the preview walks, and the reads that fill them.
#[derive(Default)]
pub(crate) struct Checks {
    answers: HashMap<PathBuf, (Fingerprint, bool)>,
    reads: Option<Reads>,
}

/// Reads for one shown file; dropping them stops reading.
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

impl Checks {
    /// Whether `entry` holds media: `None` until its bytes are read. Remote files go by
    /// their name.
    fn answer(&self, entry: &FileEntry, family: Family) -> Option<bool> {
        if probed_media(entry, family) {
            return Some(true);
        }
        let Some(path) = entry.location.native_path() else {
            return Some(true);
        };
        let (known, media) = self.answers.get(path)?;
        known.same_file(&Fingerprint::of(entry)).then_some(*media)
    }

    fn keep_only(&mut self, listing: &[FileEntry]) {
        let keep: HashSet<&Path> = listing
            .iter()
            .filter_map(|entry| entry.location.native_path())
            .collect();
        self.answers.retain(|path, _| keep.contains(path.as_path()));
    }

    fn already_reading(&self, anchor: &Location, unread: &[(PathBuf, Fingerprint)]) -> bool {
        self.reads.as_ref().is_some_and(|reads| {
            reads.anchor == *anchor && unread.iter().all(|(path, _)| reads.covered.contains(path))
        })
    }

    /// Reads `unread`, nearest first, unless reads for `anchor` already took them up.
    fn read(&mut self, anchor: &Location, unread: Vec<(PathBuf, Fingerprint)>, later: &Later<App>) {
        if unread.is_empty() || self.already_reading(anchor, &unread) {
            return;
        }
        let cancellable = gio::Cancellable::new();
        self.reads = Some(Reads {
            anchor: anchor.clone(),
            covered: unread.iter().map(|(path, _)| path.clone()).collect(),
            cancellable: cancellable.clone(),
        });
        let later = later.clone();
        glib::MainContext::default().spawn_local(read_all(unread, cancellable, later));
    }

    /// Stops reading for a file no longer shown.
    pub(crate) fn stop(&mut self) {
        self.reads = None;
    }
}

/// A metadata probe only runs on bytes that already back the media name.
fn probed_media(entry: &FileEntry, family: Family) -> bool {
    matches!(entry.duration_seconds, MetadataValue::Known(_))
        || (family == Family::Video && matches!(entry.image_dimensions, MetadataValue::Known(_)))
}

/// Which family a type is (GTK's `media_family`).
pub(super) fn family_of_type(content_type: &str) -> Option<Family> {
    if is_audio_type(content_type) {
        Some(Family::Audio)
    } else if content_type.starts_with("video/") {
        Some(Family::Video)
    } else {
        None
    }
}

fn is_audio_type(content_type: &str) -> bool {
    content_type.starts_with("audio/")
        && !matches!(
            content_type,
            "audio/x-mpegurl"
                | "audio/mpegurl"
                | "audio/x-scpls"
                | "audio/midi"
                | "audio/x-midi"
                | "audio/sp-midi"
        )
}

/// The family a listed file is named as, unless its metadata probe found no media (GTK's
/// `entry_family`): elementary streams can have dimensions without a duration.
fn entry_family(entry: &FileEntry) -> Option<Family> {
    if entry.is_directory() {
        return None;
    }
    let extension = Path::new(&entry.native_name)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    let family = family_of_type(&guess_content_type(Path::new(&format!("a.{extension}"))).0)?;
    let probed = entry.duration_seconds != MetadataValue::Unavailable
        || (family == Family::Video && matches!(entry.image_dimensions, MetadataValue::Known(_)));
    probed.then_some(family)
}

/// The files of `family` the player steps through at `depth`, and where `around` (or else
/// `current`) is among them. Files not read yet stay out, and are read.
pub(super) fn listing(
    app: &mut App,
    family: Family,
    current: &Location,
    around: Option<&Location>,
) -> Option<(Vec<FileEntry>, usize)> {
    let depth = app.preview.current_depth?;
    let show_hidden = app.browser.preferences().show_hidden;
    let named: Vec<FileEntry> = (0..)
        .map_while(|position| app.browser.entry_at(depth, position))
        .filter(|file| (show_hidden || !file.is_hidden) && entry_family(file) == Some(family))
        .collect();
    let checks = &mut app.preview.media_checks;
    checks.keep_only(&named);
    let (files, unread) = counted(checks, named, family, current);
    let later = app.preview.loader.later.clone();
    checks.read(current, unread, &later);
    let position = |location: &Location| files.iter().position(|entry| entry.location == *location);
    let index = around.and_then(position).or_else(|| position(current))?;
    Some((files, index))
}

/// Which of `named` count: those known to hold media, and `current`. And those not read yet,
/// nearest `current` first.
fn counted(
    checks: &Checks,
    named: Vec<FileEntry>,
    family: Family,
    current: &Location,
) -> (Vec<FileEntry>, Vec<(PathBuf, Fingerprint)>) {
    let shown = named
        .iter()
        .position(|entry| entry.location == *current)
        .unwrap_or_default();
    let mut unread = Vec::new();
    let files = named
        .into_iter()
        .enumerate()
        .filter(|(index, entry)| {
            let media = checks.answer(entry, family);
            if media.is_none()
                && let Some(path) = entry.location.native_path()
            {
                unread.push((*index, path.to_path_buf(), Fingerprint::of(entry)));
            }
            media == Some(true) || entry.location == *current
        })
        .map(|(_, entry)| entry)
        .collect();
    (files, nearest_first(unread, shown))
}

/// Outward from the shown file, the next one before the previous at each distance.
fn nearest_first(
    mut files: Vec<(usize, PathBuf, Fingerprint)>,
    around: usize,
) -> Vec<(PathBuf, Fingerprint)> {
    files.sort_by_key(|(index, ..)| (index.abs_diff(around), *index < around));
    files
        .into_iter()
        .map(|(_, path, fingerprint)| (path, fingerprint))
        .collect()
}

/// Moves the listing's cursor `step` files of the shown family along, as GTK's `step_to`
/// does: the preview follows the selection. The file it moved to, if there is one.
pub(super) fn step(
    app: &mut App,
    family: Family,
    current: &Location,
    step: i8,
    tx: &mut Tx,
) -> Option<Location> {
    // Repeated presses outrun the preview's debounce, so step from the cursor.
    let cursor = super::super::session::selected_entry(app)
        .1
        .map(|entry| entry.location);
    let Some((files, index)) = listing(app, family, current, cursor.as_ref()) else {
        return None;
    };
    let Some(target) = index
        .checked_add_signed(isize::from(step))
        .and_then(|index| files.get(index))
    else {
        return None;
    };
    let (Some(depth), Some(view)) = (app.preview.current_depth, shell::shown_view(app)) else {
        return None;
    };
    let at = (depth, shell::id_of(app, &target.location));
    (view.rows.select_only)(app, at, tx);
    (view.rows.reveal)(app, at, tx);
    Some(target.location.clone())
}

type Read = (PathBuf, Fingerprint, bool);

async fn read_all(files: Vec<(PathBuf, Fingerprint)>, stop: gio::Cancellable, later: Later<App>) {
    let mut queue = VecDeque::from(files);
    let mut size = FIRST_CHUNK;
    let mut run = ReadRun::default();
    while !queue.is_empty() && !stop.is_cancelled() {
        let chunk: Vec<_> = queue.drain(..size.min(queue.len())).collect();
        size = (size * 2).min(MAX_CHUNK);
        let learned = run.read_chunk(chunk, &stop).await;
        if learned.is_empty() || stop.is_cancelled() {
            continue;
        }
        later.then(move |app: &mut App, _: &mut Tx| learn(app, learned));
    }
}

/// A chunk's answers: kept, and the shown player's position refreshed.
fn learn(app: &mut App, learned: Vec<Read>) -> bool {
    app.preview.media_checks.answers.extend(
        learned
            .into_iter()
            .map(|(path, fingerprint, media)| (path, (fingerprint, media))),
    );
    super::refresh_listing(app)
}

/// What one run of reads learned about busy mounts.
#[derive(Default)]
struct ReadRun {
    /// Folders that got no answer, not asked again by the same run.
    given_up: HashSet<PathBuf>,
    /// When reads were first skipped for a busy mount: one wait covers every folder.
    busy_since: Option<Instant>,
}

impl ReadRun {
    async fn read_chunk(
        &mut self,
        chunk: Vec<(PathBuf, Fingerprint)>,
        stop: &gio::Cancellable,
    ) -> Vec<Read> {
        let mut answers = Vec::new();
        for (folder, files) in by_folder(chunk) {
            if self.given_up.contains(&folder) {
                continue;
            }
            let paths = files.iter().map(|(path, _)| path.clone()).collect();
            match self.read_folder(&folder, paths, stop).await {
                Some(read) => {
                    answers.extend(files.into_iter().zip(read).filter_map(
                        |((path, fingerprint), media)| Some((path, fingerprint, media?)),
                    ))
                }
                None if !stop.is_cancelled() => {
                    self.given_up.insert(folder);
                }
                None => {}
            }
        }
        answers
    }

    /// Background work through the folder's mount lane: it waits its turn while the mount is
    /// busy, for at most `NOT_RESPONDING_AFTER` across the run, and gives up at once when the
    /// mount has stalled.
    async fn read_folder(
        &mut self,
        folder: &Path,
        paths: Vec<PathBuf>,
        stop: &gio::Cancellable,
    ) -> Option<Vec<Option<bool>>> {
        while !stop.is_cancelled() {
            let (paths, cancel) = (paths.clone(), stop.clone());
            let read = mount_lanes::run(folder, Class::Background, move |progress| {
                paths
                    .iter()
                    .map(|path| {
                        if cancel.is_cancelled() {
                            return None;
                        }
                        progress
                            .reach(path, None, || read_supports_named_family(path))
                            .and_then(Result::ok)
                    })
                    .collect::<Vec<_>>()
            });
            match read.await {
                Ok(read) => return Some(read),
                Err(Unanswered::Skipped) if self.may_wait_for(folder) => {
                    glib::timeout_future(BUSY_RETRY).await;
                }
                Err(_) => return None,
            }
        }
        None
    }

    fn may_wait_for(&mut self, folder: &Path) -> bool {
        if mount_lanes::responsiveness(folder) != Responsiveness::Answering {
            return false;
        }
        let since = *self.busy_since.get_or_insert_with(Instant::now);
        since.elapsed() < mount_lanes::NOT_RESPONDING_AFTER
    }
}

fn by_folder(chunk: Vec<(PathBuf, Fingerprint)>) -> Vec<(PathBuf, Vec<(PathBuf, Fingerprint)>)> {
    let mut folders: Vec<(PathBuf, Vec<(PathBuf, Fingerprint)>)> = Vec::new();
    for (path, fingerprint) in chunk {
        let folder = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        match folders.iter_mut().find(|(known, _)| *known == folder) {
            Some((_, files)) => files.push((path, fingerprint)),
            None => folders.push((folder, vec![(path, fingerprint)])),
        }
    }
    folders
}

#[cfg(test)]
mod tests;
