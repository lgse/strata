// SPDX-License-Identifier: MIT

use std::{collections::HashMap, io::Cursor};

use rune::Mods;
use strata_components::contract::dnd::{
    DragPayload, DropActions, DropData, DropItem, DropOffer, Dropped, URI_LIST,
};
use strata_core::i18n::tr;
use strata_fs::services::{DroppedFile, WebLink};
use strata_platform::transfer::ReadError;

use super::{Content, Got, Reads, next_read, offered, read};

const ADDRESS: &str = "https://example.com/page";

/// What a drop offers: each type with its bytes.
type Offered<'a> = Vec<(&'a str, &'a [u8])>;

/// One drop: what it is, what it offers, and what it saves.
type Case<'a> = (&'a str, Offered<'a>, Option<Content>);

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
    let expected_png = super::decode::png_of(&jpeg).expect("decodes");
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

/// A read past P1's bound (`ReadError::TooLarge`, as the binder carries it) says so, as GTK's
/// "The dropped content is too large", rather than leaving the drop refused with no word; what
/// else the drop offers is tried first.
#[test]
fn a_dropped_type_too_large_to_read_says_so() {
    let named = "application/octet-stream;name=\"big.png\"";
    let uris = format!("{ADDRESS}\r\n");
    let too_large = || Some(Content::Unreadable(tr("The dropped content is too large")));
    // What it is, what it offers, the type whose read was too large, and what it saves.
    let cases: Vec<(&str, Offered<'_>, &str, Option<Content>)> = vec![
        ("image bytes", vec![(named, b"")], named, too_large()),
        (
            "text",
            vec![("text/plain;charset=utf-8", b"")],
            "text/plain;charset=utf-8",
            too_large(),
        ),
        (
            "image bytes, with an address to fall back on",
            vec![(named, b""), (URI_LIST, uris.as_bytes())],
            named,
            saves(vec![link_file(ADDRESS)]),
        ),
    ];
    for (what, offered, failed, expected) in cases {
        let mut drop = dropped(&offered);
        drop.data.items.retain(|item| item.mime != failed);
        let failure = crate::dnd::adapter::failure_of_p1(&ReadError::TooLarge);
        drop.data = drop.data.failed(vec![(failed.to_owned(), failure)]);
        assert_eq!(read(&drop), expected, "{what}");
    }
}

/// A drop being read: the types offered, and the bytes each one read holds.
struct Reading {
    offered: Vec<&'static str>,
    read: HashMap<String, Vec<u8>>,
}

impl Reads for Reading {
    fn offered(&self) -> Vec<&str> {
        self.offered.clone()
    }

    fn got(&self, mime: &str) -> Got<'_> {
        self.read
            .get(mime)
            .map_or(Got::Unread, |bytes| Got::Bytes(bytes))
    }

    fn local(&self) -> bool {
        false
    }
}

/// A drop reads only the types GTK's order needs, one at a time: the file list first, then a
/// browser's image bytes, then image data only when the bytes are no image, and text only
/// when no image and no address settles it.
#[test]
fn a_drop_reads_only_the_types_it_uses() {
    let jpeg = encoded(image::ImageFormat::Jpeg);
    let png = encoded(image::ImageFormat::Png);
    let links = format!("{ADDRESS}\r\n");
    let named = "application/octet-stream;name=\"photo.jpg\"";
    let text = "text/plain;charset=utf-8";
    let browser = [URI_LIST, named, "image/png", text];
    let plain = [named, "image/png", text];
    let files = [URI_LIST, "image/png", text];
    // What it is, the types offered, what each read gives, and the types read, in order.
    let cases: Vec<(&str, &[&'static str], Offered<'_>, Vec<&str>)> = vec![
        (
            "a browser's image",
            &browser,
            vec![(URI_LIST, links.as_bytes()), (named, &jpeg)],
            vec![URI_LIST, named],
        ),
        (
            "image bytes that are no image",
            &browser,
            vec![
                (URI_LIST, links.as_bytes()),
                (named, b"no image"),
                ("image/png", &png),
            ],
            vec![URI_LIST, named, "image/png"],
        ),
        (
            "no image at all, with an address",
            &browser,
            vec![
                (URI_LIST, links.as_bytes()),
                (named, b"no"),
                ("image/png", b"no"),
            ],
            vec![URI_LIST, named, "image/png"],
        ),
        (
            "no image and no address",
            &plain,
            vec![(named, b"no"), ("image/png", b"no"), (text, b"words")],
            vec![named, "image/png", text],
        ),
        (
            "files",
            &files,
            vec![(URI_LIST, b"file:///tmp/a.png\r\n")],
            vec![URI_LIST],
        ),
    ];
    for (what, offered, answers, wanted) in cases {
        let answers: HashMap<&str, &[u8]> = answers.into_iter().collect();
        let mut reading = Reading {
            offered: offered.to_vec(),
            read: HashMap::new(),
        };
        let mut order = Vec::new();
        while let Some(mime) = next_read(&reading) {
            assert!(
                order.len() < offered.len(),
                "{what}: {mime} asked for again"
            );
            let bytes = answers.get(mime.as_str()).copied().unwrap_or_default();
            reading.read.insert(mime.clone(), bytes.to_vec());
            order.push(mime);
        }
        assert_eq!(order, wanted, "{what}");
    }
}

/// Text in a charset other than UTF-8 is converted from it, as GTK's string reader converts;
/// UTF-8 that is not valid is no text.
#[test]
fn dropped_text_is_read_in_its_charset() {
    let utf16: Vec<u8> = "café".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let cases: Vec<(&str, &[u8], Option<Content>)> = vec![
        (
            "text/plain;charset=ISO-8859-1",
            b"caf\xe9",
            saves(vec![text_file("café")]),
        ),
        (
            "text/plain;charset=UTF-16LE",
            &utf16,
            saves(vec![text_file("café")]),
        ),
        (
            "text/plain;charset=UTF-8",
            "café".as_bytes(),
            saves(vec![text_file("café")]),
        ),
        (
            "text/plain;charset=utf-8",
            b"caf\xe9",
            Some(Content::Refused),
        ),
        (
            "text/plain",
            "café".as_bytes(),
            saves(vec![text_file("café")]),
        ),
    ];
    for (mime, bytes, expected) in cases {
        assert_eq!(read(&dropped(&[(mime, bytes)])), expected, "{mime}");
    }
}
