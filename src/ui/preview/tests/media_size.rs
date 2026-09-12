// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        LoadHandle, MediaPreviewSize, Preview, PreviewContent, PreviewEvent, PreviewProvider,
        PreviewRequest, PreviewRequestId, SandboxedMedia,
    },
    ui::{preview::PreviewDrawer, theme::ThemeManager},
};

struct RecordingProvider(Rc<RefCell<Vec<PreviewRequest>>>);

impl PreviewProvider for RecordingProvider {
    fn load(&self, request: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.0.borrow_mut().push(request);
        LoadHandle::new(|| {})
    }
}

fn entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/tmp/{name}")),
        native_name: name.into(),
        thumbnail_path: None,
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(100),
        modified_unix_seconds: MetadataValue::Known(1),
        mode: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "preview layout did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn decoded_frames_play_in_the_browser_and_chooser_preview_widgets() {
    crate::test_support::gtk_test(
        "ui::preview::tests::media_size::decoded_frames_play_in_the_browser_and_chooser_preview_widgets",
        || {
            for browser in [true, false] {
                let source = SandboxedMedia {
                    path: "/synthetic-video.mp4".into(),
                    size: MediaPreviewSize::new(320, 180),
                    backend: crate::sandbox::MediaPreviewBackend::Software,
                };
                let drawer = PreviewDrawer::new(
                    Rc::new(RecordingProvider(Rc::new(RefCell::new(Vec::new())))),
                    browser,
                );
                let window = gtk::Window::builder()
                    .default_width(600)
                    .default_height(800)
                    .child(&drawer.widget())
                    .build();
                drawer.state.revealer.set_reveal_child(true);
                drawer.state.opened.set(true);
                drawer.state.current_request.set(Some(PreviewRequestId(1)));
                drawer.state.render(Preview {
                    request_id: PreviewRequestId(1),
                    entry: entry("clip.mp4"),
                    content_type: "video/mp4".into(),
                    content: PreviewContent::SandboxedMedia {
                        media: source.clone(),
                    },
                });
                let media = drawer
                    .state
                    .media
                    .borrow()
                    .as_ref()
                    .expect("media stream")
                    .clone();
                let decoded = media
                    .downcast_ref::<crate::ui::media::DecodedMedia>()
                    .expect("raw texture player, not GtkMediaFile");
                crate::ui::media::tests::use_test_decoder(decoded, true, 2_000_000);
                window.present();
                wait_until(|| {
                    assert!(media.error().is_none(), "{:?}", media.error());
                    media.is_prepared() && media.timestamp() > 0
                });
                assert!(media.has_video());
                assert!(media.has_audio());
                assert!(media.downcast_ref::<gtk::MediaFile>().is_none());
                window.close();
                wait_until(|| drawer.state.media.borrow().is_none());
                assert_eq!(decoded.intrinsic_width(), 0);
                assert!(!decoded.is_playing());
                assert!(!drawer.is_open());
                drawer.state.handle_event(
                    PreviewRequestId(1),
                    PreviewEvent::Ready(Preview {
                        request_id: PreviewRequestId(1),
                        entry: entry("late.mp4"),
                        content_type: "video/mp4".into(),
                        content: PreviewContent::SandboxedMedia { media: source },
                    }),
                );
                assert!(
                    drawer.state.media.borrow().is_none(),
                    "destroyed windows reject late provider results"
                );
                drawer.close();
            }
        },
    );
}

#[test]
fn media_requests_use_the_opening_target_and_each_windows_resized_pane() {
    crate::test_support::gtk_test(
        "ui::preview::tests::media_size::media_requests_use_the_opening_target_and_each_windows_resized_pane",
        || {
            ThemeManager::shared().set_reduce_motion(false);
            let mut windows = Vec::new();
            for (allow_external_open, window_width) in [(true, 1400), (false, 1600)] {
                let requests = Rc::new(RefCell::new(Vec::new()));
                let drawer = PreviewDrawer::new(
                    Rc::new(RecordingProvider(requests.clone())),
                    allow_external_open,
                );
                let split = gtk::Paned::new(gtk::Orientation::Horizontal);
                split.set_start_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
                drawer.attach_split(&split, Rc::new(|| 400));
                let window = gtk::Window::builder()
                    .default_width(window_width)
                    .default_height(800)
                    .child(&split)
                    .build();
                window.present();
                wait_until(|| split.width() > 0 && split.height() > 0);
                let opening_width = drawer.state.opening_width(split.width());
                drawer.show(entry("first.mp4"), None);
                let first = requests.borrow()[0].media_size;
                assert_eq!(
                    first.width,
                    MediaPreviewSize::for_viewport(
                        opening_width,
                        800,
                        drawer.state.pane.scale_factor()
                    )
                    .width
                );
                assert!(first.height > 16);
                wait_until(|| !drawer.state.animating.get() && drawer.state.content.height() > 0);
                split.set_position(split.width() - super::super::MIN_WIDTH);
                wait_until(|| {
                    drawer.state.content.width() > 0
                        && drawer.state.content.width() <= super::super::MIN_WIDTH
                });
                drawer.show(entry("second.mp4"), None);
                let second = requests.borrow()[1].media_size;
                assert_eq!(
                    second,
                    MediaPreviewSize::for_viewport(
                        split.width() - split.position(),
                        drawer.state.content.height(),
                        drawer.state.pane.scale_factor()
                    )
                );
                assert!(second.width < first.width);
                windows.push((window, drawer));
            }
            for (window, drawer) in windows {
                drawer.close();
                window.close();
            }
        },
    );
}
