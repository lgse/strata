// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn track_caption_prefers_tags_and_falls_back_to_the_folder() {
    let folder = TrackPosition {
        position: 7,
        count: 9,
        results: false,
    };
    let tagged = AudioTags {
        track: Some(3),
        track_total: Some(12),
        ..AudioTags::default()
    };
    assert_eq!(
        track_caption(&tagged, Some(folder)).as_deref(),
        Some("Track 3 of 12")
    );
    let numbered = AudioTags {
        track: Some(3),
        ..AudioTags::default()
    };
    assert_eq!(
        track_caption(&numbered, Some(folder)).as_deref(),
        Some("Track 3")
    );
    assert_eq!(
        track_caption(&AudioTags::default(), Some(folder)).as_deref(),
        Some("7 of 9 in folder")
    );
    assert_eq!(track_caption(&AudioTags::default(), None), None);
    assert_eq!(
        track_caption(
            &AudioTags::default(),
            Some(TrackPosition {
                results: true,
                ..folder
            })
        )
        .as_deref(),
        Some("7 of 9 in results")
    );
}

#[test]
fn clock_shows_hours_only_for_long_audio() {
    assert_eq!(clock(-1), "0:00");
    assert_eq!(clock(61_900_000), "1:01");
    assert_eq!(clock(3_725_000_000), "1:02:05");
}
