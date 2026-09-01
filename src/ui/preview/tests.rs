// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    PDF_MAX_ZOOM, PDF_MIN_ZOOM, format_file_size, pdf_zoom_after_scroll,
    preview_width_for_empty_space,
};

#[test]
fn formats_preview_file_sizes() {
    assert_eq!(format_file_size(999), "999 B");
    assert_eq!(format_file_size(1_200), "1.2 kB");
    assert_eq!(format_file_size(2_500_000), "2.5 MB");
}

#[test]
fn initial_preview_uses_empty_space_or_a_responsive_floor() {
    assert_eq!(preview_width_for_empty_space(2_000, 500), 1_350);
    assert_eq!(preview_width_for_empty_space(2_000, 2_000), 666);
    assert_eq!(preview_width_for_empty_space(700, 650), 520);
}

#[test]
fn pdf_scroll_zoom_stays_within_its_supported_range() {
    assert!(pdf_zoom_after_scroll(1.0, -1.0) > 1.0);
    assert!(pdf_zoom_after_scroll(2.0, 1.0) < 2.0);
    assert_eq!(pdf_zoom_after_scroll(PDF_MIN_ZOOM, 100.0), PDF_MIN_ZOOM);
    assert_eq!(pdf_zoom_after_scroll(PDF_MAX_ZOOM, -100.0), PDF_MAX_ZOOM);
}
