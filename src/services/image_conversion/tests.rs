// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn detection_uses_bytes_and_not_the_filename() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("photo.PNG");
    for (bytes, expected) in [
        (b"\xff\xd8\xffdata".as_slice(), Some(ImageKind::Jpeg)),
        (b"GIF89a", Some(ImageKind::Gif)),
        (b"<html>error</html>", None),
        (b"II*\0TIFF", None),
        (b"Bitmap notes", None),
        (b"BMW", None),
    ] {
        fs::write(&path, bytes).expect("fixture");
        assert_eq!(detect(&path), expected);
    }
}

#[test]
fn bmp_detection_requires_reserved_zero_and_a_known_dib_header() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("README");
    let mut header = [0_u8; 32];
    header[..2].copy_from_slice(b"BM");
    for (size, expected) in [
        (12_u32, Some(ImageKind::Bmp)),
        (40, Some(ImageKind::Bmp)),
        (52, Some(ImageKind::Bmp)),
        (56, Some(ImageKind::Bmp)),
        (108, Some(ImageKind::Bmp)),
        (124, Some(ImageKind::Bmp)),
        (0, None),
        (13, None),
        (128, None),
    ] {
        header[14..18].copy_from_slice(&size.to_le_bytes());
        fs::write(&path, header).expect("header");
        assert_eq!(detect(&path), expected, "DIB header size {size}");
    }
    header[14..18].copy_from_slice(&40_u32.to_le_bytes());
    for reserved in 6..10 {
        header[reserved] = 1;
        fs::write(&path, header).expect("reserved field");
        assert_eq!(detect(&path), None, "nonzero reserved byte {reserved}");
        header[reserved] = 0;
    }
    fs::write(&path, &header[..17]).expect("truncated DIB header size");
    assert_eq!(detect(&path), None);
}

#[test]
fn png_names_fit_the_filesystem_even_for_long_unicode_stems() {
    let directory = tempfile::tempdir().expect("fixture");
    let input = directory.path().join(format!("{}.x", "€".repeat(84)));
    let output = with_extension(&input, "png");
    fs::write(&output, b"PNG").expect("valid filename");
    assert_eq!(output.parent(), input.parent());
    assert_eq!(output.extension().expect("extension"), "png");
}

#[test]
fn cancelled_conversion_does_not_write_an_output() {
    let directory = tempfile::tempdir().expect("fixture");
    let input = directory.path().join("input.jpg");
    let output = directory.path().join("output.png");
    fs::write(&input, b"\xff\xd8\xff").expect("fixture");
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(
        convert(&input, &output, &cancelled)
            .expect_err("cancelled")
            .contains("Preview cancelled")
    );
    assert!(input.exists());
}
