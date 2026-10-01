// SPDX-License-Identifier: MIT

use super::*;
use std::io::Write;

fn image() -> Vec<u8> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(32, 48).expect("image");
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(200, 30, 40, 255));
    pixmap.encode_png().expect("PNG")
}

fn package(path: &Path, members: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).expect("archive"));
    for (name, bytes) in members {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("member");
        zip.write_all(bytes).expect("member bytes");
    }
    zip.finish().expect("finished archive");
}

#[cfg(not(feature = "rar"))]
#[test]
fn rar_disabled_comic_cover_reports_unsupported_build() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("comic.cbr");
    fs::write(&path, b"not a RAR archive").expect("fixture");
    assert_eq!(
        thumbnail(&path, CoverFormat::Cbr).expect_err("RAR is not compiled in"),
        "RAR support is disabled in this build."
    );
}

#[test]
fn comic_uses_natural_order_not_archive_order() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("comic.cbz");
    let image = image();
    package(&path, &[("page10.png", b"invalid"), ("page2.png", &image)]);
    let png = thumbnail(&path, CoverFormat::Cbz).expect("comic cover");
    assert_eq!(crate::sandbox::png_dimensions(&png), Some((32, 48)));
    package(&path, &[("page2.png", &image), ("page1.png", b"invalid")]);
    assert!(thumbnail(&path, CoverFormat::Cbz).is_err());
}

#[test]
fn epub_two_and_three_resolve_declared_covers() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("book.epub");
    let container =
        br#"<container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#;
    let image = image();
    for (package_xml, cover) in [
        (br#"<package><metadata><meta name="cover" content="art"/></metadata><manifest><item id="art" href="../Images/cover.png" media-type="image/png"/></manifest></package>"#.as_slice(), "Images/cover.png"),
        (br#"<package><manifest><item id="art" href="cover.png" media-type="image/png" properties="nav cover-image"/></manifest></package>"#.as_slice(), "OPS/cover.png"),
        (br#"<package><metadata><meta name="cover" content="art"/></metadata><manifest><item id="art" href="OPS/images/cover.png" media-type="image/png"/></manifest></package>"#.as_slice(), "OPS/images/cover.png"),
        (br#"<package><manifest><item id="art" href="cover%20art.png" media-type="image/png" properties="cover-image"/></manifest></package>"#.as_slice(), "OPS/cover art.png"),
    ] {
        package(&path, &[("META-INF/container.xml", container), ("OPS/book.opf", package_xml), (cover, &image)]);
        let png = thumbnail(&path, CoverFormat::Epub).expect("declared cover");
        assert_eq!(crate::sandbox::png_dimensions(&png), Some((32, 48)));
    }
}

#[test]
fn epub_does_not_guess_undeclared_images_or_escape_the_package() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("book.epub");
    let container = br#"<container><rootfile full-path="OPS/book.opf"/></container>"#;
    for package_xml in [
        br#"<package><manifest><item id="art" href="cover.png" media-type="image/png"/></manifest></package>"#.as_slice(),
        br#"<package><manifest><item id="art" href="../../cover.png" media-type="image/png" properties="cover-image"/></manifest></package>"#.as_slice(),
    ] {
        package(&path, &[("META-INF/container.xml", container), ("OPS/book.opf", package_xml), ("OPS/cover.png", &image())]);
        assert!(thumbnail(&path, CoverFormat::Epub).is_err());
    }
}

#[test]
fn archive_limits_reject_large_members_and_decoded_images() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("comic.cbz");
    let oversized = vec![0; MAX_IMAGE_BYTES as usize + 1];
    package(&path, &[("page1.png", &oversized), ("page2.png", &image())]);
    assert!(
        thumbnail(&path, CoverFormat::Cbz)
            .expect_err("oversized first page")
            .contains("size limit")
    );

    let mut png = image();
    png[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    png[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
    package(&path, &[("page1.png", &png)]);
    assert!(thumbnail(&path, CoverFormat::Cbz).is_err());
}

#[test]
fn quick_preview_keeps_more_cover_detail_than_the_thumbnail() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("comic.cbz");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(512, 768).expect("image");
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(200, 30, 40, 255));
    package(&path, &[("page1.png", &pixmap.encode_png().expect("PNG"))]);

    let small = thumbnail(&path, CoverFormat::Cbz).expect("browser thumbnail");
    let large = render(&path, CoverFormat::Cbz, 800).expect("quick preview");
    assert_eq!(crate::sandbox::png_dimensions(&small), Some((171, 256)));
    assert_eq!(crate::sandbox::png_dimensions(&large), Some((512, 768)));
}
