// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn trash_child_diagnostics_must_point_inside_the_requested_folder() {
    let root = Path::new("/fixture/to-delete");
    let format = glib::dgettext(Some("glib20"), "Unable to trash child file %s");
    for (path, expected) in [
        (
            "/fixture/to-delete/nested/file: name.txt",
            Some("nested/file: name.txt"),
        ),
        ("/fixture/to-delete", None),
        ("/fixture/to-delete-other/file.txt", None),
        ("/fixture/to-delete/nested/../../elsewhere", None),
        ("nested/file.txt", None),
    ] {
        let message = format.replace("%1$s", path).replace("%s", path);
        let error = glib::Error::new(gio::IOErrorEnum::PermissionDenied, &message);
        assert_eq!(
            reported_trash_child(root, &error),
            expected.map(PathBuf::from),
            "{path}"
        );
    }
    let generic = glib::Error::new(
        gio::IOErrorEnum::PermissionDenied,
        "Unable to trash file /fixture/to-delete/nested/file.txt: Permission denied",
    );
    assert_eq!(reported_trash_child(root, &generic), None);
}

#[test]
fn gio_path_extraction_handles_reordered_placeholders_and_native_bytes() {
    assert_eq!(
        interpolate_path(
            b"'/fixture/nested/\xff.dat' could not be moved to Trash",
            b"'%1$s' could not be moved to Trash"
        ),
        Some(b"/fixture/nested/\xff.dat".as_slice()),
    );
}

#[test]
fn reveal_stops_if_a_parent_folder_is_replaced_by_a_symlink() -> std::io::Result<()> {
    let fixture = tempfile::tempdir()?;
    let root = fixture.path().join("to-delete");
    let parent = root.join("nested");
    std::fs::create_dir_all(&parent)?;
    let item = parent.join("blocked.txt");
    std::fs::write(&item, b"blocker")?;
    let blocker = DeleteBlocker {
        root,
        child: PathBuf::from("nested/blocked.txt"),
    };
    assert_eq!(
        blocker.resolve(),
        Some((Location::local(&parent), Location::local(&item)))
    );

    let moved = fixture.path().join("outside");
    std::fs::rename(&parent, &moved)?;
    std::os::unix::fs::symlink(&moved, &parent)?;
    assert!(
        item.exists(),
        "ordinary path lookup would follow the replacement"
    );
    assert_eq!(blocker.resolve(), None);
    Ok(())
}
