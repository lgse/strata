// SPDX-License-Identifier: MIT

use super::*;
use image::{GenericImageView, Rgba, RgbaImage};

#[test]
fn static_inputs_convert_at_full_resolution_and_preserve_alpha() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("misleading.svg");
    for kind in [
        ImageKind::Jpeg,
        ImageKind::Bmp,
        ImageKind::Gif,
        ImageKind::WebP,
    ] {
        let mut pixels = RgbaImage::from_pixel(23, 17, Rgba([255, 0, 0, 255]));
        pixels.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        let image = DynamicImage::ImageRgba8(pixels);
        let image = if kind == ImageKind::Jpeg {
            DynamicImage::ImageRgb8(image.to_rgb8())
        } else {
            image
        };
        image
            .save_with_format(&path, kind.format())
            .expect("input image");
        assert_eq!(
            serde_json::from_slice::<ImageKind>(&process(&path, false).expect("inspect"))
                .expect("kind"),
            kind
        );
        let png = process(&path, true).expect("conversion");
        assert_eq!(
            image::guess_format(&png).expect("format"),
            image::ImageFormat::Png
        );
        let output = image::load_from_memory(&png).expect("decode output");
        assert_eq!(output.dimensions(), (23, 17));
        if kind != ImageKind::Jpeg {
            assert_eq!(output.get_pixel(0, 0).0[3], 0, "{kind:?}");
            assert_eq!(output.get_pixel(5, 5), Rgba([255, 0, 0, 255]), "{kind:?}");
        }
    }
}

#[test]
fn jpeg_orientation_is_applied_and_colour_profile_is_preserved() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("photo");
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut bytes);
    // Little-endian TIFF EXIF, one SHORT orientation entry (6 = rotate 90 degrees).
    encoder
        .set_exif_metadata(vec![
            73, 73, 42, 0, 8, 0, 0, 0, 1, 0, 18, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ])
        .expect("EXIF");
    encoder
        .set_icc_profile(b"test profile".to_vec())
        .expect("ICC");
    encoder
        .write_image(&[128; 18], 2, 3, image::ExtendedColorType::Rgb8)
        .expect("JPEG");
    fs::write(&path, bytes).expect("fixture");
    let png = process(&path, true).expect("convert");
    let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(png)).expect("PNG");
    assert_eq!(decoder.dimensions(), (3, 2));
    assert_eq!(
        decoder.icc_profile().expect("ICC"),
        Some(b"test profile".to_vec())
    );
}

#[test]
fn grayscale_is_preserved_and_incompatible_profiles_are_rejected() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("gray.jpg");
    for space in [b"GRAY", b"CMYK"] {
        let mut profile = vec![0; 40];
        profile[16..20].copy_from_slice(space);
        profile[36..40].copy_from_slice(b"acsp");
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut bytes);
        encoder.set_icc_profile(profile.clone()).expect("ICC");
        encoder
            .write_image(&[128; 6], 2, 3, image::ExtendedColorType::L8)
            .expect("grayscale JPEG");
        fs::write(&path, bytes).expect("input");
        let result = process(&path, true);
        if space == b"CMYK" {
            assert!(
                result
                    .expect_err("incompatible profile")
                    .contains("colour profile")
            );
        } else {
            let mut decoder =
                image::codecs::png::PngDecoder::new(Cursor::new(result.expect("PNG")))
                    .expect("decode");
            assert_eq!(decoder.color_type(), image::ColorType::L8);
            assert_eq!(decoder.icc_profile().expect("profile"), Some(profile));
        }
    }
}

#[test]
fn animated_gif_and_webp_are_rejected_before_conversion() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("image");
    let mut gif = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
        for colour in [[255, 0, 0, 255], [0, 0, 255, 255]] {
            encoder
                .encode_frame(image::Frame::new(RgbaImage::from_pixel(2, 3, Rgba(colour))))
                .expect("frame");
        }
    }
    let webp = vec![
        82, 73, 70, 70, 132, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 88, 10, 0, 0, 0, 2, 0, 0, 0, 1,
        0, 0, 2, 0, 0, 65, 78, 73, 77, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 65, 78, 77, 70, 40, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 1, 0, 0, 2, 0, 0, 100, 0, 0, 2, 86, 80, 56, 76, 15, 0, 0, 0, 47, 1, 128,
        0, 0, 7, 16, 253, 143, 254, 7, 34, 162, 255, 1, 0, 65, 78, 77, 70, 40, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 1, 0, 0, 2, 0, 0, 100, 0, 0, 0, 86, 80, 56, 76, 15, 0, 0, 0, 47, 1, 128, 0, 0, 7, 16,
        209, 255, 254, 7, 34, 162, 255, 1, 0,
    ];
    for bytes in [gif, webp] {
        fs::write(&path, bytes).expect("fixture");
        for convert in [false, true] {
            assert!(
                process(&path, convert)
                    .expect_err("reject animation")
                    .contains("Animated")
            );
        }
    }
}

#[test]
fn invalid_unsupported_and_oversized_images_do_not_produce_output() {
    let directory = tempfile::tempdir().expect("fixture");
    let input = directory.path().join("image.jpg");
    let output = directory.path().join("output.png");
    for bytes in [
        b"<html>error</html>".as_slice(),
        b"<svg/>",
        b"II*\0TIFF",
        b"\xff\xd8\xffbroken",
        b"GIF89a",
    ] {
        fs::write(&input, bytes).expect("fixture");
        assert!(run(&input, &output, true).is_err());
        assert!(!output.exists());
    }
    fs::File::create(&input)
        .expect("large file")
        .set_len(MAX_INPUT_BYTES + 1)
        .expect("size");
    assert!(
        process(&input, true)
            .expect_err("bounded input")
            .contains("32 MiB")
    );
    // A GIF logical screen exceeding the pixel budget, with no pixel allocation.
    let mut gif = Vec::new();
    image::codecs::gif::GifEncoder::new(&mut gif)
        .encode_frame(image::Frame::new(RgbaImage::from_pixel(
            1,
            1,
            Rgba([0, 0, 0, 255]),
        )))
        .expect("GIF");
    gif[6..8].copy_from_slice(&5000u16.to_le_bytes());
    gif[8..10].copy_from_slice(&5000u16.to_le_bytes());
    fs::write(&input, gif).expect("large dimensions");
    let error = process(&input, true).expect_err("bounded pixels");
    assert!(error.contains("megapixel"), "{error}");
}

#[test]
fn png_is_validated_without_reencoding() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("image.jpg");
    DynamicImage::new_rgba8(2, 3)
        .save_with_format(&path, image::ImageFormat::Png)
        .expect("PNG");
    assert_eq!(
        process(&path, true).expect("PNG"),
        fs::read(path).expect("original")
    );
}
