// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gdk, gio, glib};

use crate::{
    media::peaks::BUCKETS,
    model::{FileEntry, MetadataValue},
    sandbox::{
        Cancellation, MediaPreviewBackend, ParseOperation,
        media::{PeaksEvent, PeaksSession},
        metadata::AudioTags,
    },
    services::SandboxedMedia,
};

const DETAILS_CACHE: usize = 12;
const PEAKS_CACHE: usize = 64;
/// Fast j/k browsing must not start a full decode for every track it passes.
const PEAKS_SETTLE: Duration = Duration::from_millis(450);
const PEAKS_POLL: Duration = Duration::from_millis(40);
const FOLDER_ART_STEMS: [&str; 5] = ["cover", "folder", "front", "album", "albumart"];
const FOLDER_ART_EXTENSIONS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrackKey {
    location: String,
    size: Option<u64>,
    modified: Option<i64>,
}

impl TrackKey {
    pub(super) fn of(entry: &FileEntry) -> Self {
        Self {
            location: entry.location.display_path(),
            size: known(&entry.size),
            modified: known(&entry.modified_unix_seconds),
        }
    }
}

fn known<T: Copy>(value: &MetadataValue<T>) -> Option<T> {
    match value {
        MetadataValue::Known(value) => Some(*value),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

struct Lru<V> {
    capacity: usize,
    entries: VecDeque<(TrackKey, V)>,
}

impl<V: Clone> Lru<V> {
    const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    fn get(&mut self, key: &TrackKey) -> Option<V> {
        let index = self.entries.iter().position(|(entry, _)| entry == key)?;
        let entry = self.entries.remove(index)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }

    fn insert(&mut self, key: TrackKey, value: V) {
        self.entries.retain(|(entry, _)| *entry != key);
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((key, value));
    }
}

#[derive(Clone, Default)]
pub(super) struct Details {
    pub(super) tags: AudioTags,
    pub(super) cover: Option<gdk::Texture>,
}

thread_local! {
    static DETAILS: RefCell<Lru<Rc<Details>>> = const { RefCell::new(Lru::new(DETAILS_CACHE)) };
    static PEAKS: RefCell<Lru<Rc<Vec<u8>>>> = const { RefCell::new(Lru::new(PEAKS_CACHE)) };
}

pub(super) fn cached_details(key: &TrackKey) -> Option<Rc<Details>> {
    DETAILS.with_borrow_mut(|cache| cache.get(key))
}

/// Cancels its sandbox jobs when dropped.
pub(super) struct DetailsLoad(Cancellation);

impl Drop for DetailsLoad {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn parse(path: &Path, operation: ParseOperation, cancellation: &Cancellation) -> Option<Vec<u8>> {
    crate::sandbox::parse(
        path,
        operation,
        0,
        MediaPreviewBackend::Software,
        cancellation,
    )
    .ok()
    .map(|output| output.data)
}

fn folder_art(directory: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<(usize, PathBuf)> = std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            let rank = FOLDER_ART_STEMS.iter().position(|name| *name == stem)?;
            (FOLDER_ART_EXTENSIONS.contains(&extension.as_str())
                && entry.file_type().is_ok_and(|kind| kind.is_file()))
            .then_some((rank, path))
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next().map(|(_, path)| path)
}

fn texture(png: Vec<u8>) -> Option<gdk::Texture> {
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)).ok()
}

/// Tags arrive first; embedded artwork, then folder artwork, follow.
pub(super) fn load_details(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_tags: impl Fn(AudioTags) + 'static,
    on_cover: impl Fn(Option<gdk::Texture>) + 'static,
) -> DetailsLoad {
    let key = TrackKey::of(entry);
    let cancellation = Cancellation::default();
    let details = Rc::new(RefCell::new(Details::default()));
    let pending = Rc::new(Cell::new(2));
    let finish = {
        let details = details.clone();
        let cancellation = cancellation.clone();
        move || {
            pending.set(pending.get() - 1);
            if pending.get() == 0 && !cancellation.is_cancelled() {
                let details = Rc::new(details.borrow().clone());
                DETAILS.with_borrow_mut(|cache| cache.insert(key.clone(), details));
            }
        }
    };
    let finish = Rc::new(finish);

    let path = source.path.clone();
    let lease = source.clone();
    let job = cancellation.clone();
    let cancelled = cancellation.clone();
    let tags_details = details.clone();
    let tags_finish = finish.clone();
    glib::MainContext::default().spawn_local(async move {
        let tags = gio::spawn_blocking(move || {
            let _lease = lease;
            parse(&path, ParseOperation::AudioTags, &job)
                .and_then(|json| AudioTags::from_json(&json).ok())
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        // A reused view may already show another track.
        if cancelled.is_cancelled() {
            return;
        }
        tags_details.borrow_mut().tags = tags.clone();
        tags_finish();
        on_tags(tags);
    });

    let path = source.path.clone();
    let lease = source.clone();
    let directory = entry
        .location
        .native_path()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let job = cancellation.clone();
    let cancelled = cancellation.clone();
    glib::MainContext::default().spawn_local(async move {
        let cover = gio::spawn_blocking(move || {
            let _lease = lease;
            parse(&path, ParseOperation::AudioCover, &job)
                .or_else(|| {
                    let art = folder_art(directory.as_deref()?)?;
                    parse(&art, ParseOperation::PreviewImage, &job)
                })
                .and_then(texture)
        })
        .await
        .ok()
        .flatten();
        if cancelled.is_cancelled() {
            return;
        }
        details.borrow_mut().cover = cover.clone();
        finish();
        on_cover(cover);
    });
    DetailsLoad(cancellation)
}

type Timer = Rc<RefCell<Option<glib::SourceId>>>;

/// Streams a waveform overview into `on_levels`; dropping it stops the decode.
pub(super) struct PeaksLoad(Timer);

impl Drop for PeaksLoad {
    fn drop(&mut self) {
        if let Some(timer) = self.0.borrow_mut().take() {
            timer.remove();
        }
    }
}

pub(super) fn load_peaks(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_levels: impl Fn(u32, &[u8]) + 'static,
) -> PeaksLoad {
    let key = TrackKey::of(entry);
    if let Some(levels) = PEAKS.with_borrow_mut(|cache| cache.get(&key)) {
        on_levels(0, &levels);
        return PeaksLoad(Timer::default());
    }
    let source = source.clone();
    let started = Instant::now();
    let session: RefCell<Option<PeaksSession>> = RefCell::new(None);
    let levels = RefCell::new(Vec::with_capacity(BUCKETS as usize));
    let handle = Timer::default();
    let finished = handle.clone();
    let stop = move || {
        finished.borrow_mut().take();
        glib::ControlFlow::Break
    };
    let timer = glib::timeout_add_local(PEAKS_POLL, move || {
        if started.elapsed() < PEAKS_SETTLE {
            return glib::ControlFlow::Continue;
        }
        let mut session = session.borrow_mut();
        // Another overview holds the single slot; wait while this track is shown.
        if session.is_none() {
            *session = PeaksSession::start(source.clone());
            return glib::ControlFlow::Continue;
        }
        while let Some(event) = session.as_ref().and_then(PeaksSession::receive) {
            match event {
                PeaksEvent::Levels { start, levels: run } => {
                    on_levels(start, &run);
                    levels.borrow_mut().extend_from_slice(&run);
                }
                PeaksEvent::Finished => {
                    let levels = Rc::new(levels.take());
                    PEAKS.with_borrow_mut(|cache| cache.insert(key.clone(), levels));
                    return stop();
                }
                PeaksEvent::Failed => return stop(),
            }
        }
        glib::ControlFlow::Continue
    });
    handle.replace(Some(timer));
    PeaksLoad(handle)
}
