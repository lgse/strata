// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn deleted_pins_and_descendants_are_removed_without_rewriting_other_entries() {
    let contents = b"file:///fixture/gone Gone\r\n\
        file:///fixture/gone/child Child\n\
        file:///fixture/gone%20too Keep\n\
        file:///fixture/gone-sibling Sibling\n\
        file:///disconnected/drive Offline\n\
        smb://unavailable/share Network\n\
        file:///fixture/keep Raw\xff\r\n\
        \xffinvalid\n\n\
        file:///fixture/final No newline";
    let expected = b"file:///fixture/gone%20too Keep\n\
        file:///fixture/gone-sibling Sibling\n\
        file:///disconnected/drive Offline\n\
        smb://unavailable/share Network\n\
        file:///fixture/keep Raw\xff\r\n\
        \xffinvalid\n\n\
        file:///fixture/final No newline";
    let deleted = [gio::File::for_path("/fixture/gone")];
    assert_eq!(retain_unrelated_bookmarks(contents, &deleted), expected);
}

#[test]
fn cleanup_reads_current_bookmarks_and_leaves_missing_files_absent() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("bookmarks");
    let file = gio::File::for_path(&path);
    let deleted = [Location::local("/fixture/gone")];
    remove_deleted_pins(&file, &deleted).expect("missing bookmarks");
    assert!(!path.exists());
    std::fs::write(
        &path,
        b"file:///fixture/gone Gone\nfile:///fixture/added External\xff\n",
    )
    .expect("external edit");
    remove_deleted_pins(&file, &deleted).expect("cleanup");
    assert_eq!(
        std::fs::read(&path).expect("saved bookmarks"),
        b"file:///fixture/added External\xff\n"
    );
}

#[test]
fn concurrent_edits_are_reread_after_an_etag_conflict() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("bookmarks");
    let file = gio::File::for_path(&path);
    std::fs::write(&path, b"file:///fixture/gone Gone\n").expect("initial pins");
    let mut collided = false;
    remove_deleted_pins_with(
        &file,
        &[Location::local("/fixture/gone")],
        |contents, etag| {
            if !collided {
                collided = true;
                std::fs::write(
                    &path,
                    b"file:///fixture/gone Gone\nfile:///external New\xff\r\n",
                )
                .expect("concurrent edit");
                return Err(glib::Error::new(
                    gio::IOErrorEnum::WrongEtag,
                    "concurrent edit",
                ));
            }
            assert_eq!(contents, b"file:///external New\xff\r\n");
            file.replace_contents(
                contents,
                etag,
                false,
                gio::FileCreateFlags::NONE,
                gio::Cancellable::NONE,
            )
            .map(|_| ())
        },
    )
    .expect("retry cleanup");
    assert_eq!(
        std::fs::read(path).expect("saved pins"),
        b"file:///external New\xff\r\n"
    );
}

#[test]
fn cleanup_stops_on_repeated_conflicts_and_other_write_errors() {
    for code in [
        gio::IOErrorEnum::WrongEtag,
        gio::IOErrorEnum::PermissionDenied,
    ] {
        let directory = tempfile::tempdir().expect("fixture");
        let path = directory.path().join("bookmarks");
        let original = b"file:///fixture/gone Gone\nfile:///external Keep\n";
        std::fs::write(&path, original).expect("pins");
        let mut attempts = 0;
        let result = remove_deleted_pins_with(
            &gio::File::for_path(&path),
            &[Location::local("/fixture/gone")],
            |_, _| {
                attempts += 1;
                assert!(attempts < 10, "cleanup retries must be bounded");
                Err(glib::Error::new(code, "write failed"))
            },
        );
        assert!(result.expect_err("write failure").matches(code));
        if code == gio::IOErrorEnum::PermissionDenied {
            assert_eq!(attempts, 1);
        }
    }
}

#[test]
fn deleting_symlink_paths_preserves_pins_to_their_targets() {
    let directory = tempfile::tempdir().expect("fixture");
    let target = directory.path().join("outside");
    let link = directory.path().join("link");
    std::fs::create_dir(&target).expect("target");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let pin = |path: &std::path::Path| format!("{} Pin\n", gio::File::for_path(path).uri());
    let contents = format!("{}{}{}", pin(&link), pin(&link.join("child")), pin(&target));
    assert_eq!(
        retain_unrelated_bookmarks(contents.as_bytes(), &[gio::File::for_path(&link)]),
        pin(&target).as_bytes()
    );
}

#[test]
fn non_utf8_native_paths_match_their_percent_encoded_pins() {
    use std::os::unix::ffi::OsStrExt;

    let directory = tempfile::tempdir().expect("fixture");
    let target = directory.path().join("keep");
    let pin = |path: &std::path::Path| format!("{} Pin\n", gio::File::for_path(path).uri());
    let raw = Location::local(
        directory
            .path()
            .join(std::ffi::OsStr::from_bytes(b"gone\xff")),
    );
    let contents = format!(
        "{} Pin\n{}",
        gio_file_for_location(&raw).uri(),
        pin(&target)
    );
    assert_eq!(
        retain_unrelated_bookmarks(contents.as_bytes(), &[gio_file_for_location(&raw)]),
        pin(&target).as_bytes()
    );
}
