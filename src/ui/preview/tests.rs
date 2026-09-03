// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    MEDIA_PLUGIN_INSTALL_COMMAND, PDF_MAX_ZOOM, PDF_MIN_ZOOM, format_file_size, format_media_time,
    media_error_feedback, pdf_zoom_after_scroll, preview_width_for_empty_space,
};

#[test]
fn formats_preview_file_sizes() {
    assert_eq!(format_file_size(999), "999 B");
    assert_eq!(format_file_size(1_200), "1.2 kB");
    assert_eq!(format_file_size(2_500_000), "2.5 MB");
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
fn media_time_formats_minutes_and_seconds() {
    assert_eq!(format_media_time(0, 0), "0:00/0:00");
    assert_eq!(format_media_time(1_500_000, 65_000_000), "0:01/1:05");
    assert_eq!(format_media_time(125_000_000, 125_000_000), "2:05/2:05");
}

#[test]
fn media_time_clamps_negative_timestamps_to_zero() {
    assert_eq!(format_media_time(-500_000, 10_000_000), "0:00/0:10");
}
