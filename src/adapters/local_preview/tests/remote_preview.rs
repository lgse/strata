// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::PreviewRequestId,
};

fn request(path: &Path, name: &str) -> PreviewRequest {
    PreviewRequest {
        id: PreviewRequestId(91),
        entry: FileEntry {
            location: Location::uri(gio::File::for_path(path).uri()),
            thumbnail_path: None,
            native_name: name.into(),
            display_name: name.into(),
            kind: EntryKind::File,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Known(1),
            mode: MetadataValue::Unknown,
            is_hidden: false,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
        },
        text_byte_limit: 1024,
        render_document: false,
        pdf_page: 0,
        media_size: MediaPreviewSize::new(640, 800),
    }
}

#[test]
fn uri_video_retains_one_private_input_through_player_clones_and_worker_exit() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::remote_preview::uri_video_retains_one_private_input_through_player_clones_and_worker_exit",
        || {
            use std::os::unix::fs::PermissionsExt;
            let fixture = tempfile::tempdir().expect("video fixture");
            let context = glib::MainContext::default();
            let _owner = context.acquire().expect("context");
            let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
            for name in ["clip.MOV", "clip.mp4"] {
                let original = fixture.path().join(name);
                fs::write(&original, b"remote video").expect("fixture");
                let mut request = request(&original, name);
                request.entry.size = MetadataValue::Known(100 * 1024 * 1024);
                let location = request.entry.location.clone();
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let handle = provider.load(
                    request,
                    Rc::new(move |event| emitted.borrow_mut().push(event)),
                );
                context.block_on(async {
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    while events.borrow().is_empty() && std::time::Instant::now() < deadline {
                        glib::timeout_future(Duration::from_millis(1)).await;
                    }
                });
                let event = events.borrow_mut().pop().expect("video ready");
                let PreviewEvent::Ready(preview) = event else {
                    panic!("video staging failed: {event:?}");
                };
                assert_eq!(preview.entry.location, location);
                let PreviewContent::SandboxedMedia { media } = preview.content else {
                    panic!("sandboxed video descriptor required");
                };
                let staged = media.path.clone();
                assert_ne!(staged, original);
                assert!(media.input_owner.is_some());
                assert_eq!(
                    fs::metadata(&staged)
                        .expect("staged metadata")
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
                let mut seek = media.clone();
                seek.size = MediaPreviewSize::new(320, 200);
                assert_eq!(seek.path, staged);
                let worker_source = seek.clone();
                let (release, wait) = std::sync::mpsc::channel();
                let worker = std::thread::spawn(move || {
                    let source = worker_source;
                    wait.recv_timeout(Duration::from_secs(5))
                        .expect("release worker");
                    assert_eq!(
                        fs::read(&source.path).expect("worker input"),
                        b"remote video"
                    );
                    drop(source);
                });
                drop(handle);
                drop(media);
                drop(seek);
                assert!(
                    staged.exists(),
                    "closing the player must not delete an active worker input"
                );
                release.send(()).expect("release");
                worker.join().expect("worker");
                assert!(!staged.exists(), "last worker releases the staged input");
                assert_eq!(
                    fs::read(&original).expect("original unchanged"),
                    b"remote video"
                );
                assert!(crate::ui::thumbnail_cache::lookup(&staged, 1).is_none());
            }
            let original = fixture.path().join("oversized.mov");
            let mut request = request(&original, "oversized.mov");
            request.entry.size = MetadataValue::Known(257 * 1024 * 1024);
            let events = Rc::new(RefCell::new(Vec::new()));
            let emitted = events.clone();
            let _handle = provider.load(
                request,
                Rc::new(move |event| emitted.borrow_mut().push(event)),
            );
            context.block_on(async {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while events.borrow().is_empty() && std::time::Instant::now() < deadline {
                    glib::timeout_future(Duration::from_millis(1)).await;
                }
            });
            assert!(
                matches!(&events.borrow()[0], PreviewEvent::Failed { message, .. } if message.contains("256 MiB"))
            );
        },
    );
}

#[test]
fn uri_images_stage_private_inputs_and_remove_them_after_rendering() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::remote_preview::uri_images_stage_private_inputs_and_remove_them_after_rendering",
        || {
            use std::{
                os::unix::fs::PermissionsExt,
                sync::{Arc, Mutex},
            };
            let fixture = tempfile::tempdir().expect("fixture directory");
            let context = glib::MainContext::default();
            let _owner = context.acquire().expect("main context owner");
            let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
            for name in ["photo.jpg", "photo.heic"] {
                for fail in [false, true] {
                    let original = fixture.path().join(name);
                    fs::write(&original, b"remote content").expect("fixture image");
                    let request = request(&original, name);
                    let location = request.entry.location.clone();
                    let staged_path = Arc::new(Mutex::new(None));
                    let rendered_path = staged_path.clone();
                    let events = Rc::new(RefCell::new(Vec::new()));
                    let emitted = events.clone();
                    let handle = provider.load_with_renderer(
                        request,
                        Rc::new(move |event| emitted.borrow_mut().push(event)),
                        move |path, operation, _, _, _| {
                            assert_ne!(path, original);
                            assert_eq!(fs::read(path).expect("staged input"), b"remote content");
                            assert_eq!(
                                fs::metadata(path)
                                    .expect("input permissions")
                                    .permissions()
                                    .mode()
                                    & 0o777,
                                0o600
                            );
                            assert_eq!(path.extension(), original.extension());
                            assert!(matches!(operation, ParseOperation::PreviewImage));
                            *rendered_path.lock().expect("path lock") = Some(path.to_owned());
                            if fail {
                                Err("decoder failure".into())
                            } else {
                                Ok(crate::sandbox::ParseOutput {
                                    data: b"rendered".to_vec(),
                                    page: 0,
                                    pages: 2,
                                })
                            }
                        },
                    );
                    context.block_on(async {
                        let deadline = std::time::Instant::now() + Duration::from_secs(5);
                        while events.borrow().is_empty() && std::time::Instant::now() < deadline {
                            glib::timeout_future(Duration::from_millis(1)).await;
                        }
                    });
                    assert_eq!(events.borrow().len(), 1);
                    match &events.borrow()[0] {
                        PreviewEvent::Ready(preview) => {
                            assert!(!fail);
                            assert_eq!(preview.entry.location, location);
                        }
                        PreviewEvent::Failed { message, .. } => {
                            assert!(fail);
                            assert_eq!(message, "decoder failure");
                        }
                    }
                    let path = staged_path
                        .lock()
                        .expect("path lock")
                        .clone()
                        .expect("renderer ran");
                    assert!(
                        !path.exists(),
                        "staged original removed after renderer exits"
                    );
                    assert!(crate::ui::thumbnail_cache::lookup(&path, 1).is_none());
                    assert!(
                        PREVIEW_CACHE.with(|cache| cache
                            .borrow()
                            .entries
                            .keys()
                            .all(|key| key.path != path))
                    );
                    drop(handle);
                }
            }
        },
    );
}

#[test]
fn cancelling_remote_image_render_retains_input_until_decoder_exits() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::remote_preview::cancelling_remote_image_render_retains_input_until_decoder_exits",
        || {
            let fixture = tempfile::tempdir().expect("fixture directory");
            let original = fixture.path().join("photo.jpg");
            fs::write(&original, b"remote photo").expect("fixture photo");
            let context = glib::MainContext::default();
            let _owner = context.acquire().expect("main context owner");
            let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
            let events = Rc::new(RefCell::new(Vec::new()));
            let emitted = events.clone();
            let (started, receive_started) = oneshot::channel();
            let (finish, receive_finish) = std::sync::mpsc::channel();
            let handle = provider.load_with_renderer(
                request(&original, "photo.jpg"),
                Rc::new(move |event| emitted.borrow_mut().push(event)),
                move |path, _, _, _, cancellation| {
                    started
                        .send(path.to_owned())
                        .expect("notify renderer started");
                    receive_finish
                        .recv_timeout(Duration::from_secs(5))
                        .expect("release renderer");
                    assert!(cancellation.is_cancelled());
                    assert!(path.exists());
                    Err("cancelled renderer".into())
                },
            );
            context.block_on(async {
                let path = receive_started.await.expect("renderer started");
                drop(handle);
                glib::timeout_future(Duration::from_millis(10)).await;
                assert!(path.exists());
                finish.send(()).expect("release renderer");
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while path.exists() && std::time::Instant::now() < deadline {
                    glib::timeout_future(Duration::from_millis(2)).await;
                }
                assert!(!path.exists());
            });
            assert!(events.borrow().is_empty());
            assert_eq!(
                fs::read(original).expect("unchanged original"),
                b"remote photo"
            );
        },
    );
}

#[test]
fn unsupported_and_oversized_remote_inputs_fail_before_transfer_or_render() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::remote_preview::unsupported_and_oversized_remote_inputs_fail_before_transfer_or_render",
        || {
            let context = glib::MainContext::default();
            let _owner = context.acquire().expect("main context owner");
            let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
            for (name, size, message) in [
                ("remote.pdf", MetadataValue::Unknown, "Remote PDF previews"),
                (
                    "photo.jpg",
                    MetadataValue::Known(65 * 1024 * 1024),
                    "64 MiB",
                ),
            ] {
                let mut request = request(&Path::new("/nonexistent").join(name), name);
                request.entry.size = size;
                let (send, receive) = oneshot::channel();
                let send = RefCell::new(Some(send));
                let handle = provider.load_with_renderer(
                    request,
                    Rc::new(move |event| {
                        send.borrow_mut()
                            .take()
                            .expect("single event")
                            .send(event)
                            .ok();
                    }),
                    |_, _, _, _, _| panic!("rejected input reached renderer"),
                );
                let event = context.block_on(receive).expect("failure event");
                let PreviewEvent::Failed {
                    message: actual, ..
                } = event
                else {
                    panic!("expected rejection")
                };
                assert!(actual.contains(message), "{actual}");
                drop(handle);
            }
        },
    );
}
