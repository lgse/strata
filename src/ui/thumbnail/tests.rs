// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use gtk::glib;

use super::{MAX_CACHE_ENTRIES, ThumbnailCache, ThumbnailKey, ThumbnailKind, thumbnail_kind};

#[test]
fn recognizes_mainstream_image_and_video_formats() {
    assert_eq!(
        thumbnail_kind(Path::new("photo.JPEG")),
        Some(ThumbnailKind::Image)
    );
    assert_eq!(
        thumbnail_kind(Path::new("animation.webp")),
        Some(ThumbnailKind::Image)
    );
    assert_eq!(
        thumbnail_kind(Path::new("capture.CR3")),
        Some(ThumbnailKind::RawImage)
    );
    assert_eq!(
        thumbnail_kind(Path::new("photo.nef")),
        Some(ThumbnailKind::RawImage)
    );
    assert_eq!(
        thumbnail_kind(Path::new("document.PDF")),
        Some(ThumbnailKind::Pdf)
    );
    assert_eq!(
        thumbnail_kind(Path::new("clip.mkv")),
        Some(ThumbnailKind::Video)
    );
    assert_eq!(
        thumbnail_kind(Path::new("clip.ogv")),
        Some(ThumbnailKind::Video)
    );
}

#[test]
fn thumbnail_cache_evicts_the_least_recent_entry() {
    let mut cache = ThumbnailCache::default();
    for index in 0..=MAX_CACHE_ENTRIES {
        cache.insert(
            ThumbnailKey {
                path: PathBuf::from(format!("image-{index}.png")),
                modified: Some(1),
                file_size: Some(1),
                thumbnail_size: 64,
            },
            glib::Bytes::from_static(&[1]),
        );
    }

    let oldest = ThumbnailKey {
        path: PathBuf::from("image-0.png"),
        modified: Some(1),
        file_size: Some(1),
        thumbnail_size: 64,
    };
    assert!(cache.get(&oldest).is_none());
    assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
}

#[test]
fn rejects_files_without_a_thumbnail_provider() {
    assert_eq!(thumbnail_kind(Path::new("README.md")), None);
    assert_eq!(thumbnail_kind(Path::new("no-extension")), None);
}
