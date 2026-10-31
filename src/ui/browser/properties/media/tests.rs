// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn media_rows_show_units_and_omit_unavailable_properties() {
    let metadata = MediaMetadata::from_json(br#"{
        "streams":[
            {"codec_type":"video", "width":1920, "height":1080, "codec_name":"h264", "avg_frame_rate":"24000/1001"},
            {"codec_type":"audio", "codec_name":"aac", "sample_rate":"44100", "channels":2}
        ],
        "format":{"duration":"3661.2", "bit_rate":"2500000"}
    }"#, false).expect("parse source media metadata");
    assert_eq!(
        rows(&metadata),
        vec![
            ("RESOLUTION", "1920 × 1080 pixels".into()),
            ("DURATION", "1:01:01".into()),
            ("BITRATE", "2.50 Mb/s".into()),
            ("VIDEO CODEC", "h264".into()),
            ("FRAME RATE", "23.98 fps".into()),
            ("AUDIO CODEC", "aac".into()),
            ("SAMPLE RATE", "44.1 kHz".into()),
            ("CHANNELS", "2 (Stereo)".into()),
        ]
    );
    assert!(rows(&MediaMetadata::default()).is_empty());

    let hdr = MediaMetadata::from_json(
        br#"{
        "streams":[
            {"codec_type":"video", "codec_name":"hevc", "color_transfer":"smpte2084"},
            {"codec_type":"subtitle", "tags":{"language":"eng"}},
            {"codec_type":"subtitle"}
        ],
        "chapters":[{"start_time":"0", "end_time":"5"}],
        "format":{}
    }"#,
        false,
    )
    .expect("parse hdr metadata");
    assert_eq!(
        rows(&hdr),
        vec![
            ("VIDEO CODEC", "hevc".into()),
            ("HDR", "HDR10".into()),
            ("SUBTITLES", "2 (eng)".into()),
            ("CHAPTERS", "1".into()),
        ]
    );
}
