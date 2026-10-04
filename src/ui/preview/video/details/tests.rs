// SPDX-License-Identifier: MIT

use std::time::Duration;

use super::*;
use crate::sandbox::MediaPreviewBackend;

#[test]
fn failed_probes_are_retried_and_cancelled_loads_never_publish() {
    crate::test_support::gtk_test(
        "ui::preview::video::details::tests::failed_probes_are_retried_and_cancelled_loads_never_publish",
        || {
            let directory = tempfile::tempdir().expect("clip directory");
            let path = directory.path().join("clip.mp4");
            let entry = crate::ui::preview::tests::entry(path.to_str().expect("fixture path"));
            let source = SandboxedMedia {
                path: path.clone(),
                size: crate::services::MediaPreviewSize::new(320, 240),
                backend: MediaPreviewBackend::Software,
                input_owner: None,
                audio_only: false,
            };
            let key = TrackKey::of(&entry);
            let load = |results: Rc<RefCell<Vec<Option<Rc<MediaMetadata>>>>>| {
                load_details_with(
                    &entry,
                    &source,
                    move |details| results.borrow_mut().push(details),
                    |path, operation, _| {
                        assert_eq!(operation, ParseOperation::MediaMetadata);
                        (std::fs::read(path).ok()? != b"fail").then(|| {
                            br#"{"streams":[{"codec_type":"video","codec_name":"av1","width":1920,"height":1080}]}"#.to_vec()
                        })
                    },
                )
            };
            let cancelled = Rc::new(RefCell::new(Vec::new()));
            drop(load(cancelled.clone()));

            std::fs::write(&path, "fail").expect("simulate failure");
            let failed = Rc::new(RefCell::new(Vec::new()));
            let pending = load(failed.clone());
            wait(|| !failed.borrow().is_empty());
            assert_eq!(*failed.borrow(), vec![None]);
            assert!(cached_details(&key).is_none(), "failures are not cached");
            drop(pending);

            std::fs::write(&path, "ok").expect("restore the probe");
            let results = Rc::new(RefCell::new(Vec::new()));
            let pending = load(results.clone());
            wait(|| !results.borrow().is_empty());
            let details = cached_details(&key).expect("successful probes are cached");
            assert_eq!(details.video_codec.as_deref(), Some("av1"));
            assert_eq!(results.borrow()[0].as_deref(), Some(&*details));
            drop(pending);
            std::thread::sleep(LOAD_SETTLE * 2);
            for _ in 0..20 {
                glib::MainContext::default().iteration(false);
            }
            assert!(
                cancelled.borrow().is_empty(),
                "a dropped load never publishes"
            );
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
