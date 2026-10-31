// SPDX-License-Identifier: MIT

use super::*;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";
const JPEG: &[u8] = b"\xff\xd8\xffrest";

#[test]
fn only_credential_free_web_addresses_become_links() {
    for (text, name) in [
        ("https://www.example.com/watch?v=1#t=2", Some("example.com")),
        ("  HTTP://Example.org  ", Some("Example.org")),
        ("http://[::1]:8080/", Some("::1")),
        ("https://user:secret@example.com/", None),
        ("ftp://example.com/file", None),
        ("file:///etc/passwd", None),
        ("javascript:alert(1)", None),
        ("https://example.com/a b", None),
        ("https://example.com/\nhttps://example.org/", None),
        ("https:///path-only", None),
        ("plain words", None),
    ] {
        assert_eq!(
            WebLink::parse(text).map(|link| link.name),
            name.map(str::to_owned),
            "{text:?}"
        );
    }
}

#[test]
fn saved_links_reopen_only_as_web_links() {
    let link = WebLink::parse("https://example.com/page?q=a;b").expect("web link");
    let saved = DroppedFile::link(&link);
    assert_eq!(saved.name(), "example.com.desktop");
    let entry = std::str::from_utf8(saved.bytes()).expect("UTF-8 entry");
    assert!(!entry.contains("Exec"), "{entry}");
    assert_eq!(WebLink::from_desktop_entry(entry), Some(link));

    for entry in [
        "[Desktop Entry]\nType=Application\nExec=sh -c true\nURL=https://example.com/\n",
        "[Desktop Entry]\nType=Link\nExec=sh -c true\nURL=https://example.com/\n",
        "[Desktop Entry]\nType=Link\nURL=file:///etc/passwd\n",
        "[Desktop Entry]\nType=Link\n",
        "not a key file",
    ] {
        assert_eq!(WebLink::from_desktop_entry(entry), None, "{entry}");
    }
}

#[test]
fn browser_images_keep_their_bytes_and_take_their_extension_from_them() {
    for (name, bytes, saved) in [
        (Some("photo.jpg"), JPEG, Some("photo.jpg")),
        (Some("photo.png"), JPEG, Some("photo.jpg")),
        (Some("run.desktop"), PNG, Some("run.png")),
        (Some("../../.hidden.png"), PNG, Some("hidden.png")),
        (Some("..."), PNG, Some("image.png")),
        (None, PNG, Some("image.png")),
        (Some("notes.txt"), b"plain text".as_slice(), None),
    ] {
        let file = DroppedFile::image(name, "image", bytes.to_vec());
        assert_eq!(
            file.as_ref().map(DroppedFile::name).as_deref(),
            saved,
            "{name:?}"
        );
        if let Some(file) = file {
            assert_eq!(file.bytes(), bytes);
        }
    }
}

#[test]
fn saved_files_skip_existing_names_and_dangling_symlinks() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let target = dir.path().join("missing.png");
    std::os::unix::fs::symlink(&target, dir.path().join("image.png")).expect("dangling symlink");
    std::fs::create_dir(dir.path().join("image (1).png")).expect("existing directory");
    std::fs::write(dir.path().join("image (2).png"), b"original").expect("existing image");

    let saved = save_dropped_files(
        dir.path(),
        &[
            DroppedFile::png("image", b"pasted".to_vec()),
            DroppedFile::text("Dropped Text", "line one\nline two".to_owned()),
        ],
    )
    .expect("save dropped files");

    assert_eq!(
        saved,
        [
            dir.path().join("image (3).png"),
            dir.path().join("Dropped Text.txt"),
        ]
    );
    assert_eq!(std::fs::read(&saved[0]).expect("saved image"), b"pasted");
    assert_eq!(
        std::fs::read(&saved[1]).expect("saved text"),
        b"line one\nline two"
    );
    assert!(!target.exists());
    assert_eq!(
        std::fs::read(dir.path().join("image (2).png")).expect("original image"),
        b"original"
    );
    let mode = std::fs::metadata(&saved[1])
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0, "saved content must not be executable");
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("directory").count(),
        5,
        "no temporary file may remain"
    );
}

#[test]
fn concurrent_saves_keep_every_payload() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        let handles = (0u8..8)
            .map(|value| {
                let dir = dir.path();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    let path =
                        save_new_file(dir, "image", "png", &[value]).expect("concurrent save");
                    (path, value)
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            let (path, value) = handle.join().expect("save thread");
            assert_eq!(std::fs::read(path).expect("saved payload"), [value]);
        }
    });
    assert_eq!(
        std::fs::read_dir(dir.path())
            .expect("image directory")
            .count(),
        8
    );
}

#[test]
fn saving_reports_a_missing_destination() {
    let dir = tempfile::tempdir().expect("temporary directory");
    assert_eq!(
        save_new_file(&dir.path().join("missing"), "image", "png", b"image")
            .expect_err("missing destination must fail")
            .kind(),
        io::ErrorKind::NotFound
    );
}
