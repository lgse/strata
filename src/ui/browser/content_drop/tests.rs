// SPDX-License-Identifier: MIT

use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn dropped_text_preserves_unicode_newlines_and_markup_as_plain_text() {
    let text = "日本語 and café\n<b>not HTML</b>\tend\n";
    let contents = content_files(&text.to_value()).expect("text content");
    let directory = tempfile::tempdir().expect("destination");
    let path = save_file(directory.path(), &contents[0]).expect("saved text");
    assert_eq!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("txt")
    );
    assert_eq!(std::fs::read_to_string(path).expect("text file"), text);
    assert!(content_files(&"".to_value()).is_none());
}

#[test]
fn texture_subclass_drops_save_the_supplied_image_pixels() {
    crate::test_support::gtk_test(
        "ui::browser::content_drop::tests::texture_subclass_drops_save_the_supplied_image_pixels",
        || {
            let directory = tempfile::tempdir().expect("image destination");
            let destination = Location::local(directory.path());
            let location_for_drop = destination.clone();
            let prepared =
                super::super::prepare_file_drop_target(move || Some(location_for_drop.clone()));
            let pixels = [10, 20, 30, 255, 40, 50, 60, 255];
            let texture = gdk::MemoryTexture::new(
                2,
                1,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from(&pixels),
                8,
            );
            let Some(DropRequest::Content(contents)) = DropRequest::from_value(
                &prepared.target,
                &texture.to_value(),
                &destination,
                &prepared.state,
            ) else {
                panic!("image drop request");
            };
            let path = save_file(directory.path(), &contents[0]).expect("saved image");
            let loaded = gdk::Texture::from_file(&gio::File::for_path(path)).expect("image file");
            let mut downloader = gdk::TextureDownloader::new(&loaded);
            downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
            assert_eq!(
                &downloader.download_bytes().0.as_ref()[..pixels.len()],
                &pixels
            );
        },
    );
}

#[test]
fn browser_binary_data_keeps_pixels_and_captured_destination() {
    crate::test_support::gtk_test(
        "ui::browser::content_drop::tests::browser_binary_data_keeps_pixels_and_captured_destination",
        || {
            let directory = tempfile::tempdir().expect("destination");
            let captured = Location::local(directory.path());
            let elsewhere = Location::local("/tmp/elsewhere");
            let texture = gdk::MemoryTexture::new(
                1,
                1,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from(&[30, 80, 120, 255]),
                4,
            );
            let png = texture.save_to_png_bytes();
            let stream = gio::MemoryInputStream::from_bytes(&png);
            let file = glib::MainContext::default()
                .block_on(binary::read_file(
                    &stream.upcast(),
                    "application/octet-stream;name=\"browser-photo.png\"",
                    captured.clone(),
                ))
                .expect("browser bytes");
            let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let prepared = super::super::prepare_file_drop_target(move || Some(elsewhere.clone()))
                .attach_to(&widget);
            let controllers = widget.observe_controllers();
            assert!(
                (0..controllers.n_items())
                    .filter_map(|index| controllers.item(index))
                    .any(|controller| controller.is::<gtk::DropTargetAsync>()),
                "binary drops must have a receiver on the destination widget"
            );
            let route = Rc::new(std::cell::RefCell::new(None));
            let recorded = route.clone();
            let state = prepared.state.clone();
            prepared.target.connect_drop(move |target, value, _, _| {
                let Some(DropRequest::Binary(destination, contents)) = DropRequest::from_value(
                    target,
                    value,
                    &Location::local("/tmp/elsewhere"),
                    &state,
                ) else {
                    return false;
                };
                *recorded.borrow_mut() = Some((destination, contents));
                true
            });
            assert!(
                prepared.target.emit_by_name::<bool>(
                    "drop",
                    &[&glib::BoxedValue(file.to_value()), &0f64, &0f64]
                )
            );
            let (destination, contents) = route.borrow_mut().take().expect("binary routing");
            assert_eq!(destination, captured);
            let path = save_file(directory.path(), &contents[0]).expect("saved image");
            assert_eq!(path.file_name().expect("name"), "browser-photo.png");
            assert_eq!(std::fs::read(path).expect("bytes"), png.as_ref());
        },
    );
}

#[test]
fn browser_uri_lists_create_shortcuts_and_reject_mixed_file_lists() {
    let files = gdk::FileList::from_array(&[
        gio::File::for_uri("https://example.org/one"),
        gio::File::for_uri("https://example.org/two?q=three%20four"),
    ]);
    let contents = content_files(&files.to_value()).expect("web links");
    let directory = tempfile::tempdir().expect("destination");
    for (content, address) in contents.iter().zip([
        "https://example.org/one",
        "https://example.org/two?q=three%20four",
    ]) {
        let path = save_file(directory.path(), content).expect("saved link");
        let text = std::fs::read_to_string(&path).expect("shortcut");
        assert_eq!(
            WebLink::from_desktop_entry(&text)
                .expect("valid shortcut")
                .expect("web link")
                .address,
            address
        );
        assert_eq!(
            std::fs::metadata(path).expect("mode").permissions().mode() & 0o111,
            0
        );
    }
    assert_eq!(
        std::fs::read_dir(directory.path()).expect("files").count(),
        2
    );
    let mixed = gdk::FileList::from_array(&[
        gio::File::for_path("/tmp/file"),
        gio::File::for_uri("https://example.org/"),
    ]);
    assert!(content_files(&mixed.to_value()).is_none());
}

#[test]
fn unsafe_uri_text_is_saved_without_execution_or_network_access() {
    let text = "javascript:alert(1)";
    let contents = content_files(&text.to_value()).expect("plain text");
    let directory = tempfile::tempdir().expect("destination");
    let path = save_file(directory.path(), &contents[0]).expect("saved text");
    assert_eq!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("txt")
    );
    assert_eq!(std::fs::read_to_string(path).expect("text"), text);
}

#[test]
fn content_publication_preserves_collisions_symlinks_and_concurrent_arrivals() {
    let directory = tempfile::tempdir().expect("destination");
    let missing = directory.path().join("missing");
    symlink(&missing, directory.path().join("note.txt")).expect("dangling link");
    std::fs::create_dir(directory.path().join("note (1).txt")).expect("directory collision");
    std::fs::write(directory.path().join("note (2).txt"), "original").expect("file collision");
    let barrier = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0u8..4)
            .map(|value| {
                let barrier = &barrier;
                let directory = directory.path();
                scope.spawn(move || {
                    barrier.wait();
                    let path = save_file(
                        directory,
                        &ContentFile {
                            stem: "note".into(),
                            extension: "txt",
                            data: ContentData::Bytes(vec![value]),
                        },
                    )
                    .expect("saved content");
                    (path, value)
                })
            })
            .collect();
        for handle in handles {
            let (path, value) = handle.join().expect("worker");
            assert_eq!(std::fs::read(path).expect("payload"), [value]);
        }
    });
    assert!(!missing.exists());
    assert_eq!(
        std::fs::read(directory.path().join("note (2).txt")).expect("original"),
        b"original"
    );
    assert_eq!(
        std::fs::read_dir(directory.path()).expect("files").count(),
        7
    );
}

#[test]
fn content_writes_reject_invalid_names_and_report_missing_destinations() {
    let directory = tempfile::tempdir().expect("destination");
    let content = ContentFile {
        stem: "../outside".into(),
        extension: "txt",
        data: ContentData::Bytes(b"text".to_vec()),
    };
    assert_eq!(
        save_file(directory.path(), &content)
            .expect_err("unsafe name")
            .kind(),
        io::ErrorKind::InvalidInput
    );
    let content = ContentFile {
        stem: "note".into(),
        ..content
    };
    assert_eq!(
        save_file(&directory.path().join("missing"), &content)
            .expect_err("missing folder")
            .kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("unchanged folder")
            .count(),
        0
    );
}
