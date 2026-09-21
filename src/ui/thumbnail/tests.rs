// SPDX-License-Identifier: MIT

mod search;
mod trash;

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gtk::{gdk, glib};

use super::{
    ACTIVE_REQUESTS, ARCHIVE_ART, AUDIO_ART, AUDIO_PROJECT_ART, ActiveRequest, CERT_ART,
    COMICS_ART, CONFIG_ART, CacheHit, CachedThumbnail, DATABASE_ART, DESIGN_ART, DOCX_ART,
    EBOOKS_ART, FONT_ART, IMAGE_ART, ISO_ART, LOG_ART, MAPS_ART, MAX_CACHE_ENTRIES,
    MAX_CACHE_READERS, MAX_PERSIST_QUEUE, MAX_QUEUED_THUMBNAILS, MODELS_3D_ART, MUSIC_ART,
    PACKAGE_ART, PENDING_THUMBNAILS, PLAYLISTS_ART, PPTX_ART, PendingTarget, PendingThumbnail,
    PersistJob, PersistQueue, SCIENCE_ART, SETTLE_VIEWS, SPREADSHEET_ART, SQL_ART, SUBTITLES_ART,
    TEXT_ART, THUMBNAIL_CACHE, THUMBNAIL_QUEUE, ThumbnailCache, ThumbnailKey, ThumbnailKind,
    ThumbnailQueue, VIDEO_ART, VIRTUAL_DISK_ART, VM_ART, ViewSettle, WEB_ART, cancel_thumbnail,
    clear_thumbnail_runtime, fallback_art_source, finish_thumbnail_targets,
    fire_settled_thumbnails, has_pending_thumbnail, hold_thumbnail_workers,
    refresh_all_customized_icons, retry_deferred_thumbnail, schedule_or_defer,
    set_thumbnail_or_icon, show_customized_icon, take_pending_targets, thumbnail_kind,
};
use crate::{
    model::{EntryKind, FileEntry, FolderColor, FolderColorValue, Location, MetadataValue},
    test_support::gtk_test,
    ui::{preferences::PreferenceManager, theme::ThemeTokens},
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
fn recognizes_mainstream_image_video_and_cover_art_audio_formats() {
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
    for name in [
        "song.MP3",
        "song.flac",
        "song.m4a",
        "song.m4b",
        "song.mka",
        "song.aiff",
        "song.aif",
        "song.wma",
    ] {
        assert_eq!(
            thumbnail_kind(Path::new(name)),
            Some(ThumbnailKind::Video),
            "{name}"
        );
    }
    for name in ["song.ogg", "song.oga", "song.opus", "song.wav"] {
        assert_eq!(thumbnail_kind(Path::new(name)), None, "{name}");
    }
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
fn recognizes_container_audio_and_text_formats() {
    for name in [
        "novel.epub",
        "issue1.cbz",
        "issue2.cbr",
        "novel.fb2",
        "kindle.mobi",
        "kindle.azw3",
        "scan.djvu",
        "mockup.sketch",
        "painting.kra",
        "app.ipa",
        "app.apk",
        "tablet.mobi",
        "ebook.prc",
    ] {
        assert_eq!(
            thumbnail_kind(Path::new(name)),
            Some(ThumbnailKind::Embedded),
            "{name}"
        );
    }
    for name in [
        "track.mp3",
        "album.flac",
        "song.m4a",
        "song.aac",
        "sample.wav",
        "tape.aiff",
        "clip.ogg",
        "voice.opus",
        "tune.wma",
    ] {
        assert_eq!(
            thumbnail_kind(Path::new(name)),
            Some(ThumbnailKind::AudioArt),
            "{name}"
        );
    }
    for name in [
        "README.md",
        "notes.txt",
        "data.json",
        "letter.rtf",
        "movie.srt",
        "mix.m3u8",
        "trail.gpx",
        "route.kml",
        "model.obj",
        "print.stl",
        "scene.gltf",
        "plan.dxf",
        "query.sql",
        "page.xhtml",
    ] {
        assert_eq!(
            thumbnail_kind(Path::new(name)),
            Some(ThumbnailKind::Text),
            "{name}"
        );
    }
    for (name, language) in [
        ("main.rs", crate::sandbox::CodeLanguage::Rust),
        ("script.py", crate::sandbox::CodeLanguage::Python),
        ("app.ts", crate::sandbox::CodeLanguage::TypeScript),
        ("style.css", crate::sandbox::CodeLanguage::Css),
        ("script.sh", crate::sandbox::CodeLanguage::Shell),
        ("page.html", crate::sandbox::CodeLanguage::Html),
    ] {
        assert_eq!(
            thumbnail_kind(Path::new(name)),
            Some(ThumbnailKind::Code(language)),
            "{name}"
        );
    }
    for name in [
        "disk.iso",
        "drive.img",
        "disc.bin",
        "disc.cue",
        "bundle.zip",
        "backup.7z",
        "files.tar",
        "release.tar.gz",
        "data.rar",
        "installer.dmg",
        "report.docx",
        "sheet.xlsx",
        "slides.pptx",
        "deck.key",
        "letter.odt",
        "show.ppsx",
        "template.dotx",
        "macro.xlsm",
        "type.ttf",
        "type.otf",
        "type.woff",
        "type.woff2",
    ] {
        assert_eq!(thumbnail_kind(Path::new(name)), None, "{name}");
    }
    assert_eq!(
        thumbnail_kind(Path::new("artwork.psd")),
        Some(ThumbnailKind::Image)
    );
    assert_eq!(
        thumbnail_kind(Path::new("logo.ai")),
        Some(ThumbnailKind::Pdf)
    );
}

#[test]
fn rejects_files_without_a_thumbnail_provider() {
    assert_eq!(thumbnail_kind(Path::new("backup.bak")), None);
    assert_eq!(thumbnail_kind(Path::new("ebook.kfx")), None);
    assert_eq!(thumbnail_kind(Path::new("model.fbx")), None);
    assert_eq!(thumbnail_kind(Path::new("no-extension")), None);
}

#[test]
fn fallback_art_maps_extensions_to_category_icons() {
    let cases: [(&str, &str); 40] = [
        ("photo.png", IMAGE_ART),
        ("clip.flv", VIDEO_ART),
        ("font.ttf", FONT_ART),
        ("book.kfx", EBOOKS_ART),
        ("issue.cb7", COMICS_ART),
        ("map.kmz", MAPS_ART),
        ("deck.key", PPTX_ART),
        ("legacy.doc", DOCX_ART),
        ("model.glb", MODELS_3D_ART),
        ("sub.srt", SUBTITLES_ART),
        ("list.m3u", PLAYLISTS_ART),
        ("design.dwg", DESIGN_ART),
        ("disc.iso", ISO_ART),
        ("pack.7z", ARCHIVE_ART),
        ("song.wma", AUDIO_ART),
        ("main.rs", TEXT_ART),
        ("macro.xlsm", SPREADSHEET_ART),
        ("show.ppsx", PPTX_ART),
        ("app.ipa", PACKAGE_ART),
        ("disk.vdi", VIRTUAL_DISK_ART),
        ("flatpak.ova", VM_ART),
        ("score.mscz", MUSIC_ART),
        ("lidar.step", DESIGN_ART),
        ("scene.c4d", MODELS_3D_ART),
        ("book.lrf", EBOOKS_ART),
        ("tiles.mbtiles", MAPS_ART),
        ("machine.ovf", VM_ART),
        ("database.db", DATABASE_ART),
        ("data.sqlite3", DATABASE_ART),
        ("query.sql", SQL_ART),
        ("cert.p12", CERT_ART),
        ("site.pem", CERT_ART),
        ("scope.fits", SCIENCE_ART),
        ("page.html", WEB_ART),
        ("app.ini", CONFIG_ART),
        ("secrets.env", CONFIG_ART),
        ("sys.log", LOG_ART),
        ("song.mid", MUSIC_ART),
        ("proj.flp", AUDIO_PROJECT_ART),
        ("sheet.csv", SPREADSHEET_ART),
    ];
    for (name, art) in cases {
        assert_eq!(fallback_art_source(Path::new(name)), Some(art), "{name}");
        assert!(super::fallback_art(Path::new(name)).is_some(), "{name}");
    }
    assert_eq!(fallback_art_source(Path::new("binary.dat")), None);
    assert_eq!(fallback_art_source(Path::new("no-extension")), None);
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

#[test]
fn renamed_thumbnail_rebind_reuses_texture_without_a_request() {
    gtk_test(
        "ui::thumbnail::tests::renamed_thumbnail_rebind_reuses_texture_without_a_request",
        || {
            for change in ["name", "size", "mtime", "unknown", "extension"] {
                clear_thumbnail_runtime();
                let from = Path::new("/rename/before.png");
                let to = Path::new(if change == "extension" {
                    "/rename/after.mp4"
                } else {
                    "/rename/after.png"
                });
                let pixels = glib::Bytes::from_owned(vec![255u8; 4]);
                let texture: gdk::Texture =
                    gdk::MemoryTexture::new(1, 1, gdk::MemoryFormat::R8g8b8a8, &pixels, 4).upcast();
                let key = ThumbnailKey {
                    path: from.to_path_buf(),
                    modified: Some(1),
                    file_size: Some(1),
                    thumbnail_size: crate::ui::thumbnail_cache::CANONICAL_MAX_EDGE,
                };
                THUMBNAIL_CACHE.with_borrow_mut(|cache| cache.insert(key, texture.clone()));
                let mut entry = sample_entry(to);
                match change {
                    "size" => entry.size = MetadataValue::Known(2),
                    "mtime" => entry.modified_unix_seconds = MetadataValue::Known(2),
                    "unknown" => entry.modified_unix_seconds = MetadataValue::Unknown,
                    _ => {}
                }
                super::preserve_renamed_thumbnail(&Location::local(from), &entry);
                let slot = super::ThumbnailSlot::new(64);
                set_thumbnail_or_icon(&slot, &entry, crate::assets::icons::PICTURES, 32, 64);
                if change != "name" {
                    assert!(slot.texture().is_none());
                    assert!(
                        ACTIVE_REQUESTS.with_borrow(
                            |requests| requests.contains_key(&(slot.as_ptr() as usize))
                        )
                    );
                } else {
                    assert_eq!(slot.texture(), Some(texture));
                    assert!(
                        !ACTIVE_REQUESTS.with_borrow(
                            |requests| requests.contains_key(&(slot.as_ptr() as usize))
                        )
                    );
                }
                cancel_thumbnail(slot.as_ptr() as usize);
            }
            clear_thumbnail_runtime();
        },
    );
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
            super::super::preferences::PreferenceManager::shared();
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
fn compact_text_and_code_icons_keep_fallbacks_instead_of_unreadable_content() {
    gtk_test(
        "ui::thumbnail::tests::compact_text_and_code_icons_keep_fallbacks_instead_of_unreadable_content",
        || {
            super::super::preferences::PreferenceManager::shared();
            for path in [
                PathBuf::from("/fixture/main.rs"),
                PathBuf::from("/fixture/notes.txt"),
            ] {
                let image = super::ThumbnailSlot::new(18);
                super::set_thumbnail_or_icon(
                    &image,
                    &sample_entry(&path),
                    crate::assets::icons::DOCUMENTS,
                    18,
                    18,
                );
                assert!(displayed_texture(&image).is_none(), "{}", path.display());
                assert!(!has_pending_thumbnail(&path), "{}", path.display());
            }
            clear_thumbnail_runtime();
        },
    );
}

fn thumbnail_theme(name: &str, surface: &str, accent: &str, border: &str) -> ThemeTokens {
    ThemeTokens {
        name: name.into(),
        background: "#101010".into(),
        surface: surface.into(),
        text: "#f0f0f0".into(),
        accent: accent.into(),
        danger: "#ff4466".into(),
        muted: "#303030".into(),
        highlight: "#505050".into(),
        border: border.into(),
        dim_text: "#909090".into(),
        syntax_keyword: Some("#cc66ff".into()),
        syntax_string: Some("#66dd99".into()),
        syntax_constant: Some("#ffbb55".into()),
        syntax_type: Some("#55bbff".into()),
        syntax_preprocessor: Some("#ff77aa".into()),
    }
}

#[test]
fn fallback_art_tracks_live_theme_colors() {
    gtk_test(
        "ui::thumbnail::tests::fallback_art_tracks_live_theme_colors",
        || {
            let manager = super::super::theme::ThemeManager::shared();
            let slot = super::ThumbnailSlot::new(64);
            manager.preview(&thumbnail_theme("First", "#121722", "#16a8ff", "#304050"));
            show_customized_icon(
                &slot,
                Path::new("book.kfx"),
                crate::assets::icons::DOCUMENTS,
                64,
            );
            let first = fallback_pixels(&slot);

            manager.preview(&thumbnail_theme("Second", "#f5e8d0", "#c026d3", "#8a6540"));
            let second = fallback_pixels(&slot);
            assert_ne!(first, second);

            manager.cancel_preview();
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn audio_spectrum_tracks_live_theme_colors() {
    gtk_test(
        "ui::thumbnail::tests::audio_spectrum_tracks_live_theme_colors",
        || {
            let manager = super::super::theme::ThemeManager::shared();
            let mut mask = vec![0_u8; 16 * 16 * 4];
            for y in 3..13 {
                for x in (3..13).step_by(3) {
                    let offset = (y * 16 + x) * 4;
                    mask[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
            let bytes = glib::Bytes::from_owned(mask);
            let source =
                gdk::MemoryTexture::new(16, 16, gdk::MemoryFormat::R8g8b8a8, &bytes, 16 * 4)
                    .upcast();
            let slot = super::ThumbnailSlot::new(64);

            manager.preview(&thumbnail_theme("First", "#121722", "#16a8ff", "#304050"));
            super::apply_thumbnail(&slot, &source, Path::new("song.mp3"));
            let first = texture_pixels(&slot.texture().expect("themed audio texture"));

            manager.preview(&thumbnail_theme("Second", "#f5e8d0", "#c026d3", "#8a6540"));
            let second = texture_pixels(&slot.texture().expect("refreshed audio texture"));
            assert_ne!(first, second);

            manager.cancel_preview();
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn theme_refresh_does_not_reenter_tracked_icon_refcell() {
    gtk_test(
        "ui::thumbnail::tests::theme_refresh_does_not_reenter_tracked_icon_refcell",
        || {
            super::super::preferences::PreferenceManager::shared();
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
fn path_customization_refreshes_rendered_icons() {
    gtk_test(
        "ui::thumbnail::tests::path_customization_refreshes_rendered_icons",
        || {
            let manager = PreferenceManager::shared();
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

            manager.set_folder_color(customized, Some(color.clone()));
            assert_eq!(fallback_pixels(&customized_slot), colored_pixels);
            assert_eq!(fallback_pixels(&other_slot), default_pixels);

            manager.set_custom_icon(customized, Some(crate::assets::icons::HOME));
            assert_eq!(fallback_pixels(&customized_slot), decorated_pixels);
            assert_eq!(fallback_pixels(&other_slot), default_pixels);

            manager.clear_item_customization(customized);
            assert_eq!(fallback_pixels(&customized_slot), default_pixels);
            assert_eq!(fallback_pixels(&other_slot), default_pixels);

            manager.set_folder_color(customized, Some(color.clone()));
            show_customized_icon(&customized_slot, other, crate::assets::icons::FOLDER, 32);
            assert_eq!(fallback_pixels(&customized_slot), default_pixels);
            manager.clear_item_customization(customized);
            manager.set_folder_color(other, Some(color));
            assert_eq!(fallback_pixels(&customized_slot), colored_pixels);
            assert_eq!(fallback_pixels(&other_slot), colored_pixels);

            super::show_fallback_icon(&customized_slot, crate::assets::icons::PICTURES, 32);
            let fallback = fallback_pixels(&customized_slot);
            manager.clear_item_customization(other);
            refresh_all_customized_icons();
            assert_eq!(fallback_pixels(&customized_slot), fallback);
            assert_eq!(fallback_pixels(&other_slot), default_pixels);
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn recycled_and_disposed_slots_release_thumbnail_tracking() {
    gtk_test(
        "ui::thumbnail::tests::recycled_and_disposed_slots_release_thumbnail_tracking",
        || {
            let path = Path::new("/fixture/old.png");
            let replacement = Path::new("/fixture/new.png");
            let slot = super::ThumbnailSlot::new(64);
            let other = super::ThumbnailSlot::new(64);
            let texture = sample_texture();
            for image in [&slot, &other] {
                show_customized_icon(image, path, crate::assets::icons::PICTURES, 64);
                super::apply_thumbnail(image, &texture, path);
            }
            super::apply_thumbnail(&slot, &texture, replacement);
            assert!(!super::displayed_thumbnail_matches(&slot, path));
            assert!(super::displayed_thumbnail_matches(&slot, replacement));
            assert!(super::displayed_thumbnail_matches(&other, path));

            let id = slot.as_ptr() as usize;
            let weak = slot.downgrade();
            drop(slot);
            assert!(weak.upgrade().is_none());
            assert!(!super::TRACKED_THUMBNAILS.with_borrow(|tracked| tracked.contains_key(&id)));
            assert!(
                !super::TRACKED_CUSTOMIZED_ICONS.with_borrow(|tracked| tracked.contains_key(&id))
            );
            assert!(super::displayed_thumbnail_matches(&other, path));
            super::show_fallback_icon(&other, crate::assets::icons::PICTURES, 64);
            assert!(!super::displayed_thumbnail_matches(&other, path));
            assert!(other.texture().is_none());
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
            super::super::preferences::PreferenceManager::shared();
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
fn uri_entries_with_a_local_mirror_render_via_the_mirror_path() {
    gtk_test(
        "ui::thumbnail::tests::uri_entries_with_a_local_mirror_render_via_the_mirror_path",
        || {
            super::super::preferences::PreferenceManager::shared();
            hold_thumbnail_workers();
            // file:// exercises URI routing, not GVfs/FUSE integration.
            let mirror = tempfile::Builder::new()
                .suffix(".png")
                .tempfile()
                .expect("temp mirror file");
            let entry = FileEntry {
                recent_unix_seconds: MetadataValue::Unavailable,
                location: Location::uri(gio::File::for_path(mirror.path()).uri()),
                thumbnail_path: None,
                native_name: "photo.png".into(),
                display_name: "photo.png".to_owned(),
                kind: EntryKind::File,
                size: MetadataValue::Known(1),
                modified_unix_seconds: MetadataValue::Known(1),
                mode: MetadataValue::Unavailable,
                is_hidden: false,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            };
            let image = super::ThumbnailSlot::new(64);
            bind_thumbnail(&image, &entry);
            drain_main_loop();
            assert!(has_pending_thumbnail(mirror.path()));
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn mirror_rendering_does_not_wait_for_a_metadata_fill() {
    gtk_test(
        "ui::thumbnail::tests::mirror_rendering_does_not_wait_for_a_metadata_fill",
        || {
            super::super::preferences::PreferenceManager::shared();
            hold_thumbnail_workers();
            let mirror = tempfile::Builder::new()
                .suffix(".png")
                .tempfile()
                .expect("temp mirror file");
            let entry = FileEntry {
                recent_unix_seconds: MetadataValue::Unavailable,
                location: Location::uri(gio::File::for_path(mirror.path()).uri()),
                thumbnail_path: None,
                native_name: "photo.png".into(),
                display_name: "photo.png".to_owned(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                mode: MetadataValue::Unavailable,
                is_hidden: false,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            };
            let image = super::ThumbnailSlot::new(64);
            bind_thumbnail(&image, &entry);
            drain_main_loop();
            assert!(
                has_pending_thumbnail(mirror.path()),
                "mirror rendering must not depend on a metadata producer"
            );
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn uri_entries_without_a_local_mirror_fall_back_to_a_generic_icon() {
    gtk_test(
        "ui::thumbnail::tests::uri_entries_without_a_local_mirror_fall_back_to_a_generic_icon",
        || {
            super::super::preferences::PreferenceManager::shared();
            hold_thumbnail_workers();
            let entry = FileEntry {
                recent_unix_seconds: MetadataValue::Unavailable,
                location: Location::uri("smb://example.invalid/share/photo.png"),
                thumbnail_path: None,
                native_name: "photo.png".into(),
                display_name: "photo.png".to_owned(),
                kind: EntryKind::File,
                size: MetadataValue::Known(1),
                modified_unix_seconds: MetadataValue::Known(1),
                mode: MetadataValue::Unavailable,
                is_hidden: false,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            };
            let image = super::ThumbnailSlot::new(64);
            bind_thumbnail(&image, &entry);
            drain_main_loop();
            assert!(PENDING_THUMBNAILS.with(|pending| pending.borrow().is_empty()));
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn stale_request_id_does_not_apply_completed_texture() {
    gtk_test(
        "ui::thumbnail::tests::stale_request_id_does_not_apply_completed_texture",
        || {
            super::super::preferences::PreferenceManager::shared();
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
