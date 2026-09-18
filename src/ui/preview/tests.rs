// SPDX-License-Identifier: MIT

pub(super) mod media_size;
mod preferences;
mod source;

use std::rc::Rc;

use gtk::{glib, prelude::*};

use super::{
    MEDIA_PLUGIN_INSTALL_COMMAND, PreviewDrawer, accepts_preview_event, format_file_size,
    format_media_time, media_error_feedback, pdf_zoom_after_scroll, preview_drag_entries,
    preview_target, print_fit, print_page_starts, print_progress_for_page, source_chunk_end,
};
use crate::app::{Browser, BrowserEvent, EntrySplice};
use crate::model::Location;
use crate::services::{
    LoadHandle, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
};
use crate::ui::theme::ThemeManager;

struct UnusedPreviewProvider;

impl PreviewProvider for UnusedPreviewProvider {
    fn load(&self, _request: PreviewRequest, _emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        panic!("media teardown test does not load previews")
    }
}

struct NoopPreviewProvider;

impl PreviewProvider for NoopPreviewProvider {
    fn load(&self, _request: PreviewRequest, _emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        LoadHandle::new(|| {})
    }
}

struct WeakMediaWidgets {
    overlay: glib::WeakRef<gtk::Overlay>,
    picture: glib::WeakRef<gtk::Picture>,
    media: glib::WeakRef<crate::ui::media::DecodedMedia>,
}

fn render_media_widgets(drawer: &PreviewDrawer, is_gif: bool) -> WeakMediaWidgets {
    let media = crate::ui::media::tests::player(false, 1_000_000);
    drawer
        .state
        .media
        .replace(Some(media.clone().upcast::<gtk::MediaStream>()));
    let (overlay, center_play) = drawer.state.build_media_view(media.upcast_ref());
    let picture = overlay
        .child()
        .and_downcast::<gtk::Picture>()
        .expect("production media picture");
    let section = super::media_layout::section(&overlay, &media);
    drawer.state.content.append(&section);
    drawer.state.append_media_controls(
        media.upcast_ref(),
        &ThemeManager::shared(),
        &section,
        &center_play,
        is_gif,
    );

    WeakMediaWidgets {
        overlay: overlay.downgrade(),
        picture: picture.downgrade(),
        media: media.downgrade(),
    }
}

fn assert_media_hierarchy_finalized(widgets: &WeakMediaWidgets) {
    assert!(widgets.overlay.upgrade().is_none(), "overlay must finalize");
    assert!(widgets.picture.upgrade().is_none(), "picture must finalize");
}

fn assert_media_widgets_finalized(widgets: &WeakMediaWidgets) {
    assert_media_hierarchy_finalized(widgets);
    assert!(widgets.media.upgrade().is_none(), "media must finalize");
}

#[test]
fn print_fit_centers_landscape_image_on_portrait_page() {
    let (x, y, width, height, scale) = print_fit(595.0, 842.0, 1920.0, 1080.0)
        .expect("print_fit with valid dimensions should return a layout");
    assert!((scale - 0.3098).abs() < 0.001);
    assert!((width - 595.0).abs() < 0.001);
    assert!((height - 334.7).abs() < 0.5);
    assert!(x.abs() < f64::EPSILON);
    assert!((y - (842.0 - height) / 2.0).abs() < 0.5);
}

#[test]
fn print_fit_keeps_tall_image_inside_page() {
    let (x, y, width, height, scale) = print_fit(595.0, 842.0, 1000.0, 2000.0)
        .expect("print_fit with valid dimensions should return a layout");
    let page_scale = 842.0 / 2000.0;
    assert!((scale - page_scale).abs() < 0.001);
    assert!((height - 842.0).abs() < 0.001);
    assert!((width - 1000.0 * page_scale).abs() < 0.001);
    assert!((x - (595.0 - width) / 2.0).abs() < 0.5);
    assert!(y.abs() < f64::EPSILON);
    assert!(
        (y + height / 2.0 - 842.0 / 2.0).abs() < 0.5,
        "image is vertically centered"
    );
}

#[test]
fn print_fit_rejects_zero_sized_inputs() {
    assert_eq!(print_fit(0.0, 842.0, 100.0, 100.0), None);
    assert_eq!(print_fit(595.0, 0.0, 100.0, 100.0), None);
    assert_eq!(print_fit(595.0, 842.0, 0.0, 100.0), None);
    assert_eq!(print_fit(595.0, 842.0, 100.0, -1.0), None);
}

#[test]
fn text_print_pages_start_on_line_boundaries() {
    assert_eq!(
        print_page_starts(&[(0.0, 12.0), (12.0, 24.0), (24.0, 36.0)], 25.0),
        vec![0.0, 24.0]
    );
}

#[test]
fn print_progress_reports_completed_pages_and_clamps_invalid_counts() {
    assert_eq!(
        print_progress_for_page(3, 8),
        ("Rendering page 3 of 8".to_owned(), 0.375)
    );

    assert_eq!(
        print_progress_for_page(3, 0),
        ("Rendering page 1 of 1".to_owned(), 1.0)
    );
}

#[test]
fn preview_file_sizes_use_decimal_units_and_promote_rounded_overflow() {
    assert_eq!(format_file_size(999), "999 B");
    assert_eq!(format_file_size(1_200), "1.2 kB");
    assert_eq!(format_file_size(2_500_000), "2.5 MB");

    assert_eq!(format_file_size(999_950), "1.0 MB");
    assert_eq!(format_file_size(999_950_000), "1.0 GB");
    assert_eq!(format_file_size(9_960), "10 kB");
    assert_eq!(format_file_size(10_000), "10 kB");

    assert_eq!(format_file_size(0), "0 B");
    assert_eq!(format_file_size(5), "5 B");
    assert_eq!(format_file_size(999_450), "1.0 MB");
    assert_eq!(format_file_size(999_449), "999 kB");
}

#[test]
fn media_errors_explain_missing_runtime_plugins() {
    let (title, detail, command) =
        media_error_feedback("Your GStreamer installation is missing a plug-in.");
    assert_eq!(title, "Additional media support required");
    assert!(detail.contains("GStreamer plugins"));
    assert_eq!(command, Some(MEDIA_PLUGIN_INSTALL_COMMAND));

    let (title, detail, command) = media_error_feedback("The media data is corrupt");
    assert_eq!(title, "Preview unavailable");
    assert!(detail.contains("The media data is corrupt"));
    assert_eq!(command, None);
}

#[test]
fn incremental_source_chunks_keep_utf8_boundaries() {
    let text = format!("{}é", "x".repeat(super::SOURCE_INSERT_CHUNK_BYTES - 1));
    let end = source_chunk_end(&text, 0);
    assert!(text.is_char_boundary(end));
    assert_eq!(
        &text[..end],
        "x".repeat(super::SOURCE_INSERT_CHUNK_BYTES - 1)
    );
    assert_eq!(source_chunk_end(&text, end), text.len());

    let lines = "x\n".repeat(super::SOURCE_INSERT_CHUNK_LINES + 1);
    let end = source_chunk_end(&lines, 0);
    assert_eq!(
        lines[..end].lines().count(),
        super::SOURCE_INSERT_CHUNK_LINES
    );
    assert!(end < lines.len());
}

#[test]
fn stale_preview_responses_are_rejected() {
    let current = PreviewRequestId(2);
    assert!(accepts_preview_event(Some(current), current, current));
    assert!(!accepts_preview_event(
        Some(current),
        PreviewRequestId(1),
        PreviewRequestId(1)
    ));
    assert!(!accepts_preview_event(
        Some(current),
        current,
        PreviewRequestId(1)
    ));
    assert!(!accepts_preview_event(None, current, current));
}

#[test]
fn pdf_scroll_zoom_stays_within_its_supported_range() {
    assert!(pdf_zoom_after_scroll(1.0, -1.0) > 1.0);
    assert!(pdf_zoom_after_scroll(2.0, 1.0) < 2.0);
}

#[test]
fn clear_content_cancels_decoding_and_releases_the_displayed_frame() {
    const TEST: &str =
        "ui::preview::tests::clear_content_cancels_decoding_and_releases_the_displayed_frame";
    crate::test_support::gtk_test(TEST, || {
        let media = crate::ui::media::tests::player(false, 30_000_000);
        let drawer = PreviewDrawer::new(Rc::new(UnusedPreviewProvider), false);
        drawer.state.media.replace(Some(media.clone().upcast()));
        media.play();
        crate::ui::media::tests::wait(|| media.timestamp() > 0);
        drawer.state.clear_content();
        assert!(drawer.state.media.borrow().is_none());
        assert_eq!(media.intrinsic_width(), 0);
        assert!(!media.is_playing());
    });
}

#[test]
fn replacing_repeated_media_previews_finalizes_previous_widget_trees() {
    const TEST: &str =
        "ui::preview::tests::replacing_repeated_media_previews_finalizes_previous_widget_trees";
    crate::test_support::gtk_test(TEST, || {
        let drawer = PreviewDrawer::new(Rc::new(UnusedPreviewProvider), false);
        let mut current = render_media_widgets(&drawer, true);

        for is_gif in [false, true, false, true] {
            drawer.state.clear_content();
            let next = render_media_widgets(&drawer, is_gif);
            assert_media_widgets_finalized(&current);
            current = next;
        }

        drawer.close();
        assert_media_widgets_finalized(&current);
    });
}

#[test]
fn media_time_formats_minutes_and_seconds_and_clamps_negative_timestamps() {
    assert_eq!(format_media_time(0, 0), "0:00/0:00");
    assert_eq!(format_media_time(1_500_000, 65_000_000), "0:01/1:05");
    assert_eq!(format_media_time(125_000_000, 125_000_000), "2:05/2:05");

    assert_eq!(format_media_time(-500_000, 10_000_000), "0:00/0:10");
}

#[test]
fn remote_images_and_supported_video_are_quick_preview_targets() {
    use crate::model::{EntryKind, FileEntry, Location, MetadataValue};
    for (name, supported) in [
        ("photo.jpg", true),
        ("photo.heic", true),
        ("document.pdf", false),
        ("video.mp4", true),
        ("video.MOV", true),
        ("video.mkv", false),
        ("audio.mp3", false),
        ("animated.gif", false),
    ] {
        let entry = FileEntry {
            location: Location::uri(format!("gphoto2://device/{name}")),
            native_name: name.into(),
            thumbnail_path: None,
            display_name: name.into(),
            kind: EntryKind::File,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            recent_unix_seconds: crate::model::MetadataValue::Unknown,
            is_hidden: false,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
        };
        assert_eq!(
            preview_target(Some(entry.clone())).is_some(),
            supported,
            "{name}"
        );
        let directory = FileEntry {
            kind: EntryKind::Directory,
            ..entry
        };
        assert!(preview_target(Some(directory)).is_none());
    }
}

#[test]
fn preview_drag_entries_contains_only_the_loaded_entry() {
    assert_eq!(preview_drag_entries(None), None);
    let entry = crate::model::FileEntry {
        location: crate::model::Location::local("/tmp/test.png"),
        native_name: std::ffi::OsString::from("test.png"),
        thumbnail_path: None,
        display_name: "test.png".to_owned(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Known(100),
        modified_unix_seconds: crate::model::MetadataValue::Known(1),
        mode: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    };
    let dragged = preview_drag_entries(Some(&entry));
    assert_eq!(dragged, Some(vec![entry]));
}

#[test]
fn keyboard_opened_preview_closes_when_the_displayed_entry_is_spliced_out() {
    const TEST: &str = "ui::preview::tests::keyboard_opened_preview_closes_when_the_displayed_entry_is_spliced_out";
    crate::test_support::gtk_test(TEST, || {
        let fixture = tempfile::tempdir().expect("fixture");
        std::fs::write(fixture.path().join("photo.png"), "data").expect("file");
        let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
        browser.navigate(Location::local(fixture.path()));
        crate::ui::media::tests::wait(|| browser.column_snapshot(0).is_some_and(|s| !s.loading));
        let entry = browser.entry_at(0, 0).expect("loaded entry");
        let preview = PreviewDrawer::new(Rc::new(NoopPreviewProvider), false);
        preview.toggle(preview_target(Some(entry.clone())), browser.active_depth());
        assert!(preview.is_open());
        std::fs::remove_file(fixture.path().join("photo.png")).expect("remove file");
        crate::ui::media::tests::wait(|| browser.entry_at(0, 0).is_none());
        preview.handle_browser_event(
            &browser,
            &BrowserEvent::EntriesSpliced {
                depth: 0,
                splices: vec![EntrySplice {
                    position: 0,
                    removed: 1,
                    entries: vec![],
                }],
            },
        );
        assert!(!preview.is_open());
    });
}
