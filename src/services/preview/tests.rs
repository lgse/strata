// SPDX-License-Identifier: GPL-3.0-or-later

use super::{PreviewContent, content_family, has_plain_text_extension, normalize_preview_text};

#[test]
fn preview_text_normalizes_nul_before_any_gtk_view() {
    assert_eq!(normalize_preview_text("before\0after"), "before�after");
    assert!(matches!(
        normalize_preview_text("ordinary text"),
        std::borrow::Cow::Borrowed(_)
    ));
}

#[test]
fn recognizes_configuration_files_as_plain_text() {
    assert!(has_plain_text_extension(std::ffi::OsStr::new(
        "settings.conf"
    )));
    assert!(has_plain_text_extension(std::ffi::OsStr::new(
        "SETTINGS.INI"
    )));
    assert!(!has_plain_text_extension(std::ffi::OsStr::new(
        "archive.zip"
    )));
}

#[test]
fn classifies_common_preview_content_types() {
    assert_eq!(content_family("image/png"), PreviewContent::Image);
    assert_eq!(content_family("image/gif"), PreviewContent::Media);
    assert_eq!(content_family("video/mp4"), PreviewContent::Media);
    assert!(matches!(
        content_family("application/pdf"),
        PreviewContent::Pdf { .. }
    ));
    assert!(matches!(
        content_family("text/x-rust"),
        PreviewContent::Text { .. }
    ));
    assert!(matches!(
        content_family("application/problem+json"),
        PreviewContent::Text { .. }
    ));
    assert_eq!(
        content_family("application/octet-stream"),
        PreviewContent::Unsupported
    );
}
