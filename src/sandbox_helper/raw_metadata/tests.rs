// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn libraw_and_dcraw_identification_ignore_thumbnail_sizes() {
    let captured = "Camera: Canon EOS R5 ID: 0x123\nEXIF:\n\tLens: RF 50mm F1.8\nMakernotes:\n\tLens: \nISO speed: 400\nShutter: 1/125.0\nFocal length: 50.0 mm\nThumb size: 160 x 120\nFull size: 6032 x 4032\nImage size: 6000 x 4000\nOutput size: 4000 x 6000\nImage flip: 6\n";
    let metadata = parse_identify(captured);
    assert_eq!(metadata.dimensions, Some((4000, 6000)));
    assert_eq!(metadata.camera.as_deref(), Some("Canon EOS R5"));
    assert_eq!(metadata.lens.as_deref(), Some("RF 50mm F1.8"));
    assert_eq!(metadata.focal_length, Some(50.0));
    assert_eq!(metadata.shutter_speed, Some(0.008));
    assert_eq!(metadata.iso, Some(400.0));
    assert_eq!(metadata.gps, None);
    assert_eq!(
        parse_identify("Thumb size: 160 x 120\n"),
        RawMetadata::default()
    );
    let dcraw = "Camera: Nikon Nikon D850\nFull size: 6032 x 4032\nImage size: 6000 x 4000\nOutput size: 4000 x 6000\nShutter: 2.0 sec\n";
    let metadata = parse_identify(dcraw);
    assert_eq!(metadata.dimensions, Some((4000, 6000)));
    assert_eq!(metadata.shutter_speed, Some(2.0));
    assert_eq!(metadata.camera.as_deref(), Some("Nikon D850"));
    for (camera, expected) in [
        ("NIKON Nikon D850", "Nikon D850"),
        ("Nikon NIKON D850", "NIKON D850"),
        ("Nikon NikonX D850", "Nikon NikonX D850"),
    ] {
        assert_eq!(
            parse_identify(&format!("Camera: {camera}\n"))
                .camera
                .as_deref(),
            Some(expected)
        );
    }
}

fn ascii(value: &str) -> Value {
    Value::Ascii(vec![value.as_bytes().to_vec()])
}

fn rationals(values: &[(u32, u32)]) -> Value {
    Value::Rational(
        values
            .iter()
            .map(|(num, denom)| exif::Rational {
                num: *num,
                denom: *denom,
            })
            .collect(),
    )
}

fn encode(fields: Vec<(Tag, Value)>, little_endian: bool) -> Vec<u8> {
    let fields: Vec<_> = fields
        .into_iter()
        .map(|(tag, value)| exif::Field {
            tag,
            ifd_num: In::PRIMARY,
            value,
        })
        .collect();
    let mut writer = exif::experimental::Writer::new();
    for field in &fields {
        writer.push_field(field);
    }
    let mut output = std::io::Cursor::new(Vec::new());
    writer
        .write(&mut output, little_endian)
        .expect("encode EXIF fixture");
    output.into_inner()
}

#[test]
fn exif_extraction_reads_capture_tags_without_pixels_in_both_byte_orders() {
    for little_endian in [true, false] {
        let bytes = encode(
            vec![
                (Tag::ImageWidth, Value::Long(vec![6000])),
                (Tag::ImageLength, Value::Long(vec![4000])),
                (Tag::Orientation, Value::Short(vec![6])),
                (Tag::Make, ascii("Canon")),
                (Tag::Model, ascii("Canon EOS R5")),
                (Tag::LensModel, ascii("RF 50mm F1.8")),
                (Tag::FocalLength, rationals(&[(50, 1)])),
                (Tag::ExposureTime, rationals(&[(1, 125)])),
                (Tag::PhotographicSensitivity, Value::Short(vec![400])),
                (Tag::GPSLatitude, rationals(&[(37, 1), (30, 1), (0, 1)])),
                (Tag::GPSLatitudeRef, ascii("S")),
                (Tag::GPSLongitude, rationals(&[(122, 1), (15, 1), (0, 1)])),
                (Tag::GPSLongitudeRef, ascii("W")),
            ],
            little_endian,
        );
        let directory = tempfile::tempdir().expect("fixture directory");
        let path = directory.path().join("metadata.dng");
        std::fs::write(&path, bytes).expect("write EXIF fixture");
        let metadata = RawMetadata::from_json(&read(&path).expect("inspect RAW fixture"))
            .expect("validate RAW metadata");
        assert_eq!(metadata.dimensions, Some((4000, 6000)));
        assert_eq!(metadata.camera.as_deref(), Some("Canon EOS R5"));
        assert_eq!(metadata.lens.as_deref(), Some("RF 50mm F1.8"));
        assert_eq!(metadata.focal_length, Some(50.0));
        assert_eq!(metadata.shutter_speed, Some(0.008));
        assert_eq!(metadata.iso, Some(400.0));
        assert_eq!(metadata.gps, Some((-37.5, -122.25)));
    }
}

#[test]
fn gps_requires_valid_dms_and_matching_hemisphere_including_zero() {
    for (values, reference, latitude, expected) in [
        ([(0, 1), (0, 1), (0, 1)], "N", true, Some(0.0)),
        ([(12, 1), (30, 1), (0, 1)], "N", true, Some(12.5)),
        ([(180, 1), (0, 1), (0, 1)], "E", false, Some(180.0)),
        ([(0, 1), (0, 1), (0, 1)], "", true, None),
        ([(1, 0), (0, 1), (0, 1)], "N", true, None),
        ([(1, 1), (60, 1), (0, 1)], "N", true, None),
        ([(90, 1), (0, 1), (1, 1)], "N", true, None),
        ([(180, 1), (0, 1), (1, 1)], "E", false, None),
        ([(0, 1), (0, 1), (0, 1)], "E", true, None),
    ] {
        assert_eq!(
            coordinate(&rationals(&values), reference, latitude),
            expected
        );
    }
    assert_eq!(coordinate(&rationals(&[(1, 1), (2, 1)]), "N", true), None);
}

#[test]
fn exif_dimensions_exclude_thumbnails_and_prefer_dng_crop_over_sensor_size() {
    let mut fields = vec![
        (Tag::ImageWidth, Value::Long(vec![160])),
        (Tag::ImageLength, Value::Long(vec![120])),
        (NEW_SUBFILE_TYPE, Value::Long(vec![1])),
        (Tag::PhotographicSensitivity, Value::Short(vec![65535])),
    ];
    let parse = |fields| {
        parse_exif(
            &exif::Reader::new()
                .read_raw(encode(fields, true))
                .expect("parse EXIF fields"),
        )
    };
    let metadata = parse(fields.clone());
    assert_eq!(metadata.dimensions, None);
    assert_eq!(metadata.iso, None);
    assert_eq!(metadata.gps, None);
    fields.push((Tag::PixelXDimension, Value::Long(vec![6032])));
    fields.push((Tag::PixelYDimension, Value::Long(vec![4032])));
    assert_eq!(parse(fields.clone()).dimensions, Some((6032, 4032)));
    fields.push((DEFAULT_CROP_SIZE, rationals(&[(6000, 1), (4000, 1)])));
    fields.push((Tag::ISOSpeed, Value::Long(vec![102400])));
    let metadata = parse(fields);
    assert_eq!(metadata.dimensions, Some((6000, 4000)));
    assert_eq!(metadata.iso, Some(102400.0));
}

#[test]
fn unreadable_raw_keeps_all_fields_unavailable() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("broken.nef");
    std::fs::write(&path, b"not RAW").expect("write broken RAW fixture");
    assert_eq!(
        RawMetadata::from_json(&read(&path).expect("inspect broken RAW fixture"))
            .expect("validate unavailable metadata"),
        RawMetadata::default()
    );
}
