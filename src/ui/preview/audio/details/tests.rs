// SPDX-License-Identifier: MIT

use super::*;
use gtk::prelude::*;

fn png(value: u8) -> Vec<u8> {
    gdk::MemoryTexture::new(
        1,
        1,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(vec![value, 0, 0, 255]),
        4,
    )
    .save_to_png_bytes()
    .to_vec()
}

#[test]
fn folder_art_reuses_decodes_and_invalidates_changed_files() {
    crate::test_support::gtk_test(
        "ui::preview::audio::details::tests::folder_art_reuses_decodes_and_invalidates_changed_files",
        || {
            let directory = tempfile::tempdir().expect("art directory");
            let job = Cancellation::default();
            let decodes = Cell::new(0);
            let parse = |path: &Path, _: ParseOperation, _: &Cancellation| {
                decodes.set(decodes.get() + 1);
                std::fs::read(path).ok()
            };
            let load =
                || load_folder_art(directory.path(), &job, &job, &parse).expect("folder lookup");
            assert!(load().is_none());
            let path = directory.path().join("cover.png");
            std::fs::write(&path, png(10)).expect("write artwork");
            let activity = crate::services::file_activity::begin(
                crate::services::file_activity::OperationScope::default(),
            );
            let fence = activity.fence([directory.path().to_owned()]);
            assert!(
                load_folder_art(directory.path(), &job, &job, &parse).is_none(),
                "a folder an operation fences is not listed"
            );
            drop(fence);
            assert_eq!(decodes.get(), 0);
            let first = load().expect("first cover");
            let second = load().expect("cached cover");
            assert_eq!(decodes.get(), 1);
            assert_eq!(
                first.texture, second.texture,
                "tracks in the same folder reuse the decoded art"
            );
            let copy = texture(png(10)).expect("cover copy");
            assert!(
                first == copy,
                "identical artwork compares by content, not texture identity"
            );
            // Saved under a new inode, so the change shows even within one timestamp tick.
            let replacement = directory.path().join("replacement.png");
            std::fs::write(&replacement, png(200)).expect("write replacement");
            std::fs::rename(&replacement, &path).expect("replace artwork");
            let changed = load().expect("changed cover");
            assert_eq!(decodes.get(), 2);
            assert!(first != changed);
            std::fs::remove_file(path).expect("remove artwork");
            assert!(load().is_none());
        },
    );
}

#[test]
fn failed_details_are_retried_and_cancelled_jobs_never_publish() {
    crate::test_support::gtk_test(
        "ui::preview::audio::details::tests::failed_details_are_retried_and_cancelled_jobs_never_publish",
        || {
            let directory = tempfile::tempdir().expect("track directory");
            let path = directory.path().join("song.wav");
            let entry = crate::ui::preview::tests::entry(path.to_str().expect("fixture path"));
            let source = SandboxedMedia {
                path: path.clone(),
                size: crate::services::MediaPreviewSize::new(320, 240),
                backend: MediaPreviewBackend::Software,
                input_owner: None,
                audio_only: true,
            };
            let key = TrackKey::of(&entry);
            let load = |called: Rc<Cell<u32>>| {
                let tags = called.clone();
                load_details_with(
                    &entry,
                    &source,
                    move |_| tags.set(tags.get() + 1),
                    move |_| called.set(called.get() + 1),
                    |path, operation, _| {
                        let data = std::fs::read(path).ok()?;
                        match operation {
                            ParseOperation::AudioTags if data != b"tags-fail" => {
                                Some(br#"{"format":{"tags":{"title":"Recovered"}}}"#.to_vec())
                            }
                            ParseOperation::AudioCover if data != b"cover-fail" => {
                                Some(b"null".to_vec())
                            }
                            _ => None,
                        }
                    },
                )
            };
            let cancelled = Rc::new(Cell::new(0));
            drop(load(cancelled.clone()));
            for failure in ["tags-fail", "cover-fail"] {
                std::fs::write(&path, failure).expect("simulate failure");
                let calls = Rc::new(Cell::new(0));
                let pending = load(calls.clone());
                wait(|| calls.get() == 2);
                assert_eq!(cancelled.get(), 0);
                assert!(
                    cached_details(&key).is_none(),
                    "{failure} is not a completed empty result"
                );
                drop(pending);
            }
            std::fs::write(&path, "ok").expect("restore lookups");
            let calls = Rc::new(Cell::new(0));
            let pending = load(calls.clone());
            wait(|| calls.get() == 2);
            let details = cached_details(&key).expect("successful missing artwork is cacheable");
            assert_eq!(details.tags.title.as_deref(), Some("Recovered"));
            assert!(details.cover.is_none());
            drop(pending);

            let album = tempfile::tempdir().expect("album directory");
            let song = album.path().join("song.wav");
            std::fs::write(&song, "ok").expect("song");
            std::fs::write(album.path().join("cover.png"), png(10)).expect("folder cover");
            let entry = crate::ui::preview::tests::entry(song.to_str().expect("fixture path"));
            let source = SandboxedMedia {
                path: song,
                ..source
            };
            let cover_stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stopped = cover_stopped.clone();
            let title = Rc::new(RefCell::new(None));
            let covers = Rc::new(Cell::new(0));
            let (shown, cover_calls) = (title.clone(), covers.clone());
            let pending = load_details_with(
                &entry,
                &source,
                move |tags| *shown.borrow_mut() = Some(tags.title),
                move |_| cover_calls.set(cover_calls.get() + 1),
                move |path, operation, cancellation| match operation {
                    ParseOperation::AudioTags => {
                        let deadline = std::time::Instant::now() + Duration::from_secs(10);
                        while !stopped.load(std::sync::atomic::Ordering::SeqCst)
                            && std::time::Instant::now() < deadline
                        {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        (!cancellation.is_cancelled())
                            .then(|| br#"{"format":{"tags":{"title":"Kept"}}}"#.to_vec())
                    }
                    ParseOperation::AudioCover => Some(b"null".to_vec()),
                    _ => {
                        let stop = cancellation.clone();
                        let _hold = file_activity::hold([path], move || stop.cancel()).ok()?;
                        let activity =
                            file_activity::begin(file_activity::OperationScope::default());
                        activity.fence([path.to_owned()]).stop_reads();
                        stopped.store(true, std::sync::atomic::Ordering::SeqCst);
                        None
                    }
                },
            );
            wait(|| covers.get() == 1 && title.borrow().is_some());
            assert_eq!(
                *title.borrow(),
                Some(Some("Kept".to_owned())),
                "an operation on the folder cover leaves the song's tags"
            );
            assert!(cached_details(&TrackKey::of(&entry)).is_none());
            drop(pending);
        },
    );
}

fn wait(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "details did not complete"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}
