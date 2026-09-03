// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    PreviewContent, content_family, has_plain_text_extension, is_extensionless_dotfile,
    is_non_executable_extensionless_dotfile,
};

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
fn recognizes_extensionless_dotfiles() {
    assert!(is_extensionless_dotfile(std::ffi::OsStr::new(".steampath")));
    assert!(!is_extensionless_dotfile(std::ffi::OsStr::new("steampath")));
    assert!(!is_extensionless_dotfile(std::ffi::OsStr::new(
        ".settings.toml"
    )));
}

#[test]
fn recognizes_non_executable_extensionless_dotfiles() {
    let name = std::ffi::OsStr::new(".steamid");

    assert!(is_non_executable_extensionless_dotfile(
        name,
        Some(0o100644)
    ));
    assert!(!is_non_executable_extensionless_dotfile(
        name,
        Some(0o100755)
    ));
    assert!(!is_non_executable_extensionless_dotfile(name, None));
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
