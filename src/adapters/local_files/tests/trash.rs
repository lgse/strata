// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn image_info(target: Option<&str>) -> gio::FileInfo {
    let info = gio::FileInfo::new();
    info.set_name("photo.png.2");
    info.set_display_name("photo.png");
    info.set_file_type(gio::FileType::Regular);
    info.set_size(42);
    info.set_attribute_uint64(gio::FILE_ATTRIBUTE_TIME_MODIFIED, 123);
    if let Some(target) = target {
        info.set_attribute_string(gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI, target);
    }
    info
}

#[test]
fn trash_keeps_its_identity_and_local_thumbnail_source_separate() {
    let path = PathBuf::from("/fixture/Trash/files/photo #1.png.2");
    let target = gio::File::for_path(&path).uri();
    let location = Location::uri("trash:///photo%20%231.png.2");
    let entry = entry_from_info(location.clone(), image_info(Some(&target)));
    assert_eq!(entry.location, location);
    assert!(entry.location.native_path().is_none());
    assert_eq!(entry.local_thumbnail_path(), Some(path.as_path()));
    assert_eq!(entry.display_name, "photo.png");
    assert_eq!(entry.kind, EntryKind::File);
    assert_eq!(entry.size, MetadataValue::Known(42));
    assert_eq!(entry.modified_unix_seconds, MetadataValue::Known(123));
    assert!(
        FULL_ATTRIBUTES
            .split(',')
            .any(|attribute| attribute == gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI)
    );
}

#[test]
fn trash_thumbnail_sources_preserve_native_filename_bytes() {
    let path = PathBuf::from(OsString::from_vec(
        b"/fixture/Trash/files/photo-\xff.png".to_vec(),
    ));
    let target = gio::File::for_path(&path).uri();
    let entry = entry_from_info(Location::uri("trash:///photo"), image_info(Some(&target)));
    assert_eq!(entry.local_thumbnail_path(), Some(path.as_path()));
}

#[test]
fn missing_or_non_native_trash_targets_have_no_thumbnail_source() {
    for target in [
        None,
        Some("https://example.test/photo.png"),
        Some("smb://host/share/photo.png"),
        Some("trash:///photo.png"),
        Some("not-a-uri"),
        Some("file://host/photo.png"),
    ] {
        let entry = entry_from_info(Location::uri("trash:///photo.png"), image_info(target));
        assert_eq!(entry.local_thumbnail_path(), None, "target: {target:?}");
    }
}

#[test]
fn non_trash_locations_do_not_follow_thumbnail_targets() {
    let info = image_info(Some("file:///fixture/target.png"));
    for location in [
        Location::uri("smb://host/share/photo.png"),
        Location::uri("recent:///photo.png"),
    ] {
        let entry = entry_from_info(location, info.clone());
        assert_eq!(entry.local_thumbnail_path(), None);
    }
    let entry = entry_from_info(Location::local("/fixture/photo.png"), info);
    assert_eq!(
        entry.local_thumbnail_path(),
        Some(Path::new("/fixture/photo.png"))
    );
    assert_eq!(entry.thumbnail_path, None);
}
