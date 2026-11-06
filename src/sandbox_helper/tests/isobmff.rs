// SPDX-License-Identifier: MIT

use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::sandbox::browser::wire::Operation;
use crate::sandbox_helper::{browser_render, image_dimensions, isobmff, render_raw};

fn image_box(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(data.len() + 8)
        .expect("box size")
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(data);
    bytes
}

fn extended_box(kind: &[u8; 4], size: u64) -> Vec<u8> {
    [1_u32.to_be_bytes().as_slice(), kind, &size.to_be_bytes()].concat()
}

fn extent(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0; 4];
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    image_box(b"ispe", &bytes)
}

fn metadata(primary: &[u8], properties: &[u8], associations: &[u8]) -> Vec<u8> {
    [
        vec![0; 4],
        image_box(b"pitm", primary),
        image_box(
            b"iprp",
            &[
                image_box(b"ipco", properties),
                image_box(b"ipma", associations),
            ]
            .concat(),
        ),
    ]
    .concat()
}

fn container(metadata: &[u8]) -> Vec<u8> {
    [
        image_box(b"ftyp", b"mif1\0\0\0\0avif"),
        image_box(b"meta", metadata),
    ]
    .concat()
}

fn image_metadata(width: u32, height: u32, rotation: u8) -> Vec<u8> {
    metadata(
        &[0, 0, 0, 0, 0, 42],
        &[
            extent(32, 24),
            extent(width, height),
            image_box(b"irot", &[rotation]),
        ]
        .concat(),
        &[
            0, 0, 0, 0, // version and flags
            0, 0, 0, 2, // entry count
            0, 1, 1, 1, // thumbnail item -> first property
            0, 42, 2, 0x82, 3, // primary item -> essential extent and rotation
        ],
    )
}

#[test]
fn primary_image_metadata_does_not_need_avif_or_heif_pixel_data() {
    let directory = tempfile::tempdir().expect("image headers");
    for (brand, rotation, expected) in [
        (b"avif", 0, (4000, 3000)),
        (b"heic", 1, (3000, 4000)),
        (b"mif1", 2, (4000, 3000)),
        (b"avif", 3, (3000, 4000)),
    ] {
        let path = directory.path().join("header.image");
        let mut bytes = container(&image_metadata(4000, 3000, rotation));
        bytes[16..20].copy_from_slice(brand);
        std::fs::write(&path, bytes).expect("write header without pixels");
        assert_eq!(
            image_dimensions(&path),
            Some(expected),
            "{brand:?}, rotation {rotation}"
        );
        let response = browser_render(&path, Operation::ImageMetadata);
        let metadata: serde_json::Value =
            serde_json::from_slice(&response.metadata).expect("metadata without a pixel decode");
        assert_eq!(metadata["streams"][0]["width"], expected.0);
        assert_eq!(metadata["streams"][0]["height"], expected.1);
    }
}

#[test]
fn primary_item_ids_and_property_indices_support_both_field_widths() {
    for flags in [0_u32, 1, 0x0100_0000, 0x0100_0001] {
        let wide_id = flags >> 24 != 0;
        let wide_index = flags & 1 != 0;
        let primary = if wide_id {
            70_000_u32.to_be_bytes().to_vec()
        } else {
            42_u16.to_be_bytes().to_vec()
        };
        let mut pitm = vec![u8::from(wide_id), 0, 0, 0];
        pitm.extend_from_slice(&primary);
        let mut properties = Vec::new();
        let index = if wide_index { 130 } else { 1 };
        for _ in 1..index {
            properties.extend(image_box(b"free", &[]));
        }
        properties.extend(extent(2400, 1600));
        let mut associations = flags.to_be_bytes().to_vec();
        associations.extend_from_slice(&1_u32.to_be_bytes());
        associations.extend_from_slice(&primary);
        associations.push(2);
        if wide_index {
            associations.extend_from_slice(&0_u16.to_be_bytes());
            associations.extend_from_slice(&(0x8000_u16 | index).to_be_bytes());
        } else {
            associations.extend_from_slice(&[0, 0x80 | u8::try_from(index).expect("small index")]);
        }
        let bytes = container(&metadata(&pitm, &properties, &associations));
        assert_eq!(
            isobmff::dimensions(&mut io::Cursor::new(bytes)),
            Some((2400, 1600)),
            "flags {flags:#x}"
        );
    }
}

#[test]
fn metadata_boxes_accept_extended_sizes_and_sizes_to_end_of_file() {
    let metadata = image_metadata(1200, 800, 1);
    let mut extended = extended_box(
        b"meta",
        u64::try_from(metadata.len()).expect("metadata length") + 16,
    );
    extended.extend_from_slice(&metadata);
    let mut to_end = image_box(b"meta", &metadata);
    to_end[..4].fill(0);
    for meta in [extended, to_end] {
        let bytes = [image_box(b"ftyp", b"avif\0\0\0\0mif1"), meta].concat();
        assert_eq!(
            isobmff::dimensions(&mut io::Cursor::new(bytes)),
            Some((800, 1200))
        );
    }
}

#[test]
fn malformed_or_unassociated_image_properties_are_not_used_as_dimensions() {
    let valid = container(&image_metadata(4000, 3000, 0));
    for end in 0..valid.len() {
        assert!(
            isobmff::dimensions(&mut io::Cursor::new(&valid[..end])).is_none(),
            "truncated at {end}"
        );
    }
    for (width, height, rotation) in [
        (0, 3000, 0),
        (4000, 0, 0),
        (u32::MAX, 3000, 0),
        (4000, 3000, 4),
    ] {
        let bytes = container(&image_metadata(width, height, rotation));
        assert!(isobmff::dimensions(&mut io::Cursor::new(bytes)).is_none());
    }
    let properties = [extent(4000, 3000), image_box(b"clap", &[0; 32])].concat();
    for associations in [
        vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 1], // only a different item has an extent
        vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 42, 1, 3], // out-of-range property
        vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 42, 2, 1, 2], // primary crop needs decoder fallback
        vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 42, 2, 1, 1], // duplicate spatial extent
        vec![2, 0, 0, 0, 0, 0, 0, 1, 0, 42, 1, 1], // unsupported association version
    ] {
        let bytes = container(&metadata(&[0, 0, 0, 0, 0, 42], &properties, &associations));
        assert!(isobmff::dimensions(&mut io::Cursor::new(bytes)).is_none());
    }
    for size in [2, u64::MAX] {
        let bytes = [
            image_box(b"ftyp", b"avif\0\0\0\0mif1"),
            extended_box(b"meta", size),
        ]
        .concat();
        assert!(isobmff::dimensions(&mut io::Cursor::new(bytes)).is_none());
    }
}

struct ReadCounter<R> {
    source: R,
    bytes: usize,
}

impl<R: Read> Read for ReadCounter<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.source.read(buffer)?;
        self.bytes += count;
        Ok(count)
    }
}

impl<R: Seek> Seek for ReadCounter<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.source.seek(position)
    }
}

#[test]
fn dimension_reads_seek_past_pixels_and_bound_untrusted_metadata() {
    let mut file = tempfile::tempfile().expect("sparse image");
    let ftyp = image_box(b"ftyp", b"avif\0\0\0\0mif1");
    file.write_all(&ftyp).expect("file type");
    let pixel_box_size = 3 * 1024 * 1024 * 1024;
    file.write_all(&extended_box(b"mdat", pixel_box_size))
        .expect("pixel box header");
    file.seek(SeekFrom::Start(ftyp.len() as u64 + pixel_box_size))
        .expect("skip sparse pixels");
    file.write_all(&image_box(b"meta", &image_metadata(4000, 3000, 0)))
        .expect("metadata after pixels");
    let mut reader = ReadCounter {
        source: file,
        bytes: 0,
    };
    assert_eq!(isobmff::dimensions(&mut reader), Some((4000, 3000)));
    assert!(
        reader.bytes < 1024,
        "pixel data must not be read: {} bytes",
        reader.bytes
    );

    let mut file = tempfile::tempfile().expect("oversized metadata");
    file.write_all(&ftyp).expect("file type");
    file.write_all(&extended_box(b"meta", 8 * 1024 * 1024))
        .expect("metadata header");
    file.set_len(8 * 1024 * 1024 + ftyp.len() as u64)
        .expect("sparse metadata");
    let mut reader = ReadCounter {
        source: file,
        bytes: 0,
    };
    assert!(isobmff::dimensions(&mut reader).is_none());
    assert!(
        reader.bytes < 1024,
        "oversized metadata must not be buffered"
    );
}

#[test]
fn oversized_avif_headers_enforce_the_existing_decode_budget() {
    let directory = tempfile::tempdir().expect("oversized header");
    let path = directory.path().join("oversized.avif");
    std::fs::write(&path, container(&image_metadata(20_000, 20_000, 0)))
        .expect("header without pixels");
    let error = render_raw(&path, 800).expect_err("reject before trying to decode");
    assert!(error.contains("decoded frame budget"));
    let response = browser_render(&path, Operation::Image);
    assert!(response.png.is_empty());
    let metadata: serde_json::Value =
        serde_json::from_slice(&response.metadata).expect("dimensions still available");
    assert_eq!(metadata["streams"][0]["width"], 20_000);
    assert_eq!(metadata["streams"][0]["height"], 20_000);
}
