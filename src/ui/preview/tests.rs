// SPDX-License-Identifier: MIT

mod media_size;
mod preferences;

use std::rc::Rc;

use gtk::{glib, prelude::*};

use super::{
    PDF_MAX_ZOOM, PDF_MIN_ZOOM, PreviewDrawer, format_file_size, format_media_time,
    media_error_feedback, pdf_zoom_after_scroll, preview_drag_entries, preview_target,
    preview_width_for_empty_space, print_fit, print_page_starts, print_progress_for_page,
};
use crate::app::{Browser, BrowserEvent, EntrySplice};
use crate::model::Location;
use crate::services::{LoadHandle, PreviewEvent, PreviewProvider, PreviewRequest};
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
    drawer.state.content.append(&overlay);
    drawer.state.append_media_controls(
        media.upcast_ref(),
        &ThemeManager::shared(),
        &overlay.clone().upcast(),
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
fn media_errors_preserve_capability_guidance_without_guessing_package_commands() {
    let message =
        "Media helper runtime libraries are missing. Install GStreamer core/base, then retry.";
    let (title, detail, command) = media_error_feedback(message);
    assert_eq!(title, "Preview unavailable");
    assert!(detail.contains(message));
    assert_eq!(command, None);

    let (title, detail, command) = media_error_feedback("The media data is corrupt");
    assert_eq!(title, "Preview unavailable");
    assert!(detail.contains("The media data is corrupt"));
    assert_eq!(command, None);
}

#[test]
fn initial_preview_uses_most_of_the_unoccupied_width() {
    assert_eq!(preview_width_for_empty_space(2_000, 500), 1_350);
    assert_eq!(preview_width_for_empty_space(700, 650), 560);
    assert_eq!(preview_width_for_empty_space(500, 500), 560);
}

#[test]
fn pdf_scroll_zoom_stays_within_its_supported_range() {
    assert!(pdf_zoom_after_scroll(1.0, -1.0) > 1.0);
    assert!(pdf_zoom_after_scroll(2.0, 1.0) < 2.0);
    assert_eq!(pdf_zoom_after_scroll(PDF_MIN_ZOOM, 100.0), PDF_MIN_ZOOM);
    assert_eq!(pdf_zoom_after_scroll(PDF_MAX_ZOOM, -100.0), PDF_MAX_ZOOM);
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
        is_hidden: false,
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
