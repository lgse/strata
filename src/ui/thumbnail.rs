// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    model::{FileEntry, MetadataValue},
    sandbox::{Cancellation, ParseOperation},
};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

thread_local! {
    static ACTIVE_REQUESTS: RefCell<HashMap<usize, ActiveRequest>> =
        RefCell::new(HashMap::new());
    static THUMBNAIL_CACHE: RefCell<ThumbnailCache> = RefCell::new(ThumbnailCache::default());
}

struct ActiveRequest {
    id: u64,
    image: glib::WeakRef<gtk::Image>,
    cancellation: Cancellation,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ThumbnailKey {
    path: PathBuf,
    modified: Option<i64>,
    file_size: Option<u64>,
    thumbnail_size: i32,
}

#[derive(Default)]
struct ThumbnailCache {
    entries: HashMap<ThumbnailKey, glib::Bytes>,
    recent: VecDeque<ThumbnailKey>,
    byte_count: usize,
}

impl ThumbnailCache {
    fn get(&mut self, key: &ThumbnailKey) -> Option<glib::Bytes> {
        let bytes = self.entries.get(key)?.clone();
        self.recent.retain(|candidate| candidate != key);
        self.recent.push_back(key.clone());
        Some(bytes)
    }

    fn insert(&mut self, key: ThumbnailKey, bytes: glib::Bytes) {
        if let Some(previous) = self.entries.remove(&key) {
            self.byte_count = self.byte_count.saturating_sub(previous.len());
        }
        self.recent.retain(|candidate| candidate != &key);
        self.byte_count = self.byte_count.saturating_add(bytes.len());
        self.recent.push_back(key.clone());
        self.entries.insert(key, bytes);
        while self.entries.len() > MAX_CACHE_ENTRIES || self.byte_count > MAX_CACHE_BYTES {
            let Some(oldest) = self.recent.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.byte_count = self.byte_count.saturating_sub(removed.len());
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThumbnailKind {
    Image,
    RawImage,
    Pdf,
    Video,
}

pub(super) fn set_thumbnail_or_icon(
    image: &gtk::Image,
    entry: &FileEntry,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    let Some(path) = entry.location.native_path() else {
        show_fallback_icon(image, fallback_icon, icon_size);
        return;
    };
    set_thumbnail_for_path(
        image,
        path,
        known_metadata(&entry.modified_unix_seconds),
        known_metadata(&entry.size),
        fallback_icon,
        icon_size,
        thumbnail_size,
    );
}

pub(super) fn set_thumbnail_or_icon_for_path(
    image: &gtk::Image,
    path: &Path,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    set_thumbnail_for_path(
        image,
        path,
        None,
        None,
        fallback_icon,
        icon_size,
        thumbnail_size,
    );
}

fn set_thumbnail_for_path(
    image: &gtk::Image,
    path: &Path,
    modified: Option<i64>,
    file_size: Option<u64>,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    let (image_id, request, cancellation) = set_fallback_icon(image, fallback_icon, icon_size);
    let path = path.to_path_buf();
    let Some(kind) = thumbnail_kind(&path) else {
        return;
    };
    let thumbnail_size = thumbnail_size.clamp(16, 256);
    let key = ThumbnailKey {
        path: path.clone(),
        modified,
        file_size,
        thumbnail_size,
    };
    if let Some(bytes) = THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&key)) {
        apply_thumbnail(image, &bytes, thumbnail_size);
        return;
    }

    let weak_image = glib::WeakRef::new();
    weak_image.set(Some(image));
    glib::MainContext::default().spawn_local(async move {
        let result = gio::spawn_blocking(move || {
            render_thumbnail(&path, kind, thumbnail_size, &cancellation)
        })
        .await;
        let Ok(Ok(png)) = result else {
            return;
        };
        let bytes = glib::Bytes::from_owned(png);
        THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert(key, bytes.clone()));
        let is_current = ACTIVE_REQUESTS.with(|requests| {
            requests
                .borrow()
                .get(&image_id)
                .is_some_and(|active| active.id == request)
        });
        if !is_current {
            return;
        }
        let Some(image) = weak_image.upgrade() else {
            ACTIVE_REQUESTS.with(|requests| {
                requests.borrow_mut().remove(&image_id);
            });
            return;
        };
        apply_thumbnail(&image, &bytes, thumbnail_size);
    });
}

fn known_metadata<T: Copy>(value: &MetadataValue<T>) -> Option<T> {
    match value {
        MetadataValue::Known(value) => Some(*value),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

fn apply_thumbnail(image: &gtk::Image, bytes: &glib::Bytes, thumbnail_size: i32) {
    if let Ok(texture) = gdk::Texture::from_bytes(bytes) {
        crate::assets::remove_primary_icon(image);
        image.set_pixel_size(thumbnail_size);
        image.set_size_request(thumbnail_size, thumbnail_size);
        image.set_paintable(Some(&texture));
        image.set_opacity(1.0);
    }
}

pub(super) fn show_fallback_icon(image: &gtk::Image, icon: &str, size: i32) {
    set_fallback_icon(image, icon, size);
}

fn set_fallback_icon(image: &gtk::Image, icon: &str, size: i32) -> (usize, u64, Cancellation) {
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let image_id = image.as_ptr() as usize;
    let weak_image = glib::WeakRef::new();
    weak_image.set(Some(image));
    let cancellation = Cancellation::default();
    ACTIVE_REQUESTS.with(|requests| {
        let mut requests = requests.borrow_mut();
        requests.retain(|_, active| active.image.upgrade().is_some());
        if let Some(previous) = requests.insert(
            image_id,
            ActiveRequest {
                id: request,
                image: weak_image,
                cancellation: cancellation.clone(),
            },
        ) {
            previous.cancellation.cancel();
        }
    });
    image.set_pixel_size(size);
    image.set_size_request(size, size);
    crate::assets::set_primary_icon(image, icon);
    (image_id, request, cancellation)
}

fn thumbnail_kind(path: &Path) -> Option<ThumbnailKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" => {
            Some(ThumbnailKind::Image)
        }
        "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf" | "kdc" | "mef" | "mos" | "mrw"
        | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2" | "rwl" | "sr2" | "srf" | "srw"
        | "x3f" => Some(ThumbnailKind::RawImage),
        "pdf" => Some(ThumbnailKind::Pdf),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "ogv" => {
            Some(ThumbnailKind::Video)
        }
        _ => None,
    }
}

fn render_thumbnail(
    path: &Path,
    kind: ThumbnailKind,
    size: i32,
    cancellation: &Cancellation,
) -> Result<Vec<u8>, String> {
    let operation = match kind {
        ThumbnailKind::Image => ParseOperation::ThumbnailImage,
        ThumbnailKind::RawImage => ParseOperation::ThumbnailRaw,
        ThumbnailKind::Pdf => ParseOperation::ThumbnailPdf,
        ThumbnailKind::Video => ParseOperation::ThumbnailVideo,
    };
    crate::sandbox::parse(path, operation, size.clamp(16, 256), cancellation)
        .map(|output| output.data)
}

#[cfg(test)]
mod tests;
