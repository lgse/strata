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
    ] {
        fs::write(&path, bytes).expect("fixture");
        assert_eq!(detect(&path), expected);
    }
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
            .contains("cancelled")
    );
    assert!(!output.exists());
    assert!(input.exists());
}
