// SPDX-License-Identifier: MIT

use super::{
    MediaPreviewSize, PreviewContent, SandboxedMedia, content_family, has_plain_text_extension,
    is_extensionless_dotfile, is_non_executable_extensionless_dotfile,
};

#[test]
fn media_viewport_sizes_follow_display_scale_without_exceeding_the_pixel_budget() {
    assert_eq!(
        MediaPreviewSize::for_viewport(520, 800, 1),
        MediaPreviewSize::new(520, 800)
    );
    assert_eq!(
        MediaPreviewSize::for_viewport(520, 800, 2),
        MediaPreviewSize::new(1040, 1280)
    );
    assert_eq!(
        MediaPreviewSize::for_viewport(i32::MAX, i32::MAX, 2),
        MediaPreviewSize::new(1280, 1280)
    );
}

#[test]
fn normalized_media_is_private_shared_without_copying_and_removed_after_the_last_owner() {
    use std::os::unix::fs::PermissionsExt;

    let media = SandboxedMedia::from_normalized(b"normalized clip").expect("private media");
    let path = media.path().to_path_buf();
    assert_eq!(
        std::fs::read(&path).expect("media bytes"),
        b"normalized clip"
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("private file")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(media.byte_len(), 15);
    let player = media.clone();
    assert_eq!(player.path(), media.path());
    drop(media);
    assert!(path.is_file());
    drop(player);
    assert!(!path.exists());
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
