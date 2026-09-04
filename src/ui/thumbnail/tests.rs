// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gtk::glib;

use super::{
    ACTIVE_REQUESTS, ActiveRequest, CacheHit, CachedThumbnail, MAX_CACHE_ENTRIES,
    MAX_LOOKAHEAD_ITEMS, MAX_QUEUED_THUMBNAILS, MAX_THUMBNAIL_WORKERS, PENDING_THUMBNAILS,
    PendingTarget, PendingThumbnail, THUMBNAIL_QUEUE, ThumbnailCache, ThumbnailKey, ThumbnailKind,
    ThumbnailPriority, ThumbnailQueue, cancel_thumbnail, clear_lookahead, finish_thumbnail_targets,
    retry_deferred_thumbnail, schedule_or_defer, take_pending_targets, thumbnail_kind,
    update_lookahead,
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

    for _ in 0..MAX_THUMBNAIL_WORKERS {
        assert!(queue.begin_next().is_some());
    }
    assert!(queue.begin_next().is_none());
    queue.finish();
    assert!(queue.begin_next().is_some());
}

#[test]
fn saturated_queue_defers_the_live_request() {
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

fn test_entry(index: usize) -> crate::model::FileEntry {
    let name = format!("photo-{index}.png");
    crate::model::FileEntry {
        location: crate::model::Location::local(PathBuf::from(&name)),
        native_name: std::ffi::OsString::from(&name),
        display_name: name,
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Known(100),
        modified_unix_seconds: crate::model::MetadataValue::Known(1),
        is_hidden: false,
    }
}

#[test]
fn high_priority_thumbnails_run_before_low_priority_lookahead() {
    let mut queue = ThumbnailQueue::default();
    assert!(queue.enqueue_with_priority(key(1), ThumbnailPriority::Low));
    assert!(queue.enqueue_with_priority(key(2), ThumbnailPriority::Low));
    assert!(queue.enqueue_with_priority(key(99), ThumbnailPriority::High));

    let first = queue.begin_next();
    assert_eq!(first, Some(key(99)));
    queue.finish();

    let second = queue.begin_next();
    assert_eq!(second, Some(key(1)));
    queue.finish();

    let third = queue.begin_next();
    assert_eq!(third, Some(key(2)));
    queue.finish();

    assert!(queue.begin_next().is_none());
}

#[test]
fn lookahead_item_promoted_when_live_target_is_scheduled() {
    let mut queue = ThumbnailQueue::default();
    assert!(queue.enqueue_with_priority(key(5), ThumbnailPriority::Low));
    assert!(queue.low_priority.contains(&key(5)));
    assert!(!queue.queued.contains(&key(5)));

    queue.promote(&key(5));
    assert!(!queue.low_priority.contains(&key(5)));
    assert!(queue.queued.contains(&key(5)));
}

#[test]
fn lookahead_queue_bounds_speculative_items() {
    let mut queue = ThumbnailQueue::default();
    for index in 0..MAX_LOOKAHEAD_ITEMS {
        assert!(queue.enqueue_with_priority(key(index), ThumbnailPriority::Low));
    }
    assert!(!queue.enqueue_with_priority(key(MAX_LOOKAHEAD_ITEMS), ThumbnailPriority::Low));
}

#[test]
fn update_lookahead_clears_unstarted_speculative_items() {
    PENDING_THUMBNAILS.with(|pending| pending.borrow_mut().clear());
    THUMBNAIL_QUEUE.with(|queue| {
        queue.borrow_mut().queued.clear();
        queue.borrow_mut().low_priority.clear();
    });

    let first_batch = vec![test_entry(1), test_entry(2)];
    update_lookahead(&first_batch, 64);

    let key1 = ThumbnailKey {
        path: PathBuf::from("photo-1.png"),
        modified: Some(1),
        file_size: Some(100),
        thumbnail_size: 64,
    };
    let key2 = ThumbnailKey {
        path: PathBuf::from("photo-2.png"),
        modified: Some(1),
        file_size: Some(100),
        thumbnail_size: 64,
    };
    let key3 = ThumbnailKey {
        path: PathBuf::from("photo-3.png"),
        modified: Some(1),
        file_size: Some(100),
        thumbnail_size: 64,
    };

    PENDING_THUMBNAILS.with(|pending| {
        let pending = pending.borrow();
        assert!(pending.contains_key(&key1));
        assert!(pending.contains_key(&key2));
    });

    let second_batch = vec![test_entry(3)];
    update_lookahead(&second_batch, 64);

    PENDING_THUMBNAILS.with(|pending| {
        let pending = pending.borrow();
        assert!(!pending.contains_key(&key2));
        assert!(pending.contains_key(&key3));
    });

    clear_lookahead();
    PENDING_THUMBNAILS.with(|pending| {
        assert!(pending.borrow().is_empty());
    });
    THUMBNAIL_QUEUE.with(|queue| {
        assert!(queue.borrow().low_priority.is_empty());
    });
}

#[test]
fn cancel_thumbnail_does_not_cancel_unrelated_lookahead_jobs() {
    PENDING_THUMBNAILS.with(|pending| pending.borrow_mut().clear());
    THUMBNAIL_QUEUE.with(|queue| {
        queue.borrow_mut().queued.clear();
        queue.borrow_mut().low_priority.clear();
    });

    let lookahead_key = key(42);
    PENDING_THUMBNAILS.with(|pending| {
        pending.borrow_mut().insert(
            lookahead_key.clone(),
            PendingThumbnail {
                id: 100,
                kind: ThumbnailKind::Image,
                cancellation: crate::sandbox::Cancellation::default(),
                targets: Vec::new(),
            },
        );
    });
    THUMBNAIL_QUEUE.with(|queue| {
        assert!(
            queue
                .borrow_mut()
                .enqueue_with_priority(lookahead_key.clone(), ThumbnailPriority::Low)
        );
    });

    cancel_thumbnail(999);

    PENDING_THUMBNAILS.with(|pending| {
        assert!(pending.borrow().contains_key(&lookahead_key));
        pending.borrow_mut().clear();
    });
    THUMBNAIL_QUEUE.with(|queue| {
        assert!(queue.borrow().low_priority.contains(&lookahead_key));
        queue.borrow_mut().low_priority.clear();
    });
}
