// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::hash_map::DefaultHasher,
    hash::Hasher,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use rune::{Pixels, Tx};
use strata_components::media_player::{Art, AudioTags};
use strata_core::{file_activity, model::FileEntry};
use strata_fs::services::SandboxedMedia;
use strata_preview::sandbox::{
    self, Cancellation, MediaPreviewBackend, ParseOperation, metadata::AudioTags as SandboxedTags,
};

use super::cache::{Lru, TrackKey};
use crate::{App, slots::Later};

const DETAILS_CACHE: usize = 12;
/// Fast browsing must not start a tags/cover decode for every track it passes.
const LOAD_SETTLE: Duration = Duration::from_millis(50);
const FOLDER_ART_STEMS: [&str; 5] = ["cover", "folder", "front", "album", "albumart"];
const FOLDER_ART_EXTENSIONS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];
const MAX_GPU_EDGE: u32 = 8192;

/// Cached tags and album art for one track.
#[derive(Clone, Default)]
pub(super) struct CachedDetails {
    pub(super) tags: AudioTags,
    pub(super) art: Option<Art>,
}

/// Incremental details updates delivered into Rune through `Later`.
pub(super) enum Update {
    Tags(AudioTags),
    Cover(Option<Art>),
}

/// Where audio details updates go: the viewer id and delivery callback on `Later`.
#[derive(Clone)]
pub(super) struct Inlet {
    pub(super) later: Later<App>,
    pub(super) viewer: u64,
    pub(super) deliver: fn(&mut App, u64, Update, &mut Tx) -> bool,
}

/// Active sandboxed audio tags and cover jobs; dropping cancels both immediately.
pub(super) struct AudioDetailsLoad(Cancellation, Cancellation);

impl Drop for AudioDetailsLoad {
    fn drop(&mut self) {
        self.0.cancel();
        self.1.cancel();
    }
}

/// Decoded RGBA8 cover bytes ready to upload on the UI thread, tagged with its content hash.
#[derive(Clone)]
struct DecodedCover {
    id: u64,
    size: [u32; 2],
    rgba: Vec<u8>,
}

impl DecodedCover {
    fn to_art(&self) -> Art {
        let pixels = Pixels::new();
        pixels.set(self.size[0], self.size[1], self.rgba.clone());
        Art {
            id: self.id,
            pixels,
            size: self.size,
        }
    }
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
    cover: Arc<DecodedCover>,
}

thread_local! {
    static DETAILS: RefCell<Lru<TrackKey, CachedDetails>> =
        const { RefCell::new(Lru::new(DETAILS_CACHE)) };
}

static FOLDER_CACHE: OnceLock<Mutex<Lru<PathBuf, FolderArt>>> = OnceLock::new();

fn folder_cache() -> &'static Mutex<Lru<PathBuf, FolderArt>> {
    FOLDER_CACHE.get_or_init(|| Mutex::new(Lru::new(DETAILS_CACHE)))
}

#[cfg(test)]
pub(in crate::preview) type ParseFn =
    Arc<dyn Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>> + Send + Sync>;

#[cfg(test)]
thread_local! {
    /// Test hook replacing sandbox parsing so unit and offscreen tests need no host helpers.
    pub(in crate::preview) static PARSE: RefCell<Option<ParseFn>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(in crate::preview) fn clear_cache() {
    DETAILS.with_borrow_mut(Lru::clear);
    folder_cache().lock().expect("folder artwork cache").clear();
}

/// Cached tags and album art for `entry`, if previously decoded cleanly.
pub(super) fn cached(entry: &FileEntry) -> Option<CachedDetails> {
    let key = TrackKey::of(entry);
    DETAILS.with_borrow_mut(|cache| cache.get(&key))
}

fn sandbox_parse(
    path: &Path,
    operation: ParseOperation,
    cancellation: &Cancellation,
) -> Option<Vec<u8>> {
    sandbox::parse(
        path,
        operation,
        0,
        MediaPreviewBackend::Software,
        cancellation,
    )
    .ok()
    .map(|output| output.data)
}

fn content_id(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn decode_cover(bytes: &[u8]) -> Option<Arc<DecodedCover>> {
    let id = content_id(bytes);
    let decoded = image::load_from_memory(bytes).ok()?;
    let (w, h) = (decoded.width().max(1), decoded.height().max(1));
    let fitted = if w > MAX_GPU_EDGE || h > MAX_GPU_EDGE {
        decoded.resize(
            MAX_GPU_EDGE,
            MAX_GPU_EDGE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        decoded
    };
    let rgba = fitted.to_rgba8();
    let size = [rgba.width(), rgba.height()];
    Some(Arc::new(DecodedCover {
        id,
        size,
        rgba: rgba.into_raw(),
    }))
}

fn convert_tags(tags: SandboxedTags) -> AudioTags {
    AudioTags {
        title: tags.title,
        artist: tags.artist,
        album: tags.album,
        track: tags.track,
        track_total: tags.track_total,
    }
}

fn folder_art(directory: &Path) -> std::io::Result<Option<PathBuf>> {
    let mut best = None;
    for entry in file_activity::list(directory)? {
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

/// Rescan: coarse directory timestamps can miss cover additions and removals.
fn load_folder_art(
    directory: &Path,
    job: &Cancellation,
    decode: &Cancellation,
    parse: &impl Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>>,
) -> Option<Option<Arc<DecodedCover>>> {
    let Some(path) = folder_art(directory).ok()? else {
        return Some(None);
    };
    let version = FileVersion::read(&path)?;
    let cached = folder_cache()
        .lock()
        .expect("folder artwork cache")
        .get(&path);
    if let Some(cached) = cached.filter(|cached| cached.version == version) {
        return Some(Some(cached.cover));
    }
    let cover = decode_cover(&parse(&path, ParseOperation::PreviewImage, decode)?)?;
    if !job.is_cancelled() {
        folder_cache().lock().expect("folder artwork cache").insert(
            path,
            FolderArt {
                version,
                cover: cover.clone(),
            },
        );
    }
    Some(Some(cover))
}

fn probe_cover(
    path: &Path,
    directory: Option<&Path>,
    job: &Cancellation,
    decode: &Cancellation,
    parse: &impl Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>>,
) -> (Option<Arc<DecodedCover>>, bool) {
    let embedded = parse(path, ParseOperation::AudioCover, job).and_then(|bytes| {
        if bytes == b"null" {
            Some(None)
        } else {
            decode_cover(&bytes).map(Some)
        }
    });
    if let Some(Some(cover)) = embedded.as_ref() {
        return (Some(cover.clone()), true);
    }
    let folder = directory.map_or(Some(None), |dir| load_folder_art(dir, job, decode, parse));
    let completed = embedded.is_some() && folder.is_some();
    (folder.flatten(), completed)
}

struct Completion {
    key: TrackKey,
    cancellation: Cancellation,
    details: RefCell<CachedDetails>,
    pending: Cell<u8>,
    completed: Cell<bool>,
}

impl Completion {
    fn new(key: TrackKey, cancellation: Cancellation) -> Rc<Self> {
        Rc::new(Self {
            key,
            cancellation,
            details: RefCell::new(CachedDetails::default()),
            pending: Cell::new(2),
            completed: Cell::new(true),
        })
    }

    fn finish_tags(&self, tags: AudioTags, ok: bool) {
        self.details.borrow_mut().tags = tags;
        self.finish(ok);
    }

    fn finish_cover(&self, art: Option<Art>, ok: bool) {
        self.details.borrow_mut().art = art;
        self.finish(ok);
    }

    fn finish(&self, ok: bool) {
        self.completed.set(self.completed.get() && ok);
        self.pending.set(self.pending.get() - 1);
        if self.pending.get() == 0 && self.completed.get() && !self.cancellation.is_cancelled() {
            let entry = self.details.borrow().clone();
            let key = self.key.clone();
            DETAILS.with_borrow_mut(|cache| cache.insert(key, entry));
        }
    }
}

/// Probes tags and cover art for `entry` after `LOAD_SETTLE`, delivering updates via `inlet`.
pub(super) fn load(entry: &FileEntry, source: &SandboxedMedia, inlet: Inlet) -> AudioDetailsLoad {
    #[cfg(test)]
    {
        if let Some(hook) = PARSE.with_borrow(Clone::clone) {
            return load_with(entry, source, inlet, move |path, op, cancel| {
                hook(path, op, cancel)
            });
        }
        if super::stream::START.get().is_some() {
            return AudioDetailsLoad(Cancellation::default(), Cancellation::default());
        }
    }
    load_with(entry, source, inlet, sandbox_parse)
}

fn load_with<F>(
    entry: &FileEntry,
    source: &SandboxedMedia,
    inlet: Inlet,
    parse: F,
) -> AudioDetailsLoad
where
    F: Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>> + Clone + Send + 'static,
{
    let cancellation = Cancellation::default();
    let folder_cover = Cancellation::default();
    let completion = Completion::new(TrackKey::of(entry), cancellation.clone());
    spawn_tags(source, &cancellation, &completion, &inlet, parse.clone());
    let directory = entry
        .location
        .native_path()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    spawn_cover(
        source,
        directory,
        (&cancellation, &folder_cover),
        &completion,
        (&inlet, parse),
    );
    AudioDetailsLoad(cancellation, folder_cover)
}

fn spawn_tags<F>(
    source: &SandboxedMedia,
    cancellation: &Cancellation,
    completion: &Rc<Completion>,
    inlet: &Inlet,
    parse: F,
) where
    F: Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>> + Send + 'static,
{
    let path = source.path.clone();
    let lease = source.clone();
    let job = cancellation.clone();
    let cancelled = cancellation.clone();
    let completion = completion.clone();
    let inlet = inlet.clone();
    glib::MainContext::default().spawn_local(async move {
        glib::timeout_future(LOAD_SETTLE).await;
        if cancelled.is_cancelled() {
            return;
        }
        let probed = gio::spawn_blocking(move || {
            let _lease = lease;
            parse(&path, ParseOperation::AudioTags, &job)
                .and_then(|json| SandboxedTags::from_json(&json).ok())
                .map(convert_tags)
        })
        .await
        .ok()
        .flatten();
        if cancelled.is_cancelled() {
            return;
        }
        let ok = probed.is_some();
        let tags = probed.unwrap_or_default();
        completion.finish_tags(tags.clone(), ok);
        let (viewer, deliver) = (inlet.viewer, inlet.deliver);
        inlet
            .later
            .then(move |app: &mut App, tx: &mut Tx| deliver(app, viewer, Update::Tags(tags), tx));
    });
}

fn spawn_cover<F>(
    source: &SandboxedMedia,
    directory: Option<PathBuf>,
    (cancellation, folder_cover): (&Cancellation, &Cancellation),
    completion: &Rc<Completion>,
    (inlet, parse): (&Inlet, F),
) where
    F: Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>> + Send + 'static,
{
    let path = source.path.clone();
    let lease = source.clone();
    let job = cancellation.clone();
    let cancelled = cancellation.clone();
    let decode = folder_cover.clone();
    let completion = completion.clone();
    let inlet = inlet.clone();
    glib::MainContext::default().spawn_local(async move {
        glib::timeout_future(LOAD_SETTLE).await;
        if cancelled.is_cancelled() {
            return;
        }
        let (cover, ok) = gio::spawn_blocking(move || {
            let _lease = lease;
            probe_cover(&path, directory.as_deref(), &job, &decode, &parse)
        })
        .await
        .unwrap_or((None, false));
        if cancelled.is_cancelled() {
            return;
        }
        let art = cover.map(|decoded| decoded.to_art());
        completion.finish_cover(art.clone(), ok);
        let (viewer, deliver) = (inlet.viewer, inlet.deliver);
        inlet
            .later
            .then(move |app: &mut App, tx: &mut Tx| deliver(app, viewer, Update::Cover(art), tx));
    });
}

#[cfg(test)]
mod tests;
