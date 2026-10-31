// SPDX-License-Identifier: MIT

use super::*;

fn saved_names(value: &glib::Value) -> Option<Vec<String>> {
    dropped_content(value).map(|files| files.iter().map(DroppedFile::name).collect())
}

fn file_list(uris: &[&str]) -> glib::Value {
    gdk::FileList::from_array(
        &uris
            .iter()
            .map(|uri| gio::File::for_uri(uri))
            .collect::<Vec<_>>(),
    )
    .to_value()
}

#[test]
fn dropped_values_save_content_while_existing_files_stay_transfers() {
    let texture = gdk::MemoryTexture::new(
        1,
        1,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_static(&[255, 0, 0, 255]),
        4,
    );
    let image = dropped_content(&texture.upcast::<gdk::Texture>().to_value()).expect("image");
    assert_eq!(
        image.iter().map(DroppedFile::name).collect::<Vec<_>>(),
        ["image.png"]
    );
    assert!(image[0].bytes().starts_with(b"\x89PNG"));

    for (value, saved) in [
        (
            file_list(&["https://example.com/a", "http://example.org/b"]),
            Some(vec!["example.com.desktop", "example.org.desktop"]),
        ),
        (file_list(&["file:///fixture/notes.txt"]), None),
        (
            file_list(&["https://example.com/a", "file:///fixture/notes.txt"]),
            None,
        ),
        (file_list(&["sftp://host.example/notes.txt"]), None),
        (
            "https://example.com/watch?v=1".to_value(),
            Some(vec!["example.com.desktop"]),
        ),
        (
            "Selected words\nhttps://example.com/".to_value(),
            Some(vec!["dropped text.txt"]),
        ),
        ("".to_value(), None),
    ] {
        assert_eq!(
            saved_names(&value),
            saved.map(|names| names.into_iter().map(str::to_owned).collect()),
            "{value:?}"
        );
    }
}

#[test]
fn only_browser_file_contents_formats_are_read_as_images() {
    for (mime, found, name) in [
        ("application/octet-stream", true, None),
        (
            r#"application/octet-stream;name="cat \"one\" \\ two.jpg""#,
            true,
            Some(r#"cat "one" \ two.jpg"#),
        ),
        ("application/octet-streamish", false, None),
        ("application/pdf", false, None),
    ] {
        let formats = gdk::ContentFormats::new(&["text/plain", mime]);
        assert_eq!(file_contents_mime(&formats).is_some(), found, "{mime}");
        assert_eq!(supplied_name(mime).as_deref(), name, "{mime}");
    }
}
