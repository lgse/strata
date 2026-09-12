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
    MAX_PERSIST_QUEUE, MAX_QUEUED_THUMBNAILS, MAX_THUMBNAIL_WORKERS, METADATA_WAITERS,
    MetadataWaiter, PENDING_THUMBNAILS, PendingTarget, PendingThumbnail, PersistJob, PersistQueue,
    SETTLE_VIEWS, SettledPark, THUMBNAIL_CACHE, THUMBNAIL_QUEUE, ThumbnailCache, ThumbnailKey,
    ThumbnailKind, ThumbnailQueue, ViewSettle, cancel_thumbnail, clear_thumbnail_runtime,
    finish_thumbnail_targets, fire_settled_thumbnails, has_pending_thumbnail,
    hold_thumbnail_workers, note_metadata, refresh_all_customized_icons, retry_deferred_thumbnail,
    schedule_or_defer, set_thumbnail_or_icon, show_customized_icon, take_pending_targets,
    thumbnail_kind,
};
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    test_support::gtk_test,
};
use gtk::prelude::*;

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

    for index in 0..MAX_THUMBNAIL_WORKERS {
        assert_eq!(queue.begin_next(), Some(key(index)));
    }
    assert!(queue.begin_next().is_none());
    queue.finish();
    assert_eq!(queue.begin_next(), Some(key(MAX_THUMBNAIL_WORKERS)));
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
fn viewport_eligibility_covers_visible_plus_overscan() {
    use super::rect_eligible;
    assert!(rect_eligible(10.0, 10.0, 100.0, 40.0, 1000.0, 760.0));
    assert!(rect_eligible(-20.0, 100.0, 100.0, 40.0, 1000.0, 760.0));
    assert!(rect_eligible(950.0, 100.0, 100.0, 40.0, 1000.0, 760.0));
    assert!(rect_eligible(100.0, -20.0, 100.0, 40.0, 1000.0, 760.0));
    assert!(rect_eligible(100.0, 750.0, 100.0, 40.0, 1000.0, 760.0));
    assert!(rect_eligible(100.0, -190.0, 100.0, 40.0, 1000.0, 760.0));
    assert!(rect_eligible(
        100.0,
        760.0 + 100.0,
        100.0,
        40.0,
        1000.0,
        760.0
    ));
    assert!(!rect_eligible(100.0, -300.0, 100.0, 40.0, 1000.0, 760.0));
    assert!(!rect_eligible(
        100.0,
        760.0 + 500.0,
        100.0,
        40.0,
        1000.0,
        760.0
    ));
    assert!(!rect_eligible(2000.0, 100.0, 100.0, 40.0, 1000.0, 760.0));
    assert!(!rect_eligible(-500.0, 100.0, 100.0, 40.0, 1000.0, 760.0));
    assert!(!rect_eligible(0.0, 4.0, 0.0, 0.0, 1000.0, 760.0));
    assert!(!rect_eligible(0.0, 4.0, -1.0, 40.0, 1000.0, 760.0));
    assert!(!rect_eligible(0.0, 0.0, 100.0, 40.0, 0.0, 0.0));
}

#[test]
fn metadata_fill_updates_thumbnail_waiting_for_settle() {
    let path = PathBuf::from("pending.png");
    SETTLE_VIEWS.with(|views| {
        views.borrow_mut().insert(
            0,
            ViewSettle {
                viewport: glib::WeakRef::new(),
                pending: vec![SettledPark {
                    key: ThumbnailKey {
                        path: path.clone(),
                        modified: None,
                        file_size: None,
                        thumbnail_size: 64,
                    },
                    kind: ThumbnailKind::Image,
                    target: PendingTarget {
                        image_id: 1,
                        request: 1,
                        image: glib::WeakRef::new(),
                    },
                    wait_for_metadata: true,
                }],
                timer: None,
                first_park: None,
                hooked: false,
            },
        );
    });

    note_metadata(&path, Some(42), Some(99));

    SETTLE_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let park = &views[&0].pending[0];
        assert_eq!(park.key.modified, Some(42));
        assert_eq!(park.key.file_size, Some(99));
        assert!(!park.wait_for_metadata);
        views.clear();
    });
}

#[test]
fn unavailable_metadata_releases_settled_thumbnail_work() {
    let path = PathBuf::from("unavailable.png");
    SETTLE_VIEWS.with(|views| {
        views.borrow_mut().insert(
            0,
            ViewSettle {
                viewport: glib::WeakRef::new(),
                pending: vec![SettledPark {
                    key: ThumbnailKey {
                        path: path.clone(),
                        modified: None,
                        file_size: None,
                        thumbnail_size: 64,
                    },
                    kind: ThumbnailKind::Image,
                    target: PendingTarget {
                        image_id: 1,
                        request: 1,
                        image: glib::WeakRef::new(),
                    },
                    wait_for_metadata: true,
                }],
                timer: None,
                first_park: None,
                hooked: false,
            },
        );
    });

    note_metadata(&path, None, None);

    SETTLE_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let park = &views[&0].pending[0];
        assert_eq!(park.key.modified, None);
        assert!(!park.wait_for_metadata);
        views.clear();
    });
}

#[test]
fn cancellation_removes_metadata_waiters() {
    let path = PathBuf::from("cancelled.png");
    METADATA_WAITERS.with(|waiters| {
        waiters.borrow_mut().insert(
            path.clone(),
            vec![MetadataWaiter {
                group: 0,
                kind: ThumbnailKind::Image,
                target: PendingTarget {
                    image_id: 7,
                    request: 1,
                    image: glib::WeakRef::new(),
                },
                file_size: None,
                thumbnail_size: 64,
            }],
        );
    });

    cancel_thumbnail(7);

    METADATA_WAITERS.with(|waiters| assert!(!waiters.borrow().contains_key(&path)));
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
                first_park: None,
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
        is_hidden: false,
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
            let texture = sample_texture();
            THUMBNAIL_CACHE.with(|cache| {
                cache.borrow_mut().insert(
                    ThumbnailKey {
                        path: path.clone(),
                        modified: Some(1),
                        file_size: Some(1),
                        thumbnail_size: 64,
                    },
                    texture.clone(),
                );
            });
            let image = super::ThumbnailSlot::new(64);
            bind_thumbnail(&image, &sample_entry(&path));
            assert_eq!(displayed_texture(&image).as_ref(), Some(&texture));
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
