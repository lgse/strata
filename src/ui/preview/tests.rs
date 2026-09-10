// SPDX-License-Identifier: MIT

mod preferences;

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use super::{
    MEDIA_PLUGIN_INSTALL_COMMAND, PDF_MAX_ZOOM, PDF_MIN_ZOOM, PreviewDrawer, format_file_size,
    format_media_time, media_error_feedback, pdf_zoom_after_scroll, preview_drag_entries,
    preview_width_for_empty_space, print_fit, print_page_starts, print_progress_for_page,
};
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
        PreviewRequestId,
    },
};

struct UnusedPreviewProvider;

impl PreviewProvider for UnusedPreviewProvider {
    fn load(&self, _request: PreviewRequest, _emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        panic!("media teardown test does not load previews")
    }
}

struct WeakMediaWidgets {
    overlay: glib::WeakRef<gtk::Overlay>,
    picture: glib::WeakRef<gtk::Picture>,
    media: glib::WeakRef<gtk::MediaFile>,
}

fn generated_media_data(content_type: &str) -> Vec<u8> {
    if content_type == "image/gif" {
        b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;".to_vec()
    } else {
        let mut data = 24_u32.to_be_bytes().to_vec();
        data.extend_from_slice(b"ftypisom\0\0\x02\0isomiso2");
        data
    }
}

fn render_media_widgets(drawer: &PreviewDrawer, content_type: &str) -> WeakMediaWidgets {
    let filename = if content_type == "image/gif" {
        "generated.gif"
    } else {
        "generated.mp4"
    };
    let entry = FileEntry {
        location: Location::local(std::path::PathBuf::from(filename)),
        native_name: filename.into(),
        thumbnail_path: None,
        display_name: filename.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Known(0),
        modified_unix_seconds: MetadataValue::Known(0),
        mode: MetadataValue::Unknown,
        is_hidden: false,
    };
    drawer.state.render(Preview {
        request_id: PreviewRequestId(1),
        entry,
        content_type: content_type.to_owned(),
        content: PreviewContent::SandboxedMedia {
            data: generated_media_data(content_type),
        },
    });

    let overlay = drawer
        .state
        .content
        .first_child()
        .and_downcast::<gtk::Overlay>()
        .expect("production media overlay");
    let picture = overlay
        .child()
        .and_downcast::<gtk::Picture>()
        .expect("production media picture");
    let media = drawer
        .state
        .media
        .borrow()
        .as_ref()
        .expect("production media stream")
        .clone()
        .downcast::<gtk::MediaFile>()
        .expect("media file");

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
fn print_progress_reports_completed_pages() {
    assert_eq!(
        print_progress_for_page(3, 8),
        ("Rendering page 3 of 8".to_owned(), 0.375)
    );
}

#[test]
fn print_progress_clamps_invalid_counts() {
    assert_eq!(
        print_progress_for_page(3, 0),
        ("Rendering page 1 of 1".to_owned(), 1.0)
    );
}

#[test]
fn formats_preview_file_sizes() {
    assert_eq!(format_file_size(999), "999 B");
    assert_eq!(format_file_size(1_200), "1.2 kB");
    assert_eq!(format_file_size(2_500_000), "2.5 MB");
}

#[test]
fn preview_file_sizes_round_before_choosing_the_unit() {
    assert_eq!(format_file_size(999_950), "1.0 MB");
    assert_eq!(format_file_size(999_950_000), "1.0 GB");
    assert_eq!(format_file_size(9_960), "10 kB");
    assert_eq!(format_file_size(10_000), "10 kB");
}

#[test]
fn preview_file_sizes_keep_bytes_whole_and_promote_displayed_overflow() {
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
    assert_eq!(
        command,
        Some("sudo pacman -S --needed gst-plugins-good gst-libav")
    );

    let (title, detail, command) = media_error_feedback("The media data is corrupt");
    assert_eq!(title, "Preview unavailable");
    assert!(detail.contains("The media data is corrupt"));
    assert_eq!(command, None);
}

#[test]
fn initial_preview_uses_most_of_the_unoccupied_width() {
    assert_eq!(preview_width_for_empty_space(2_000, 500), 1_350);
    assert_eq!(preview_width_for_empty_space(700, 650), 280);
}

#[test]
fn pdf_scroll_zoom_stays_within_its_supported_range() {
    assert!(pdf_zoom_after_scroll(1.0, -1.0) > 1.0);
    assert!(pdf_zoom_after_scroll(2.0, 1.0) < 2.0);
    assert_eq!(pdf_zoom_after_scroll(PDF_MIN_ZOOM, 100.0), PDF_MIN_ZOOM);
    assert_eq!(pdf_zoom_after_scroll(PDF_MAX_ZOOM, -100.0), PDF_MAX_ZOOM);
}

#[test]
fn clear_content_clears_media_file_input_stream() {
    const TEST: &str = "ui::preview::tests::clear_content_clears_media_file_input_stream";
    crate::test_support::gtk_test(TEST, || {
        let bytes = glib::Bytes::from_static(b"media fixture");
        let input = gio::MemoryInputStream::from_bytes(&bytes);
        let media = gtk::MediaFile::for_input_stream(&input);
        let drawer = PreviewDrawer::new(Rc::new(UnusedPreviewProvider), false);
        drawer
            .state
            .media
            .replace(Some(media.clone().upcast::<gtk::MediaStream>()));

        assert!(media.input_stream().is_some());
        drawer.state.clear_content();

        assert!(drawer.state.media.borrow().is_none());
        assert!(
            media.input_stream().is_none(),
            "clearing preview content must detach the media source"
        );
    });
}

#[test]
fn closing_media_preview_finalizes_production_widget_tree() {
    const TEST: &str = "ui::preview::tests::closing_media_preview_finalizes_production_widget_tree";
    crate::test_support::gtk_test(TEST, || {
        let drawer = PreviewDrawer::new(Rc::new(UnusedPreviewProvider), false);
        let widgets = render_media_widgets(&drawer, "image/gif");

        drawer.close();

        assert_media_widgets_finalized(&widgets);
    });
}

#[test]
fn replacing_repeated_media_previews_finalizes_previous_widget_trees() {
    const TEST: &str =
        "ui::preview::tests::replacing_repeated_media_previews_finalizes_previous_widget_trees";
    crate::test_support::gtk_test(TEST, || {
        let drawer = PreviewDrawer::new(Rc::new(UnusedPreviewProvider), false);
        let mut current = render_media_widgets(&drawer, "image/gif");

        for content_type in ["video/mp4", "image/gif", "video/mp4", "image/gif"] {
            let next = render_media_widgets(&drawer, content_type);
            assert_media_widgets_finalized(&current);
            current = next;
        }

        drawer.close();
        assert_media_widgets_finalized(&current);
    });
}

#[test]
fn media_time_formats_minutes_and_seconds() {
    assert_eq!(format_media_time(0, 0), "0:00/0:00");
    assert_eq!(format_media_time(1_500_000, 65_000_000), "0:01/1:05");
    assert_eq!(format_media_time(125_000_000, 125_000_000), "2:05/2:05");
}

#[test]
fn media_time_clamps_negative_timestamps_to_zero() {
    assert_eq!(format_media_time(-500_000, 10_000_000), "0:00/0:10");
}

#[test]
fn preview_drag_entries_returns_none_when_no_entry_loaded() {
    assert_eq!(preview_drag_entries(None), None);
}

#[test]
fn preview_drag_entries_wraps_loaded_file_entry() {
    let entry = crate::model::FileEntry {
        location: crate::model::Location::local("/tmp/test.png"),
        native_name: std::ffi::OsString::from("test.png"),
        thumbnail_path: None,
        display_name: "test.png".to_owned(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Known(100),
        modified_unix_seconds: crate::model::MetadataValue::Known(1),
        mode: crate::model::MetadataValue::Unknown,
        is_hidden: false,
    };
    let dragged = preview_drag_entries(Some(&entry));
    assert_eq!(dragged, Some(vec![entry]));
}
