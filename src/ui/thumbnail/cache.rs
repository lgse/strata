// SPDX-License-Identifier: MIT

//! The in-memory thumbnail cache: recent textures and short-lived failures,
//! bounded by entry count and texture bytes.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use gtk::{gdk, prelude::*};

use super::{ThumbnailKey, known_metadata};
use crate::model::FileEntry;

thread_local! {
    pub(super) static THUMBNAIL_CACHE: RefCell<ThumbnailCache> = RefCell::new(ThumbnailCache::default());
}

const MAX_CACHE_ENTRIES: usize = 256;

const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

const FAILED_THUMBNAIL_TTL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct ThumbnailCache {
    entries: HashMap<ThumbnailKey, CachedThumbnail>,
    recent: VecDeque<ThumbnailKey>,
    byte_count: usize,
}

#[derive(Clone)]
enum CachedThumbnail {
    Ready(gdk::Texture),
    Failed(Instant),
}

pub(super) enum CacheHit {
    Ready(gdk::Texture),
    Failed,
}

impl ThumbnailCache {
    pub(super) fn get(&mut self, key: &ThumbnailKey) -> Option<CacheHit> {
        let entry = self.entries.get(key)?.clone();
        if matches!(entry, CachedThumbnail::Failed(expires) if expires <= Instant::now()) {
            self.remove(key);
            return None;
        }
        self.recent.retain(|candidate| candidate != key);
        self.recent.push_back(key.clone());
        Some(match entry {
            CachedThumbnail::Ready(texture) => CacheHit::Ready(texture),
            CachedThumbnail::Failed(_) => CacheHit::Failed,
        })
    }

    pub(super) fn insert(&mut self, key: ThumbnailKey, texture: gdk::Texture) {
        self.insert_entry(key, CachedThumbnail::Ready(texture));
    }

    pub(super) fn insert_failure(&mut self, key: ThumbnailKey) {
        self.insert_entry(
            key,
            CachedThumbnail::Failed(Instant::now() + FAILED_THUMBNAIL_TTL),
        );
    }

    fn insert_entry(&mut self, key: ThumbnailKey, entry: CachedThumbnail) {
        self.remove(&key);
        self.byte_count = self.byte_count.saturating_add(entry.byte_len());
        self.recent.push_back(key.clone());
        self.entries.insert(key, entry);
        while self.entries.len() > MAX_CACHE_ENTRIES || self.byte_count > MAX_CACHE_BYTES {
            let Some(oldest) = self.recent.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.byte_count = self.byte_count.saturating_sub(removed.byte_len());
            }
        }
    }

    pub(super) fn remove(&mut self, key: &ThumbnailKey) {
        if let Some(removed) = self.entries.remove(key) {
            self.byte_count = self.byte_count.saturating_sub(removed.byte_len());
        }
        self.recent.retain(|candidate| candidate != key);
    }
}

impl CachedThumbnail {
    fn byte_len(&self) -> usize {
        match self {
            Self::Ready(texture) => {
                (texture.width().max(0) as usize).saturating_mul(texture.height().max(0) as usize)
                    * 4
            }
            Self::Failed(_) => 0,
        }
    }
}

/// Cache-only lookup: a placeholder must not queue thumbnail work.
pub(in crate::ui) fn cached_thumbnail(entry: &FileEntry) -> Option<gdk::Texture> {
    let key = ThumbnailKey {
        path: entry.local_thumbnail_path()?.to_path_buf(),
        modified: known_metadata(&entry.modified_unix_seconds),
        file_size: known_metadata(&entry.size),
        thumbnail_size: super::super::thumbnail_cache::CANONICAL_MAX_EDGE,
    };
    match THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&key)) {
        Some(CacheHit::Ready(texture)) => Some(texture),
        _ => None,
    }
}

#[cfg(test)]
pub(in crate::ui) fn remember_thumbnail_for_test(entry: &FileEntry, texture: gdk::Texture) {
    let key = ThumbnailKey {
        path: entry
            .local_thumbnail_path()
            .expect("local entry")
            .to_path_buf(),
        modified: known_metadata(&entry.modified_unix_seconds),
        file_size: known_metadata(&entry.size),
        thumbnail_size: super::super::thumbnail_cache::CANONICAL_MAX_EDGE,
    };
    THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert(key, texture));
}

pub(in crate::ui) fn preserve_renamed_thumbnail(from: &crate::model::Location, entry: &FileEntry) {
    let Some((from, to)) = from.native_path().zip(entry.location.native_path()) else {
        return;
    };
    if from == to || from.extension() != to.extension() {
        return;
    }
    let (Some(modified), Some(file_size)) = (
        known_metadata(&entry.modified_unix_seconds),
        known_metadata(&entry.size),
    ) else {
        return;
    };
    THUMBNAIL_CACHE.with_borrow_mut(|cache| {
        let key = ThumbnailKey {
            path: from.to_path_buf(),
            modified: Some(modified),
            file_size: Some(file_size),
            thumbnail_size: super::super::thumbnail_cache::CANONICAL_MAX_EDGE,
        };
        if let Some(CacheHit::Ready(texture)) = cache.get(&key) {
            cache.remove(&key);
            cache.insert(
                ThumbnailKey {
                    path: to.to_path_buf(),
                    ..key
                },
                texture,
            );
        }
    });
}
