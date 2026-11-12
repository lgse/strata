// SPDX-License-Identifier: MIT
//! Icon details (dimensions, child counts, durations) remembered per file fingerprint.

use super::*;

pub(super) const MAX_ICON_DETAILS_CACHE_ENTRIES: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct IconDetailsFingerprint {
    pub(super) size: u64,
    pub(super) modified_seconds: i64,
    pub(super) modified_nanoseconds: i64,
    pub(super) changed_seconds: i64,
    pub(super) changed_nanoseconds: i64,
}

impl IconDetailsFingerprint {
    pub(super) fn read(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        Some(Self {
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }

    pub(super) fn of(target: &FolderStamp) -> Self {
        Self {
            size: target.size,
            modified_seconds: target.modified.0,
            modified_nanoseconds: i64::from(target.modified.1),
            changed_seconds: target.changed.0,
            changed_nanoseconds: i64::from(target.changed.1),
        }
    }
}

#[derive(Clone)]
pub(super) struct IconDetails {
    pub(super) image_dimensions: MetadataValue<(u32, u32)>,
    pub(super) child_count: MetadataValue<u64>,
    pub(super) duration_seconds: MetadataValue<u64>,
}

impl IconDetails {
    pub(super) fn from_update(update: &MetadataUpdate) -> Self {
        Self {
            image_dimensions: update.image_dimensions.clone(),
            child_count: update.child_count.clone(),
            duration_seconds: update.duration_seconds.clone(),
        }
    }

    pub(super) fn apply_to(&self, update: &mut MetadataUpdate) {
        update.image_dimensions = self.image_dimensions.clone();
        update.child_count = self.child_count.clone();
        update.duration_seconds = self.duration_seconds.clone();
    }

    pub(super) fn apply_to_entry(&self, entry: &mut FileEntry) {
        entry.image_dimensions = self.image_dimensions.clone();
        entry.child_count = self.child_count.clone();
        entry.duration_seconds = self.duration_seconds.clone();
    }
}

pub(super) struct CachedIconDetails {
    pub(super) fingerprint: IconDetailsFingerprint,
    pub(super) details: IconDetails,
    pub(super) generation: u64,
}

#[derive(Default)]
pub(super) struct IconDetailsCache {
    pub(super) entries: HashMap<PathBuf, CachedIconDetails>,
    pub(super) recent: VecDeque<(PathBuf, u64)>,
    pub(super) generation: u64,
}

impl IconDetailsCache {
    pub(super) fn get(
        &mut self,
        path: &Path,
        fingerprint: IconDetailsFingerprint,
    ) -> Option<IconDetails> {
        if self
            .entries
            .get(path)
            .is_some_and(|cached| cached.fingerprint != fingerprint)
        {
            self.entries.remove(path);
            return None;
        }
        let cached = self.entries.get_mut(path)?;
        self.generation = self.generation.saturating_add(1);
        cached.generation = self.generation;
        self.recent.push_back((path.to_path_buf(), self.generation));
        let details = cached.details.clone();
        self.compact_recent();
        Some(details)
    }

    pub(super) fn insert(
        &mut self,
        path: PathBuf,
        fingerprint: IconDetailsFingerprint,
        details: IconDetails,
    ) {
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        self.entries.insert(
            path.clone(),
            CachedIconDetails {
                fingerprint,
                details,
                generation,
            },
        );
        self.recent.push_back((path, generation));
        while self.entries.len() > MAX_ICON_DETAILS_CACHE_ENTRIES {
            let Some((oldest_path, oldest_generation)) = self.recent.pop_front() else {
                break;
            };
            if self
                .entries
                .get(&oldest_path)
                .is_some_and(|cached| cached.generation == oldest_generation)
            {
                self.entries.remove(&oldest_path);
            }
        }
        self.compact_recent();
    }

    pub(super) fn compact_recent(&mut self) {
        if self.recent.len() > MAX_ICON_DETAILS_CACHE_ENTRIES * 4 {
            self.recent.retain(|(path, generation)| {
                self.entries
                    .get(path)
                    .is_some_and(|cached| cached.generation == *generation)
            });
        }
    }
}

pub(super) fn icon_details_cache() -> &'static Mutex<IconDetailsCache> {
    static CACHE: OnceLock<Mutex<IconDetailsCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(IconDetailsCache::default()))
}

pub(super) fn cached_icon_details(
    path: &Path,
    fingerprint: IconDetailsFingerprint,
) -> Option<IconDetails> {
    icon_details_cache().lock().ok()?.get(path, fingerprint)
}

pub(super) static CACHE_HAS_ENTRIES: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(super) fn revisits(path: &Path) -> bool {
    CACHE_HAS_ENTRIES.load(std::sync::atomic::Ordering::Acquire)
        && icon_details_cache()
            .lock()
            .is_ok_and(|cache| cache.entries.contains_key(path))
}

pub(super) fn cached_icon_details_for_revisit(path: &Path) -> Option<IconDetails> {
    if !revisits(path) {
        return None;
    }
    cached_icon_details(path, IconDetailsFingerprint::read(path)?)
}

pub(super) fn cache_icon_details(
    path: &Path,
    fingerprint: Option<IconDetailsFingerprint>,
    update: &MetadataUpdate,
) {
    let Some(fingerprint) = fingerprint else {
        return;
    };
    if let Ok(mut cache) = icon_details_cache().lock() {
        cache.insert(
            path.to_path_buf(),
            fingerprint,
            IconDetails::from_update(update),
        );
        CACHE_HAS_ENTRIES.store(true, std::sync::atomic::Ordering::Release);
    }
}
