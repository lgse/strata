// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn raw_metadata_round_trip_preserves_requested_properties() {
    let metadata = RawMetadata {
        dimensions: Some((4000, 6000)),
        camera: Some("Canon EOS R5".into()),
        lens: Some("RF 50mm F1.8".into()),
        focal_length: Some(50.0),
        shutter_speed: Some(1.0 / 125.0),
        iso: Some(400.0),
        gps: Some((-37.5, -122.25)),
    };
    assert_eq!(
        RawMetadata::from_json(&serde_json::to_vec(&metadata).expect("serialize RAW metadata"))
            .expect("parse RAW metadata"),
        metadata
    );
    assert_eq!(
        RawMetadata::from_json(b"{}").expect("parse empty metadata"),
        RawMetadata::default()
    );
    assert_eq!(
        RawMetadata::from_json(br#"{"gps":[0,0],"owner":"private"}"#)
            .expect("accept zero coordinates and ignore unrelated tags")
            .gps,
        Some((0.0, 0.0))
    );
}

#[test]
fn untrusted_raw_metadata_is_bounded_and_sanitized() {
    for input in [
        b"null".as_slice(),
        b"[]",
        b"not json",
        br#"{"gps":[1]}"#,
        br#"{"iso":"400"}"#,
    ] {
        assert!(RawMetadata::from_json(input).is_err());
    }
    assert!(RawMetadata::from_json(&vec![b' '; MAX_METADATA_BYTES as usize + 1]).is_err());
    for input in [
        serde_json::json!({"dimensions":[0,100],"camera":"\u{001b}[31m", "lens":"\u{202e}spoof", "focal_length":-1,"shutter_speed":0,"iso":100_000_001,"gps":[91,0]}),
        serde_json::json!({"dimensions":[1_000_001,100],"camera":" ","lens":"x".repeat(257),"focal_length":100_001,"shutter_speed":604_801,"iso":-1,"gps":[0,181]}),
    ] {
        assert_eq!(
            RawMetadata::from_json(&serde_json::to_vec(&input).expect("serialize invalid fields"))
                .expect("sanitize invalid fields"),
            RawMetadata::default()
        );
    }
}

#[test]
fn raw_extensions_are_case_insensitive_and_exclude_other_images() {
    for name in [
        "photo.NEF",
        "photo.cR3",
        "photo.3fr",
        "photo.x3f",
        "photo.DNG",
    ] {
        assert!(is_raw(Path::new(name)), "{name}");
    }
    for name in ["photo.jpg", "photo.tiff", "photo.dng.jpg", "dng", ""] {
        assert!(!is_raw(Path::new(name)), "{name}");
    }
}
