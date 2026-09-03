// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    DocumentView, FOCUS_PREVIEW_DELAY, MEDIA_PLUGIN_INSTALL_COMMAND, PDF_MAX_ZOOM, PDF_MIN_ZOOM,
    accepts_preview_event, document_view_action, format_file_size, initial_document_view,
    media_error_feedback, pdf_zoom_after_scroll, preview_width_for_empty_space, source_chunk_end,
};
use crate::services::PreviewRequestId;
use std::time::Duration;

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
fn focus_following_preview_waits_for_key_repeat_to_settle() {
    assert_eq!(FOCUS_PREVIEW_DELAY, Duration::from_millis(75));
}

#[test]
fn pdf_scroll_zoom_stays_within_its_supported_range() {
    assert!(pdf_zoom_after_scroll(1.0, -1.0) > 1.0);
    assert!(pdf_zoom_after_scroll(2.0, 1.0) < 2.0);
    assert_eq!(pdf_zoom_after_scroll(PDF_MIN_ZOOM, 100.0), PDF_MIN_ZOOM);
    assert_eq!(pdf_zoom_after_scroll(PDF_MAX_ZOOM, -100.0), PDF_MAX_ZOOM);
}

#[test]
fn each_document_uses_the_current_default_and_unavailable_rendering_forces_source() {
    assert_eq!(initial_document_view(true, true), DocumentView::Rendered);
    assert_eq!(initial_document_view(false, true), DocumentView::Source);
    assert_eq!(initial_document_view(true, false), DocumentView::Source);
}

#[test]
fn document_view_action_describes_its_destination() {
    assert_eq!(
        document_view_action(DocumentView::Rendered),
        ("View source", crate::assets::icons::FILE_CODE)
    );
    assert_eq!(
        document_view_action(DocumentView::Source),
        ("View rendered", crate::assets::icons::DOCUMENTS)
    );
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
