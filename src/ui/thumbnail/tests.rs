// SPDX-License-Identifier: MIT

use super::*;
use crate::{test_support::gtk_test, ui::preferences::PreferenceManager};

fn texture() -> gdk::Texture {
    gdk::MemoryTexture::new(
        1,
        1,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(vec![120, 80, 40, 255]),
        4,
    )
    .upcast()
}

#[test]
fn stl_opt_in_gates_cached_artwork_and_cancels_late_results() {
    gtk_test(
        "ui::thumbnail::tests::stl_opt_in_gates_cached_artwork_and_cancels_late_results",
        || {
            let manager = PreferenceManager::shared();
            assert!(!manager.stl_thumbnails());
            let directory = tempfile::tempdir().expect("model folder");
            let cached = directory.path().join("cached.STL");
            THUMBNAIL_CACHE.with_borrow_mut(|cache| {
                cache.insert(
                    ThumbnailKey {
                        path: cached.clone(),
                        modified: None,
                        file_size: None,
                        thumbnail_size: super::super::thumbnail_cache::CANONICAL_MAX_EDGE,
                    },
                    texture(),
                );
            });
            let slot = ThumbnailSlot::new(18);
            set_thumbnail_or_icon_for_path(&slot, &cached, crate::assets::icons::DOCUMENTS, 18, 32);
            assert!(slot.texture().is_none());
            manager.set_stl_thumbnails(true);
            assert!(slot.texture().is_some());
            manager.set_show_thumbnails(false);
            assert!(slot.texture().is_none());
            manager.set_show_thumbnails(true);
            assert!(slot.texture().is_some());

            let uncached = directory.path().join("uncached.stl");
            set_thumbnail_or_icon_for_path(
                &slot,
                &uncached,
                crate::assets::icons::DOCUMENTS,
                18,
                32,
            );
            let target = ACTIVE_REQUESTS.with_borrow(|requests| {
                let active = requests
                    .get(&(slot.as_ptr() as usize))
                    .expect("pending model");
                PendingTarget {
                    image_id: slot.as_ptr() as usize,
                    request: active.id,
                    image: active.image.clone(),
                }
            });
            manager.set_stl_thumbnails(false);
            assert!(!request_is_live(&target));
            finish_thumbnail_targets(vec![target], Some(&texture()), &uncached);
            assert!(slot.texture().is_none());
        },
    );
}

#[test]
fn a_running_model_does_not_block_other_thumbnail_jobs() {
    gtk_test(
        "ui::thumbnail::tests::a_running_model_does_not_block_other_thumbnail_jobs",
        || {
            let directory = tempfile::tempdir().expect("thumbnail folder");
            for (id, kind, name) in [
                (1, ThumbnailKind::RenderedModel, "second.stl"),
                (2, ThumbnailKind::Image, "photo.png"),
            ] {
                let key = ThumbnailKey {
                    path: directory.path().join(name),
                    modified: None,
                    file_size: None,
                    thumbnail_size: 256,
                };
                RENDER_QUEUE.with_borrow_mut(|queue| {
                    queue.push_back(ThumbnailJob {
                        id,
                        resolved: key.clone(),
                        key,
                        kind,
                        cancellation: Cancellation::default(),
                    })
                });
            }
            MODEL_RUNNING.with(|running| running.set(true));
            assert_eq!(
                next_render_job(4).expect("image can run").kind,
                ThumbnailKind::Image
            );
            assert!(next_render_job(4).is_none(), "second model waits");
            MODEL_RUNNING.with(|running| running.set(false));
            assert_eq!(
                next_render_job(4).expect("model resumes").kind,
                ThumbnailKind::RenderedModel
            );
            assert!(next_render_job(4).is_none());
        },
    );
}

#[test]
fn remote_stl_does_not_request_a_camera_or_mirror_thumbnail() {
    gtk_test(
        "ui::thumbnail::tests::remote_stl_does_not_request_a_camera_or_mirror_thumbnail",
        || {
            PreferenceManager::shared().set_stl_thumbnails(true);
            let directory = tempfile::tempdir().expect("model folder");
            let path = directory.path().join("model.stl");
            std::fs::write(&path, b"fixture").expect("model fixture");
            let mut entry = glib::MainContext::default()
                .block_on(crate::adapters::query_file_entry(
                    crate::model::Location::local(&path),
                ))
                .expect("model entry");
            for uri in [
                "gphoto2://camera/model.stl".to_owned(),
                gio::File::for_path(&path).uri().to_string(),
            ] {
                entry.location = crate::model::Location::uri(uri);
                let slot = ThumbnailSlot::new(18);
                set_thumbnail_or_icon(&slot, &entry, crate::assets::icons::DOCUMENTS, 18, 32);
                assert!(slot.thumbnail_source().is_none());
                assert!(slot.texture().is_none());
            }
        },
    );
}

#[test]
fn show_thumbnails_saved_setting_bypasses_cached_and_uncached_media() {
    gtk_test(
        "ui::thumbnail::tests::show_thumbnails_saved_setting_bypasses_cached_and_uncached_media",
        || {
            let config = crate::ui::preferences::config_directory();
            std::fs::create_dir_all(&config).expect("config directory");
            std::fs::write(config.join("settings.toml"), "show_thumbnails = false\n")
                .expect("saved thumbnail setting");
            let manager = PreferenceManager::shared();
            let directory = tempfile::tempdir().expect("media folder");
            let cached = directory.path().join("cached.png");
            let pending = directory.path().join("uncached.pdf");
            let preview = texture();
            THUMBNAIL_CACHE.with_borrow_mut(|cache| {
                cache.insert(
                    ThumbnailKey {
                        path: cached.clone(),
                        modified: None,
                        file_size: None,
                        thumbnail_size: super::super::thumbnail_cache::CANONICAL_MAX_EDGE,
                    },
                    preview,
                );
            });
            for (path, icon) in [
                (&cached, crate::assets::icons::PICTURES),
                (&pending, crate::assets::icons::DOCUMENTS),
            ] {
                let slot = ThumbnailSlot::new(18);
                set_thumbnail_or_icon_for_path(&slot, path, icon, 18, 32);
                assert!(slot.texture().is_none());
                assert_eq!(slot.fallback_icon().as_deref(), Some(icon));
                ACTIVE_REQUESTS.with_borrow(|requests| {
                    assert!(!requests.contains_key(&(slot.as_ptr() as usize)));
                });
            }
            assert!(!manager.show_thumbnails());
        },
    );
}

#[test]
fn show_thumbnails_cancels_late_results_and_forgets_recycled_targets() {
    gtk_test(
        "ui::thumbnail::tests::show_thumbnails_cancels_late_results_and_forgets_recycled_targets",
        || {
            let manager = PreferenceManager::shared();
            let directory = tempfile::tempdir().expect("media folder");
            let old_path = directory.path().join("old.png");
            let slot = ThumbnailSlot::new(18);
            set_thumbnail_or_icon_for_path(
                &slot,
                &old_path,
                crate::assets::icons::PICTURES,
                18,
                32,
            );
            let target = ACTIVE_REQUESTS.with_borrow(|requests| {
                let image_id = slot.as_ptr() as usize;
                let active = requests.get(&image_id).expect("pending thumbnail");
                PendingTarget {
                    image_id,
                    request: active.id,
                    image: active.image.clone(),
                }
            });
            manager.set_show_thumbnails(false);
            assert!(!request_is_live(&target));
            finish_thumbnail_targets(vec![target.clone()], Some(&texture()), &old_path);
            assert!(slot.texture().is_none());

            let new_path = directory.path().join("new.png");
            let new_texture = texture();
            THUMBNAIL_CACHE.with_borrow_mut(|cache| {
                cache.insert(
                    ThumbnailKey {
                        path: new_path.clone(),
                        modified: None,
                        file_size: None,
                        thumbnail_size: super::super::thumbnail_cache::CANONICAL_MAX_EDGE,
                    },
                    new_texture.clone(),
                );
            });
            set_thumbnail_or_icon_for_path(
                &slot,
                &new_path,
                crate::assets::icons::PICTURES,
                18,
                32,
            );
            manager.set_show_thumbnails(true);
            assert_eq!(slot.texture(), Some(new_texture.clone()));
            finish_thumbnail_targets(vec![target], Some(&texture()), &old_path);
            assert_eq!(slot.texture(), Some(new_texture));

            manager.set_show_thumbnails(false);
            cancel_thumbnails_in(slot.upcast_ref());
            manager.set_show_thumbnails(true);
            assert!(slot.texture().is_none());
            ACTIVE_REQUESTS.with_borrow(|requests| assert!(requests.is_empty()));
        },
    );
}

#[test]
fn a_thumbnail_released_by_an_operation_is_not_remembered_as_failed() {
    gtk_test(
        "ui::thumbnail::tests::a_thumbnail_released_by_an_operation_is_not_remembered_as_failed",
        || {
            let directory = tempfile::tempdir().expect("media folder");
            let links = tempfile::tempdir().expect("link folder");
            let link = links.path().join("media");
            std::os::unix::fs::symlink(directory.path(), &link).expect("linked folder");
            for (row, shown_in, name, refused_after_end) in [
                (
                    "refused while it runs",
                    directory.path(),
                    "running.png",
                    false,
                ),
                (
                    "refused through a linked folder while it runs",
                    link.as_path(),
                    "linked.png",
                    false,
                ),
                (
                    "refused after it ended",
                    directory.path(),
                    "ended.png",
                    true,
                ),
            ] {
                let key = ThumbnailKey {
                    path: shown_in.join(name),
                    modified: None,
                    file_size: None,
                    thumbnail_size: super::super::thumbnail_cache::CANONICAL_MAX_EDGE,
                };
                let slot = ThumbnailSlot::new(18);
                let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
                let target =
                    register_active_request(&slot, slot.as_ptr() as usize, request, key.clone());
                let job = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
                PENDING_THUMBNAILS.with_borrow_mut(|pending| {
                    pending.insert(
                        key.clone(),
                        PendingThumbnail {
                            id: job,
                            kind: ThumbnailKind::Image,
                            cancellation: Cancellation::default(),
                            targets: vec![target.clone()],
                        },
                    );
                });
                let operation = crate::adapters::ActiveOperation::begin([]);
                glib::MainContext::default().block_on(
                    operation.release(vec![directory.path().join(name)], &gio::Cancellable::new()),
                );
                let operation = if refused_after_end {
                    drop(operation);
                    None
                } else {
                    Some(operation)
                };

                glib::MainContext::default().block_on(finish_thumbnail_job(
                    ThumbnailJob {
                        id: job,
                        key: key.clone(),
                        resolved: key.clone(),
                        kind: ThumbnailKind::Image,
                        cancellation: Cancellation::default(),
                    },
                    Err(crate::services::file_activity::HELD_FOR_OPERATION.to_owned()),
                ));

                assert!(
                    THUMBNAIL_CACHE
                        .with_borrow_mut(|cache| cache.get(&key))
                        .is_none(),
                    "{row}: an operation's refusal is no failure to remember"
                );
                assert!(request_is_live(&target), "{row}");
                assert_eq!(
                    PENDING_THUMBNAILS.with_borrow(|pending| pending.contains_key(&key)),
                    refused_after_end,
                    "{row}: asked for again at once only when no operation will end later"
                );
                drop(operation);
                assert!(
                    PENDING_THUMBNAILS.with_borrow(|pending| pending.contains_key(&key)),
                    "{row}: the thumbnail is asked for again once the operation ends"
                );
            }
        },
    );
}
