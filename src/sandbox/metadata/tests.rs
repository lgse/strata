// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn video_metadata_preserves_source_dimensions_and_audio_details() {
    let metadata = MediaMetadata::from_json(
        br#"{
        "streams": [
            {"codec_type":"video", "codec_name":"h264", "width":1920, "height":1080,
             "avg_frame_rate":"30000/1001", "side_data_list":[{"rotation":-90}]},
            {"codec_type":"audio", "codec_name":"aac", "sample_rate":"48000", "channels":2}
        ],
        "format":{"duration":"125.5", "bit_rate":"2500000"}
    }"#,
        false,
    )
    .expect("parse video metadata");
    assert_eq!(metadata.dimensions, Some((1080, 1920)));
    assert_eq!(metadata.duration, Some(125.5));
    assert_eq!(metadata.bitrate, Some(2_500_000.0));
    assert_eq!(metadata.video_codec.as_deref(), Some("h264"));
    assert!((metadata.frame_rate.expect("video frame rate") - 29.97003).abs() < 0.00001);
    assert_eq!(metadata.audio_codec.as_deref(), Some("aac"));
    assert_eq!(metadata.sample_rate, Some(48000.0));
    assert_eq!(metadata.channels, Some(2));
}

#[test]
fn audio_cover_art_is_not_reported_as_video() {
    let metadata = MediaMetadata::from_json(br#"{
        "streams":[
            {"codec_type":"video", "codec_name":"mjpeg", "width":600, "height":600,
             "disposition":{"attached_pic":1}},
            {"codec_type":"audio", "codec_name":"flac", "duration":"20.25", "sample_rate":"44100", "channels":1}
        ]
    }"#, false).expect("parse audio metadata with cover art");
    assert_eq!(metadata.dimensions, None);
    assert_eq!(metadata.video_codec, None);
    assert_eq!(metadata.audio_codec.as_deref(), Some("flac"));
    assert_eq!(metadata.duration, Some(20.25));
}

#[test]
fn missing_container_duration_uses_the_longest_relevant_stream() {
    let mut value = serde_json::json!({
        "streams": [
            {"codec_type": "video", "duration": "2"},
            {"codec_type": "audio", "duration": "5"},
            {"codec_type": "audio", "duration": "10"},
            {"codec_type": "subtitle", "duration": "30"},
            {"codec_type": "video", "duration": "100", "disposition": {"attached_pic": 1}},
            {"codec_type": "audio", "duration": "NaN"}
        ]
    });
    for (container_duration, expected) in [("N/A", 10.0), ("999999999999", 10.0), ("12", 12.0)] {
        value["format"] = serde_json::json!({"duration": container_duration});
        let bytes = serde_json::to_vec(&value).expect("serialize multi-stream fixture");
        assert_eq!(
            MediaMetadata::from_json(&bytes, false)
                .expect("parse stream durations")
                .duration,
            Some(expected)
        );
    }
}

#[test]
fn still_images_do_not_inherit_synthetic_video_timing() {
    let metadata = MediaMetadata::from_json(
        br#"{
        "streams":[{"codec_type":"video", "width":4000, "height":3000,
                    "codec_name":"png", "avg_frame_rate":"25/1"}],
        "format":{"duration":"0.04", "bit_rate":"2000000"}
    }"#,
        true,
    )
    .expect("parse image metadata");
    assert_eq!(
        metadata,
        MediaMetadata {
            dimensions: Some((4000, 3000)),
            ..Default::default()
        }
    );
}

#[test]
fn malformed_or_unknown_values_are_not_presented_as_facts() {
    for bytes in [b"not json".as_slice(), b"{}", b"{\"streams\":null}"] {
        assert!(MediaMetadata::from_json(bytes, false).is_err());
    }
    assert!(MediaMetadata::from_json(&vec![b' '; MAX_METADATA_BYTES as usize + 1], false).is_err());
    let metadata = MediaMetadata::from_json(br#"{
        "streams":[
            {"codec_type":"video", "width":0, "height":1080, "codec_name":"<bad>\n", "avg_frame_rate":"1/0", "r_frame_rate":"0/0"},
            {"codec_type":"audio", "sample_rate":"NaN", "channels":0}
        ],
        "format":{"duration":"inf", "bit_rate":"-1"}
    }"#, false).expect("omit invalid field values");
    assert_eq!(metadata, MediaMetadata::default());
}
