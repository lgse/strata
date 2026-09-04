// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gtk::glib;

use super::{
    ACTIVE_REQUESTS, ActiveRequest, CacheHit, CachedThumbnail, MAX_CACHE_ENTRIES,
    MAX_PERSIST_QUEUE, MAX_QUEUED_THUMBNAILS, MAX_THUMBNAIL_WORKERS, METADATA_WAITERS,
    MetadataWaiter, PENDING_THUMBNAILS, PendingTarget, PendingThumbnail, PersistJob, PersistQueue,
    SETTLE_VIEWS, SettledPark, THUMBNAIL_QUEUE, ThumbnailCache, ThumbnailKey, ThumbnailKind,
    ThumbnailQueue, ViewSettle, cancel_thumbnail, finish_thumbnail_targets,
    fire_settled_thumbnails, note_metadata, retry_deferred_thumbnail, schedule_or_defer,
    take_pending_targets, thumbnail_kind,
};

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
        cache.insert(key(index), glib::Bytes::from_static(&[1]));
    }

    let oldest = key(0);
    assert!(cache.get(&oldest).is_none());
    assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
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
        64,
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
