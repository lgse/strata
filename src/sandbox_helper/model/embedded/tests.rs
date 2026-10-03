// SPDX-License-Identifier: MIT

use super::*;
use resvg::tiny_skia::{Color, Pixmap};
use std::io::Write;

fn png() -> Vec<u8> {
    let mut image = Pixmap::new(512, 512).expect("thumbnail pixmap");
    image.fill(Color::from_rgba8(255, 0, 0, 255));
    image.encode_png().expect("thumbnail PNG")
}

fn package(path: &Path, format: ModelFormat, thumbnails: &[Vec<u8>]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).expect("package file"));
    let options = zip::write::SimpleFileOptions::default();
    for model in ["3D/3dmodel.model", "3D/Objects/part.model"] {
        zip.start_file(model, options).expect("model part");
        zip.write_all(b"geometry must never be parsed")
            .expect("invalid geometry");
    }
    for (i, bytes) in thumbnails.iter().enumerate() {
        let name = if format == ModelFormat::FreeCad {
            "thumbnails/Thumbnail.png".to_owned()
        } else {
            format!("Metadata/{i}-thumbnail.png")
        };
        zip.start_file(name, options).expect("thumbnail part");
        zip.write_all(bytes).expect("thumbnail bytes");
    }
    zip.finish().expect("finished package");
}

#[test]
fn one_usable_thumbnail_survives_corrupt_candidates_and_multipart_geometry() {
    let dir = tempfile::tempdir().expect("fixture directory");
    let path = dir.path().join("extensionless-target");
    let valid = png();
    for candidates in [
        vec![valid.clone()],
        vec![b"corrupt".to_vec(), valid.clone()],
        vec![valid, b"corrupt".to_vec()],
    ] {
        package(&path, ModelFormat::ThreeMf, &candidates);
        let image =
            thumbnail(&path, ModelFormat::ThreeMf, 256, 256, &|_| {}).expect("embedded image");
        assert_eq!(
            Pixmap::decode_png(&image)
                .expect("decoded image")
                .pixel(0, 0)
                .expect("first pixel")
                .red(),
            255
        );
        let (width, height) = crate::sandbox::png_dimensions(&image).expect("PNG dimensions");
        assert!(
            width <= 256 && height <= 256,
            "thumbnail must fit the worker output budget"
        );
        let quick = super::super::render_reporting(&path, "3mf:256x256:00ff00:101010", &|_| {})
            .expect("embedded quick preview");
        assert_eq!(image, quick);
    }
}

#[test]
fn unsupported_compression_in_geometry_and_other_candidates_does_not_hide_a_thumbnail() {
    let dir = tempfile::tempdir().expect("fixture directory");
    let path = dir.path().join("input");
    package(&path, ModelFormat::ThreeMf, &[png(), png()]);
    let mut archive = zip::ZipArchive::new(fs::File::open(&path).expect("package")).expect("ZIP");
    let offsets: Vec<_> = [0, 2]
        .into_iter()
        .map(|i| {
            let file = archive.by_index_raw(i).expect("raw part");
            (
                file.header_start() as usize + 8,
                file.central_header_start() as usize + 10,
            )
        })
        .collect();
    drop(archive);
    let mut bytes = fs::read(&path).expect("package bytes");
    for (local, central) in offsets {
        bytes[local..local + 2].copy_from_slice(&0xfffeu16.to_le_bytes());
        bytes[central..central + 2].copy_from_slice(&0xfffeu16.to_le_bytes());
    }
    fs::write(&path, bytes).expect("unsupported methods");
    assert!(thumbnail(&path, ModelFormat::ThreeMf, 256, 256, &|_| {}).is_ok());
}

#[test]
fn missing_invalid_and_ambiguous_thumbnails_never_fall_back_to_geometry() {
    let dir = tempfile::tempdir().expect("fixture directory");
    let path = dir.path().join("input");
    for candidates in [vec![], vec![b"corrupt".to_vec()], vec![png(), png()]] {
        package(&path, ModelFormat::ThreeMf, &candidates);
        assert!(
            thumbnail(&path, ModelFormat::ThreeMf, 256, 256, &|_| {})
                .expect_err("bounded failure")
                .contains("no unambiguous usable")
        );
    }
    package(&path, ModelFormat::FreeCad, &[png()]);
    assert!(thumbnail(&path, ModelFormat::FreeCad, 256, 256, &|_| {}).is_ok());
    package(&path, ModelFormat::FreeCad, &[]);
    assert!(
        thumbnail(&path, ModelFormat::FreeCad, 256, 256, &|_| {})
            .expect_err("bounded failure")
            .contains("no usable embedded thumbnail")
    );
}

#[test]
fn candidate_and_decode_budgets_are_enforced_before_expensive_work() {
    let dir = tempfile::tempdir().expect("fixture directory");
    let path = dir.path().join("input");
    package(
        &path,
        ModelFormat::ThreeMf,
        &vec![b"bad".to_vec(); MAX_THUMBNAIL_CANDIDATES + 1],
    );
    assert!(
        Package::open(&path, ModelFormat::ThreeMf)
            .err()
            .expect("candidate rejection")
            .contains("candidate limit")
    );

    let mut enormous = png();
    enormous[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    enormous[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
    package(&path, ModelFormat::ThreeMf, &[enormous, png()]);
    assert!(thumbnail(&path, ModelFormat::ThreeMf, 256, 256, &|_| {}).is_ok());

    let oversized = vec![0; MAX_THUMBNAIL_BYTES as usize + 1];
    package(&path, ModelFormat::ThreeMf, &[oversized, png()]);
    assert!(thumbnail(&path, ModelFormat::ThreeMf, 256, 256, &|_| {}).is_ok());

    let mut costly = png();
    costly[16..20].copy_from_slice(&4096u32.to_be_bytes());
    costly[20..24].copy_from_slice(&4096u32.to_be_bytes());
    package(&path, ModelFormat::ThreeMf, &[costly, png()]);
    assert!(
        thumbnail(&path, ModelFormat::ThreeMf, 256, 256, &|_| {})
            .expect_err("bounded failure")
            .contains("pixel budget")
    );

    let mut candidates = vec![vec![0; MAX_THUMBNAIL_BYTES as usize]; 4];
    candidates.push(png());
    package(&path, ModelFormat::ThreeMf, &candidates);
    assert!(
        thumbnail(&path, ModelFormat::ThreeMf, 256, 256, &|_| {})
            .expect_err("bounded failure")
            .contains("byte budget")
    );
}
