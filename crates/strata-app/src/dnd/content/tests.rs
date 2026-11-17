// SPDX-License-Identifier: MIT

use std::io::Cursor;

use rune::Mods;
use strata_components::contract::dnd::{
    DragPayload, DropActions, DropData, DropItem, DropOffer, Dropped, URI_LIST,
};
use strata_core::i18n::tr;
use strata_fs::services::{DroppedFile, WebLink};

use super::{Content, offered, read};

const ADDRESS: &str = "https://example.com/page";

/// One drop: what it is, what it offers (each type with its bytes), and what it saves.
type Case<'a> = (&'a str, Vec<(&'a str, &'a [u8])>, Option<Content>);

fn dropped(items: &[(&str, &[u8])]) -> Dropped {
    let mimes: Vec<std::borrow::Cow<'static, str>> = items
        .iter()
        .map(|(mime, _)| (*mime).to_owned().into())
        .collect();
    let items = items
        .iter()
        .map(|(mime, bytes)| DropItem::new(*mime, bytes.to_vec()))
        .collect();
    Dropped::new(
        DropOffer::new(mimes, DropActions::COPY),
        DropData::new(items),
        Mods::default(),
    )
}

fn encoded(format: image::ImageFormat) -> Vec<u8> {
    let pixel = image::RgbImage::from_pixel(2, 2, image::Rgb([200, 40, 10]));
    let mut bytes = Cursor::new(Vec::new());
    pixel.write_to(&mut bytes, format).expect("encode");
    bytes.into_inner()
}

fn text_file(text: &str) -> DroppedFile {
    DroppedFile::text(&tr("dropped_text.name"), text.to_owned())
}

fn link_file(address: &str) -> DroppedFile {
    DroppedFile::link(&WebLink::parse(address).expect("a web link"))
}

fn saves(files: Vec<DroppedFile>) -> Option<Content> {
    Some(Content::Save(files))
}

/// Each kind of content another app drops saves what GTK's content drop saves, in its order:
/// a browser's image bytes, then image data, then web addresses, then text; and files are
/// left to the transfer.
#[test]
fn a_foreign_drop_saves_what_gtk_saves_from_it() {
    let png = encoded(image::ImageFormat::Png);
    let uris = format!("{ADDRESS}\r\n");
    let file_uri = "file:///tmp/a.txt\r\n";
    let mixed = format!("{ADDRESS}\r\nfile:///tmp/a.txt\r\n");
    let named = "application/octet-stream;name=\"photo \\\"one\\\".png\"";
    let cases: Vec<Case<'_>> = vec![
        (
            "text",
            vec![("text/plain;charset=utf-8", b"Selected words")],
            saves(vec![text_file("Selected words")]),
        ),
        (
            "text that is one address",
            vec![("text/plain", ADDRESS.as_bytes())],
            saves(vec![link_file(ADDRESS)]),
        ),
        (
            "empty text",
            vec![("text/plain", b"")],
            Some(Content::Refused),
        ),
        (
            "web addresses",
            vec![(URI_LIST, uris.as_bytes()), ("text/plain", b"ignored")],
            saves(vec![link_file(ADDRESS)]),
        ),
        (
            "addresses mixed with files",
            vec![(URI_LIST, mixed.as_bytes())],
            Some(Content::Refused),
        ),
        ("files", vec![(URI_LIST, file_uri.as_bytes())], None),
        (
            "a browser's image bytes",
            vec![(named, &png), (URI_LIST, uris.as_bytes())],
            saves(vec![
                DroppedFile::image(Some("photo \"one\".png"), "unused", png.clone())
                    .expect("an image"),
            ]),
        ),
        (
            "unreadable image bytes, falling back on the address",
            vec![(named, b"not an image"), (URI_LIST, uris.as_bytes())],
            saves(vec![link_file(ADDRESS)]),
        ),
        (
            "unreadable image bytes, falling back on text",
            vec![(named, b"not an image"), ("text/plain", b"caption")],
            saves(vec![text_file("caption")]),
        ),
        (
            "unreadable image bytes alone",
            vec![(named, b"not an image")],
            Some(Content::Unreadable(tr("Unsupported image format"))),
        ),
        (
            "PNG image data",
            vec![("image/png", &png)],
            saves(vec![DroppedFile::png(
                &tr("pasted_image.name"),
                png.clone(),
            )]),
        ),
    ];
    for (what, items, expected) in cases {
        assert_eq!(read(&dropped(&items)), expected, "{what}");
    }
}

/// Image data in another format is saved as a PNG of the same picture.
#[test]
fn dropped_image_data_is_saved_as_png() {
    let jpeg = encoded(image::ImageFormat::Jpeg);
    let Some(Content::Save(files)) = read(&dropped(&[("image/jpeg", &jpeg)])) else {
        panic!("the image is saved");
    };
    let [file] = files.as_slice() else {
        panic!("one file: {files:?}");
    };
    let expected_png = super::png_of(&jpeg).expect("decodes");
    assert_eq!(
        image::guess_format(&expected_png).ok(),
        Some(image::ImageFormat::Png)
    );
    assert_eq!(
        file,
        &DroppedFile::png(&tr("pasted_image.name"), expected_png)
    );
}

/// A drag from this process is never content, even when it carries text: its payload says
/// which files it moves.
#[test]
fn a_drag_from_this_process_is_not_content() {
    let mut drop = dropped(&[("text/plain", b"private")]);
    drop.data = drop.data.local(DragPayload::new(Vec::new(), Vec::new()));
    assert_eq!(read(&drop), None);

    let local = DropOffer::new(vec!["text/plain".into()], DropActions::COPY).local(true);
    assert!(!offered(&local));
}

/// While a drag hovers, content is what another app offers without a file list.
#[test]
fn a_foreign_drag_offers_content_without_a_file_list() {
    let offer = |mimes: &[&'static str]| {
        let mimes: Vec<std::borrow::Cow<'static, str>> = mimes.iter().map(|&m| m.into()).collect();
        DropOffer::new(mimes, DropActions::COPY)
    };
    assert!(offered(&offer(&["text/plain;charset=utf-8"])));
    assert!(offered(&offer(&[
        "application/octet-stream;name=\"a.png\""
    ])));
    assert!(offered(&offer(&["image/webp"])));
    assert!(!offered(&offer(&[URI_LIST, "text/plain"])));
    assert!(!offered(&offer(&["application/x-strata-private"])));
}
