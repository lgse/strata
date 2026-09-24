// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn raw_rows_format_capture_settings_and_keep_missing_fields() {
    let metadata = RawMetadata {
        dimensions: Some((6000, 4000)),
        camera: Some("Canon EOS R5".into()),
        lens: Some("RF 50mm F1.8".into()),
        focal_length: Some(50.0),
        shutter_speed: Some(1.0 / 250.0),
        iso: Some(400.0),
        gps: Some((-37.5, -122.25)),
    };
    assert_eq!(
        rows(&metadata),
        [
            ("DIMENSIONS", "6000 × 4000 pixels".into()),
            ("CAMERA", "Canon EOS R5".into()),
            ("LENS", "RF 50mm F1.8".into()),
            ("FOCAL LENGTH", "50 mm".into()),
            ("SHUTTER SPEED", "1/250 s".into()),
            ("ISO", "400".into()),
            ("GPS COORDINATES", "-37.500000, -122.250000".into()),
        ]
    );
    assert!(
        rows(&RawMetadata::default())
            .iter()
            .all(|(_, value)| value == "N/A")
    );
    let partial = RawMetadata {
        shutter_speed: Some(2.5),
        gps: Some((0.0, 0.0)),
        ..Default::default()
    };
    let values = rows(&partial);
    assert_eq!(values[4].1, "2.5 s");
    assert_eq!(values[6].1, "0.000000, 0.000000");
    assert_eq!(values[1].1, "N/A");
    for (exposure, expected) in [
        (0.3, "0.3 s"),
        (0.0003, "0.0003 s"),
        (1.0 / 8000.0, "1/8000 s"),
        (1.0 / 3.0, "1/3 s"),
        (1.0, "1 s"),
    ] {
        let metadata = RawMetadata {
            shutter_speed: Some(exposure),
            ..Default::default()
        };
        assert_eq!(rows(&metadata)[4].1, expected);
    }
}
