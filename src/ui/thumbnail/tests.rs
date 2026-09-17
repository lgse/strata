// SPDX-License-Identifier: MIT

mod search;
mod trash;

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gtk::{gdk, glib};

use super::{
    ACTIVE_REQUESTS, ActiveRequest, CacheHit, CachedThumbnail, MAX_CACHE_ENTRIES,
    MAX_CACHE_READERS, MAX_PERSIST_QUEUE, MAX_QUEUED_THUMBNAILS, PENDING_THUMBNAILS, PendingTarget,
    PendingThumbnail, PersistJob, PersistQueue, SETTLE_VIEWS, THUMBNAIL_CACHE, THUMBNAIL_QUEUE,
    ThumbnailCache, ThumbnailKey, ThumbnailKind, ThumbnailQueue, ViewSettle, cancel_thumbnail,
    clear_thumbnail_runtime, finish_thumbnail_targets, fire_settled_thumbnails,
    has_pending_thumbnail, hold_thumbnail_workers, refresh_all_customized_icons,
    retry_deferred_thumbnail, schedule_or_defer, set_thumbnail_or_icon, show_customized_icon,
    take_pending_targets, thumbnail_kind,
};
use crate::{
    model::{EntryKind, FileEntry, FolderColor, FolderColorValue, Location, MetadataValue},
    test_support::gtk_test,
    ui::theme::ThemeManager,
};
use gtk::prelude::*;

pub(crate) fn complete_pending_thumbnail(path: &Path) {
    let (key, id) = PENDING_THUMBNAILS.with(|pending| {
        pending
            .borrow()
            .iter()
            .find(|(key, _)| key.path == path)
            .map(|(key, pending)| (key.clone(), pending.id))
            .expect("thumbnail admitted while indexing")
    });
    let targets = take_pending_targets(&key, id).expect("pending thumbnail targets");
    let images = targets
        .iter()
        .filter_map(|target| target.image.upgrade())
        .collect::<Vec<_>>();
    assert!(!images.is_empty());
    let pixels = glib::Bytes::from_owned(vec![255u8; 4]);
    let texture = gdk::MemoryTexture::new(1, 1, gdk::MemoryFormat::R8g8b8a8, &pixels, 4).upcast();
    THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().cancel(&key));
    finish_thumbnail_targets(targets, Some(&texture), path);
    assert!(
        images
            .iter()
            .all(|image| super::displayed_thumbnail_matches(image, path))
    );
}

fn key(index: usize) -> ThumbnailKey {
    ThumbnailKey {
        path: PathBuf::from(format!("image-{index}.png")),
        modified: Some(1),
        file_size: Some(1),
        thumbnail_size: 64,
    }
}

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
        thumbnail_kind(Path::new("vector.svg")),
        Some(ThumbnailKind::Image)
    );
    for name in ["photo.HEIC", "photo.heif", "photo.avif", "photo.jxl"] {
        assert_eq!(
            thumbnail_kind(Path::new(name)),
            Some(ThumbnailKind::Image),
            "{name}"
        );
    }
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

fn sample_texture() -> gdk::Texture {
    // 1×1 transparent PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    gdk::Texture::from_bytes(&glib::Bytes::from_static(PNG)).expect("1x1 PNG texture")
}

#[test]
fn thumbnail_cache_evicts_the_least_recent_entry() {
    let mut cache = ThumbnailCache::default();
    for index in 0..=MAX_CACHE_ENTRIES {
        cache.insert(key(index), sample_texture());
    }

    let oldest = key(0);
    assert!(cache.get(&oldest).is_none());
    assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
}

#[test]
fn thumbnail_cache_hits_reuse_the_decoded_texture() {
    let texture = sample_texture();
    let mut cache = ThumbnailCache::default();
    cache.insert(key(0), texture.clone());
    match cache.get(&key(0)) {
        Some(CacheHit::Ready(hit)) => assert_eq!(hit, texture),
        _ => panic!("expected a cached texture"),
    }
}

#[test]
fn thumbnail_queue_bounds_waiting_and_running_jobs() {
    let mut queue = ThumbnailQueue::default();
    for index in 0..MAX_QUEUED_THUMBNAILS {
        assert!(queue.enqueue(key(index)));
    }
    assert!(!queue.enqueue(key(MAX_QUEUED_THUMBNAILS)));

    for index in 0..MAX_CACHE_READERS {
        assert_eq!(queue.begin_next(), Some(key(index)));
    }
    assert!(queue.begin_next().is_none());
    queue.finish();
    assert_eq!(queue.begin_next(), Some(key(MAX_CACHE_READERS)));
}

#[test]
fn saturated_queue_defers_the_live_request() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let image_id = 99;
    let request = 7;
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(
            image_id,
            ActiveRequest {
                id: request,
                key: key(MAX_QUEUED_THUMBNAILS),
                image: glib::WeakRef::new(),
                deferred: None,
            },
        );
    });
    THUMBNAIL_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        for index in 0..MAX_QUEUED_THUMBNAILS {
            assert!(queue.enqueue(key(index)));
        }
    });

    let deferred_key = key(MAX_QUEUED_THUMBNAILS);
    schedule_or_defer(
        deferred_key.clone(),
        ThumbnailKind::Image,
        PendingTarget {
            image_id,
            request,
            image: glib::WeakRef::new(),
        },
    );
    fire_settled_thumbnails();
    SETTLE_VIEWS.with(|views| {
        let settle = &views.borrow()[&0];
        assert!(settle.timer.is_none());
        assert!(settle.pending.is_empty());
    });
    ACTIVE_REQUESTS.with(|requests| {
        let requests = requests.borrow();
        let deferred = requests[&image_id]
            .deferred
            .as_ref()
            .expect("request should be deferred");
        assert_eq!(deferred.key, deferred_key);
        assert_eq!(deferred.kind, ThumbnailKind::Image);
    });
    THUMBNAIL_QUEUE.with(|queue| {
        let _removed = queue.borrow_mut().queued.pop_front();
    });
    let (image, deferred) = ACTIVE_REQUESTS.with(|requests| {
        let requests = requests.borrow();
        let active = &requests[&image_id];
        (
            active.image.clone(),
            active.deferred.clone().expect("request should be deferred"),
        )
    });
    assert!(retry_deferred_thumbnail(image_id, request, image, deferred));
    ACTIVE_REQUESTS.with(|requests| {
        assert!(requests.borrow()[&image_id].deferred.is_none());
    });
    PENDING_THUMBNAILS.with(|pending| {
        assert!(pending.borrow().contains_key(&deferred_key));
        pending.borrow_mut().clear();
    });
    THUMBNAIL_QUEUE.with(|queue| {
        assert_eq!(queue.borrow().queued.len(), MAX_QUEUED_THUMBNAILS);
        queue.borrow_mut().queued.clear();
    });
    ACTIVE_REQUESTS.with(|requests| requests.borrow_mut().clear());
}

#[test]
fn failed_jobs_release_their_active_requests() {
    let image_id = 99;
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(
            image_id,
            ActiveRequest {
                id: 7,
                key: key(0),
                image: glib::WeakRef::new(),
                deferred: None,
            },
        );
    });

    finish_thumbnail_targets(
        vec![PendingTarget {
            image_id,
            request: 7,
            image: glib::WeakRef::new(),
        }],
        None,
        Path::new("image.png"),
    );

    ACTIVE_REQUESTS.with(|requests| assert!(requests.borrow().is_empty()));
}

#[test]
fn cancelling_the_last_target_cancels_shared_work() {
    let key = key(0);
    let cancellation = crate::sandbox::Cancellation::default();
    PENDING_THUMBNAILS.with(|pending| {
        pending.borrow_mut().insert(
            key.clone(),
            PendingThumbnail {
                id: 1,
                kind: ThumbnailKind::Image,
                cancellation: cancellation.clone(),
                targets: vec![
                    PendingTarget {
                        image_id: 1,
                        request: 1,
                        image: glib::WeakRef::new(),
                    },
                    PendingTarget {
                        image_id: 2,
                        request: 2,
                        image: glib::WeakRef::new(),
                    },
                ],
            },
        );
    });
    THUMBNAIL_QUEUE.with(|queue| assert!(queue.borrow_mut().enqueue(key.clone())));

    cancel_thumbnail(1);
    assert!(!cancellation.is_cancelled());
    PENDING_THUMBNAILS.with(|pending| {
        assert_eq!(pending.borrow()[&key].targets.len(), 1);
    });

    cancel_thumbnail(2);
    assert!(cancellation.is_cancelled());
    PENDING_THUMBNAILS.with(|pending| assert!(!pending.borrow().contains_key(&key)));
    THUMBNAIL_QUEUE.with(|queue| assert!(queue.borrow().queued.is_empty()));
}

#[test]
fn stale_completion_cannot_remove_a_requeued_job() {
    let key = key(0);
    PENDING_THUMBNAILS.with(|pending| {
        pending.borrow_mut().insert(
            key.clone(),
            PendingThumbnail {
                id: 2,
                kind: ThumbnailKind::Image,
                cancellation: crate::sandbox::Cancellation::default(),
                targets: Vec::new(),
            },
        );
    });

    assert!(take_pending_targets(&key, 1).is_none());
    PENDING_THUMBNAILS.with(|pending| assert!(pending.borrow().contains_key(&key)));
    assert!(take_pending_targets(&key, 2).is_some());
}

#[test]
fn failed_thumbnails_expire_and_share_the_cache_bound() {
    let mut cache = ThumbnailCache::default();
    for index in 0..=MAX_CACHE_ENTRIES {
        cache.insert_failure(key(index));
    }
    assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
    assert!(matches!(cache.get(&key(1)), Some(CacheHit::Failed)));

    let expired = key(MAX_CACHE_ENTRIES + 1);
    cache.insert_entry(
        expired.clone(),
        CachedThumbnail::Failed(Instant::now() - Duration::from_secs(1)),
    );
    assert!(cache.get(&expired).is_none());
}

#[test]
fn rejects_files_without_a_thumbnail_provider() {
    assert_eq!(thumbnail_kind(Path::new("README.md")), None);
    assert_eq!(thumbnail_kind(Path::new("no-extension")), None);
}

#[test]
fn cancelling_drops_hooked_settle_groups_with_a_dead_viewport() {
    SETTLE_VIEWS.with(|views| {
        views.borrow_mut().insert(
            42,
            ViewSettle {
                viewport: glib::WeakRef::new(),
                pending: Vec::new(),
                timer: None,
                hooked: true,
            },
        );
    });

    cancel_thumbnail(1);

    SETTLE_VIEWS.with(|views| {
        assert!(
            !views.borrow().contains_key(&42),
            "a hooked settle group whose viewport is gone should drop"
        );
    });
}

#[test]
fn persist_queue_bounds_and_drains_oldest_first() {
    let mut queue = PersistQueue::new();
    for index in 0..MAX_PERSIST_QUEUE + 5 {
        queue.push(PersistJob {
            path: PathBuf::from(index.to_string()),
            mtime: 1,
            png: vec![1],
        });
    }
    assert_eq!(queue.len(), MAX_PERSIST_QUEUE);
    assert_eq!(
        queue.pop_front().expect("queue should drain").path,
        PathBuf::from("5")
    );
    let mut drained = 1;
    while queue.pop_front().is_some() {
        drained += 1;
    }
    assert_eq!(drained, MAX_PERSIST_QUEUE);
}

fn sample_entry(path: &Path) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: path
            .file_name()
            .map_or_else(Default::default, |name| name.to_os_string()),
        display_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        kind: EntryKind::File,
        size: MetadataValue::Known(1),
        modified_unix_seconds: MetadataValue::Known(1),
        mode: MetadataValue::Known(0o100644),
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    }
}

fn drain_main_loop() {
    let context = glib::MainContext::default();
    for _ in 0..64 {
        if !context.iteration(false) {
            break;
        }
    }
}

fn displayed_texture(image: &super::ThumbnailSlot) -> Option<gdk::Texture> {
    image.texture()
}

fn texture_pixels(texture: &gdk::Texture) -> Vec<u8> {
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    downloader.download_bytes().0.to_vec()
}

fn fallback_pixels(slot: &super::ThumbnailSlot) -> Vec<u8> {
    texture_pixels(
        &slot
            .fallback_texture()
            .expect("customized icon should rasterize"),
    )
}

fn bind_thumbnail(image: &super::ThumbnailSlot, entry: &FileEntry) {
    set_thumbnail_or_icon(image, entry, crate::assets::icons::PICTURES, 64, 64);
}

#[test]
fn cache_hit_applies_texture_on_idle_not_during_bind() {
    gtk_test(
        "ui::thumbnail::tests::cache_hit_applies_texture_on_idle_not_during_bind",
        || {
            super::super::theme::ThemeManager::shared();
            let path = PathBuf::from("/fixture/cache-hit.png");
            let pending = super::ThumbnailSlot::new(64);
            bind_thumbnail(&pending, &sample_entry(&path));
            let texture = sample_texture();
            THUMBNAIL_CACHE.with(|cache| {
                cache.borrow_mut().insert(
                    ThumbnailKey {
                        path: path.clone(),
                        modified: Some(1),
                        file_size: Some(1),
                        thumbnail_size: 256,
                    },
                    texture.clone(),
                );
            });
            bind_thumbnail(&pending, &sample_entry(&path));
            assert_eq!(displayed_texture(&pending).as_ref(), Some(&texture));
            for size in [17, 18, 96] {
                let image = super::ThumbnailSlot::new(size);
                super::set_thumbnail_or_icon(
                    &image,
                    &sample_entry(&path),
                    crate::assets::icons::PICTURES,
                    size,
                    size,
                );
                assert_eq!(displayed_texture(&image).as_ref(), Some(&texture));
                assert!(!has_pending_thumbnail(&path));
            }
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn theme_refresh_does_not_reenter_tracked_icon_refcell() {
    gtk_test(
        "ui::thumbnail::tests::theme_refresh_does_not_reenter_tracked_icon_refcell",
        || {
            super::super::theme::ThemeManager::shared();
            let list = gtk::ListBox::new();
            let scroll = gtk::ScrolledWindow::builder()
                .child(&list)
                .min_content_height(80)
                .build();
            let window = gtk::Window::builder().child(&scroll).build();
            window.present();
            for name in ["a.txt", "b.txt"] {
                let slot = super::ThumbnailSlot::new(19);
                show_customized_icon(&slot, Path::new(name), crate::assets::icons::DOCUMENTS, 19);
                let row = gtk::ListBoxRow::new();
                row.set_child(Some(&slot));
                list.append(&row);
            }
            drain_main_loop();
            refresh_all_customized_icons();
            drain_main_loop();
            clear_thumbnail_runtime();
        },
    );
}

/// Icon resolution reads folder color and custom icon from the manager, and
/// already-shown slots refresh when those preferences change.
#[test]
fn path_customization_refreshes_rendered_icons() {
    gtk_test(
        "ui::thumbnail::tests::path_customization_refreshes_rendered_icons",
        || {
            let manager = ThemeManager::shared();
            let customized = Path::new("/fixture/custom-folder");
            let other = Path::new("/fixture/plain-folder");
            let color = FolderColorValue::Preset(FolderColor::Red);
            let default = crate::assets::primary_icon_paintable(crate::assets::icons::FOLDER)
                .expect("default folder icon");
            let colored = crate::assets::custom_colored_icon_paintable(
                crate::assets::icons::FOLDER,
                color.hex(),
            )
            .expect("colored folder icon");
            let decorated =
                crate::assets::folder_decoration_paintable(crate::assets::icons::HOME, color.hex())
                    .expect("decorated folder icon");
            let default_pixels = texture_pixels(&default);
            let colored_pixels = texture_pixels(&colored);
            let decorated_pixels = texture_pixels(&decorated);
            assert_ne!(default_pixels, colored_pixels);
            assert_ne!(colored_pixels, decorated_pixels);

            let customized_slot = super::ThumbnailSlot::new(32);
            let other_slot = super::ThumbnailSlot::new(32);
            show_customized_icon(
                &customized_slot,
                customized,
                crate::assets::icons::FOLDER,
                32,
            );
            show_customized_icon(&other_slot, other, crate::assets::icons::FOLDER, 32);
            assert_eq!(fallback_pixels(&customized_slot), default_pixels);
            assert_eq!(fallback_pixels(&other_slot), default_pixels);

            manager.set_folder_color(customized, Some(color));
            assert_eq!(fallback_pixels(&customized_slot), colored_pixels);
            assert_eq!(fallback_pixels(&other_slot), default_pixels);

            manager.set_custom_icon(customized, Some(crate::assets::icons::HOME));
            assert_eq!(fallback_pixels(&customized_slot), decorated_pixels);
            assert_eq!(fallback_pixels(&other_slot), default_pixels);

            manager.clear_item_customization(customized);
            assert_eq!(fallback_pixels(&customized_slot), default_pixels);
            assert_eq!(fallback_pixels(&other_slot), default_pixels);
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn texture_swap_does_not_queue_resize() {
    gtk_test(
        "ui::thumbnail::tests::texture_swap_does_not_queue_resize",
        || {
            let image = super::ThumbnailSlot::new(64);
            let before = image.measure(gtk::Orientation::Horizontal, -1);
            let resizes = image.resize_calls();
            image.set_texture(&sample_texture());
            image.set_fallback(crate::assets::icons::PICTURES, Some(&sample_texture()));
            assert_eq!(image.resize_calls(), resizes);
            assert_eq!(image.measure(gtk::Orientation::Horizontal, -1), before);
        },
    );
}

#[test]
fn cache_miss_enqueues_sandbox_job_without_settle_timeout() {
    gtk_test(
        "ui::thumbnail::tests::cache_miss_enqueues_sandbox_job_without_settle_timeout",
        || {
            super::super::theme::ThemeManager::shared();
            hold_thumbnail_workers();
            let path = PathBuf::from("/fixture/cache-miss.png");
            let image = super::ThumbnailSlot::new(64);
            bind_thumbnail(&image, &sample_entry(&path));
            drain_main_loop();
            assert!(has_pending_thumbnail(&path));
            SETTLE_VIEWS.with(|views| {
                let views = views.borrow();
                if let Some(settle) = views.get(&0) {
                    assert!(settle.timer.is_none());
                    assert!(settle.pending.is_empty());
                }
            });
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn stale_request_id_does_not_apply_completed_texture() {
    gtk_test(
        "ui::thumbnail::tests::stale_request_id_does_not_apply_completed_texture",
        || {
            super::super::theme::ThemeManager::shared();
            let path = PathBuf::from("/fixture/stale.png");
            let image = super::ThumbnailSlot::new(64);
            let image_id = image.as_ptr() as usize;
            let weak = glib::WeakRef::new();
            weak.set(Some(&image));
            ACTIVE_REQUESTS.with(|requests| {
                requests.borrow_mut().insert(
                    image_id,
                    ActiveRequest {
                        id: 2,
                        key: key(0),
                        image: weak.clone(),
                        deferred: None,
                    },
                );
            });
            let texture = sample_texture();
            finish_thumbnail_targets(
                vec![PendingTarget {
                    image_id,
                    request: 1,
                    image: weak,
                }],
                Some(&texture),
                &path,
            );
            drain_main_loop();
            assert_ne!(displayed_texture(&image).as_ref(), Some(&texture));
            ACTIVE_REQUESTS.with(|requests| {
                assert_eq!(
                    requests.borrow().get(&image_id).map(|active| active.id),
                    Some(2)
                );
            });
            clear_thumbnail_runtime();
        },
    );
}
