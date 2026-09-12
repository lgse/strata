// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
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

pub(in crate::ui::preview) struct RecordingProvider(
    pub(in crate::ui::preview) Rc<RefCell<Vec<PreviewRequest>>>,
);

impl PreviewProvider for RecordingProvider {
    fn load(&self, request: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.0.borrow_mut().push(request);
        LoadHandle::new(|| {})
    }
}

pub(in crate::ui::preview) fn entry(name: &str) -> FileEntry {
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

#[track_caller]
pub(in crate::ui::preview) fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "preview layout did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn image(width: i32, height: i32) -> Vec<u8> {
    gtk::gdk::MemoryTexture::new(
        width,
        height,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(vec![255u8; width as usize * height as usize * 4]),
        width as usize * 4,
    )
    .save_to_png_bytes()
    .to_vec()
}

#[test]
fn images_stay_centered_and_bounded_while_text_and_pdf_use_the_full_pane() {
    crate::test_support::gtk_test(
        "ui::preview::tests::media_size::images_stay_centered_and_bounded_while_text_and_pdf_use_the_full_pane",
        || {
            let preferences = ThemeManager::shared();
            let drawer = PreviewDrawer::new(
                Rc::new(RecordingProvider(Rc::new(RefCell::new(Vec::new())))),
                true,
            );
            let window = gtk::Window::builder()
                .default_width(1800)
                .default_height(1000)
                .child(&drawer.widget())
                .build();
            drawer.state.revealer.set_reveal_child(true);
            drawer.state.opened.set(true);
            window.set_width_request(1800);
            window.present();
            wait_until(|| drawer.state.content.width() > 1280);
            for (width, height) in [(160, 90), (3000, 1200), (90, 180)] {
                drawer.state.render(Preview {
                    request_id: PreviewRequestId(1),
                    entry: entry("image.png"),
                    content_type: "image/png".into(),
                    content: PreviewContent::Rasterized {
                        png: image(width, height),
                    },
                });
                let section = drawer.state.content.first_child().expect("media section");
                let picture = section.first_child().expect("image");
                for text_size in [
                    crate::ui::theme::TextSize::Small,
                    crate::ui::theme::TextSize::Large,
                ] {
                    preferences.set_text_size(text_size);
                    let allocated = Rc::new(Cell::new(false));
                    let ready = allocated.clone();
                    picture.add_tick_callback(move |_, _| {
                        ready.set(true);
                        glib::ControlFlow::Break
                    });
                    wait_until(|| allocated.get() && picture.width() > 0);
                    let bounds = picture
                        .compute_bounds(&drawer.state.content)
                        .expect("image bounds");
                    assert!(bounds.width() <= 1280.0);
                    assert!(bounds.width() <= (width * 2) as f32);
                    assert!(bounds.height() <= (height * 2) as f32);
                    assert!(
                        (bounds.width() / bounds.height() - width as f32 / height as f32).abs()
                            < 0.02
                    );
                    assert!(
                        (bounds.x() + bounds.width() / 2.0
                            - drawer.state.content.width() as f32 / 2.0)
                            .abs()
                            <= 1.0
                    );
                }
            }
            for content in [
                PreviewContent::Text {
                    content: "wide text".into(),
                    truncated: false,
                },
                PreviewContent::Pdf {
                    png: image(300, 400),
                    page: 0,
                    pages: 1,
                },
            ] {
                drawer.state.render(Preview {
                    request_id: PreviewRequestId(1),
                    entry: entry("document.pdf"),
                    content_type: "application/pdf".into(),
                    content,
                });
                let scroll = drawer
                    .state
                    .content
                    .first_child()
                    .expect("document scroller");
                wait_until(|| scroll.width() > 1280);
                assert_eq!(scroll.width(), drawer.state.content.width());
            }
            drawer.close();
            window.destroy();
        },
    );
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
                window.set_width_request(1800);
                window.present();
                wait_until(|| {
                    assert!(media.error().is_none(), "{:?}", media.error());
                    media.is_prepared() && media.timestamp() > 0
                });
                assert!(media.has_video());
                assert!(media.has_audio());
                assert!(media.downcast_ref::<gtk::MediaFile>().is_none());
                let section = drawer.state.content.first_child().expect("media section");
                let video = section.first_child().expect("video overlay");
                let controls = section.last_child().expect("playback controls");
                wait_until(|| video.width() > 0 && controls.width() > 0);
                assert!(video.width() <= decoded.intrinsic_width() * 2);
                assert!(video.height() <= decoded.intrinsic_height() * 2);
                assert!(controls.width() <= 1280);
                let play = controls
                    .first_child()
                    .expect("play button")
                    .downcast::<gtk::Button>()
                    .expect("playback button");
                play.emit_clicked();
                assert!(!media.is_playing());
                play.emit_clicked();
                assert!(media.is_playing());
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
                let browser = crate::ui::browser::BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    crate::ui::browser::PeekBehavior::default(),
                );
                let content = gtk::Paned::new(gtk::Orientation::Horizontal);
                content.set_end_child(Some(&browser.widget()));
                let split = gtk::Paned::new(gtk::Orientation::Horizontal);
                split.set_start_child(Some(&content));
                drawer.attach_split(&split, &content, &browser);
                let window = gtk::Window::builder()
                    .default_width(window_width)
                    .default_height(800)
                    .child(&split)
                    .build();
                window.present();
                wait_until(|| split.width() > 0 && split.height() > 0);
                let opening_width = drawer.state.opening_width(split.width());
                drawer.show(entry("first.mp4"));
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
                drawer.state.resize_preview(&split, split.width() - 650);
                wait_until(|| {
                    drawer.state.content.width() > 0 && drawer.state.content.width() <= 650
                });
                drawer.show(entry("second.mp4"));
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
