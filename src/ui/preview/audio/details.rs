// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Mutex, OnceLock},
    time::Duration,
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
const LOAD_SETTLE: Duration = Duration::from_millis(50);
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

struct Lru<K, V> {
    capacity: usize,
    entries: VecDeque<(K, V)>,
}

impl<K: PartialEq, V: Clone> Lru<K, V> {
    const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    fn get(&mut self, key: &K) -> Option<V> {
        let index = self.entries.iter().position(|(entry, _)| entry == key)?;
        let entry = self.entries.remove(index)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }

    fn insert(&mut self, key: K, value: V) {
        self.entries.retain(|(entry, _)| *entry != key);
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((key, value));
    }
}

#[derive(Clone)]
pub(super) struct Cover {
    pub(super) texture: gdk::Texture,
    digest: [u8; 32],
}

impl PartialEq for Cover {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
    }
}

#[derive(Clone, Default)]
pub(super) struct Details {
    pub(super) tags: AudioTags,
    pub(super) cover: Option<Cover>,
}

thread_local! {
    static DETAILS: RefCell<Lru<TrackKey, Rc<Details>>> = const { RefCell::new(Lru::new(DETAILS_CACHE)) };
    static PEAKS: RefCell<Lru<TrackKey, Rc<Vec<u8>>>> = const { RefCell::new(Lru::new(PEAKS_CACHE)) };
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

fn folder_art(directory: &Path) -> std::io::Result<Option<PathBuf>> {
    let mut best = None;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            continue;
        };
        let Some(rank) = FOLDER_ART_STEMS
            .iter()
            .position(|name| name.eq_ignore_ascii_case(stem))
        else {
            continue;
        };
        if FOLDER_ART_EXTENSIONS
            .iter()
            .any(|name| name.eq_ignore_ascii_case(extension))
            && entry.file_type()?.is_file()
        {
            let candidate = (rank, path);
            if best.as_ref().is_none_or(|best| candidate < *best) {
                best = Some(candidate);
            }
        }
    }
    Ok(best.map(|(_, path)| path))
}

fn texture(png: Vec<u8>) -> Option<Cover> {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&png).into();
    let texture = gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)).ok()?;
    Some(Cover { texture, digest })
}

#[derive(Clone, PartialEq)]
struct FileVersion(u64, u64, u64, i64, i64, i64, i64);

impl FileVersion {
    fn read(path: &Path) -> Option<Self> {
        let info = std::fs::metadata(path).ok()?;
        Some(Self(
            info.dev(),
            info.ino(),
            info.len(),
            info.mtime(),
            info.mtime_nsec(),
            info.ctime(),
            info.ctime_nsec(),
        ))
    }
}

#[derive(Clone)]
struct FolderArt {
    version: FileVersion,
    cover: Cover,
}

/// The folder is rescanned every time: a directory's timestamps can miss an
/// added or removed cover within one coarse timestamp tick.
fn load_folder_art(
    directory: &Path,
    job: &Cancellation,
    parse: &impl Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>>,
) -> Option<Option<Cover>> {
    static CACHE: OnceLock<Mutex<Lru<PathBuf, FolderArt>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Lru::new(DETAILS_CACHE)));
    let Some(path) = folder_art(directory).ok()? else {
        return Some(None);
    };
    let version = FileVersion::read(&path)?;
    let cached = cache.lock().expect("folder artwork cache").get(&path);
    if let Some(cached) = cached.filter(|cached| cached.version == version) {
        return Some(Some(cached.cover));
    }
    let cover = texture(parse(&path, ParseOperation::PreviewImage, job)?)?;
    if !job.is_cancelled() {
        cache.lock().expect("folder artwork cache").insert(
            path,
            FolderArt {
                version,
                cover: cover.clone(),
            },
        );
    }
    Some(Some(cover))
}

/// Tags arrive first; embedded artwork, then folder artwork, follow.
pub(super) fn load_details(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_tags: impl Fn(AudioTags) + 'static,
    on_cover: impl Fn(Option<Cover>) + 'static,
) -> DetailsLoad {
    load_details_with(entry, source, on_tags, on_cover, parse)
}

fn load_details_with(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_tags: impl Fn(AudioTags) + 'static,
    on_cover: impl Fn(Option<Cover>) + 'static,
    parse: impl Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>> + Clone + Send + 'static,
) -> DetailsLoad {
    let key = TrackKey::of(entry);
    let cancellation = Cancellation::default();
    let details = Rc::new(RefCell::new(Details::default()));
    let pending = Rc::new(Cell::new(2));
    let completed = Cell::new(true);
    let finish = {
        let details = details.clone();
        let cancellation = cancellation.clone();
        move |success: bool| {
            completed.set(completed.get() && success);
            pending.set(pending.get() - 1);
            if pending.get() == 0 && completed.get() && !cancellation.is_cancelled() {
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
    let parse_tags = parse.clone();
    glib::MainContext::default().spawn_local(async move {
        glib::timeout_future(LOAD_SETTLE).await;
        if cancelled.is_cancelled() {
            return;
        }
        let tags = gio::spawn_blocking(move || {
            let _lease = lease;
            parse_tags(&path, ParseOperation::AudioTags, &job)
                .and_then(|json| AudioTags::from_json(&json).ok())
        })
        .await
        .ok()
        .flatten();
        // A reused view may already show another track.
        if cancelled.is_cancelled() {
            return;
        }
        let completed = tags.is_some();
        let tags = tags.unwrap_or_default();
        tags_details.borrow_mut().tags = tags.clone();
        tags_finish(completed);
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
        glib::timeout_future(LOAD_SETTLE).await;
        if cancelled.is_cancelled() {
            return;
        }
        let (cover, completed) = gio::spawn_blocking(move || {
            let _lease = lease;
            let embedded = parse(&path, ParseOperation::AudioCover, &job).and_then(|bytes| {
                if bytes == b"null" {
                    Some(None)
                } else {
                    texture(bytes).map(Some)
                }
            });
            if let Some(Some(cover)) = embedded.as_ref() {
                return (Some(cover.clone()), true);
            }
            let folder = directory.as_deref().map_or(Some(None), |directory| {
                load_folder_art(directory, &job, &parse)
            });
            let completed = embedded.is_some() && folder.is_some();
            (folder.flatten(), completed)
        })
        .await
        .unwrap_or((None, false));
        if cancelled.is_cancelled() {
            return;
        }
        details.borrow_mut().cover = cover.clone();
        finish(completed);
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
    let session: RefCell<Option<PeaksSession>> = RefCell::new(None);
    let levels = RefCell::new(Vec::with_capacity(BUCKETS as usize));
    let handle = Timer::default();
    let finished = handle.clone();
    let stop = move || {
        finished.borrow_mut().take();
        glib::ControlFlow::Break
    };
    let polling = handle.clone();
    let timer = glib::timeout_add_local_once(LOAD_SETTLE, move || {
        *session.borrow_mut() = PeaksSession::start(source.clone());
        let timer = glib::timeout_add_local(PEAKS_POLL, move || {
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
        polling.replace(Some(timer));
    });
    handle.replace(Some(timer));
    PeaksLoad(handle)
}

#[cfg(test)]
mod tests;
