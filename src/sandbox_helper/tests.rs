// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use super::media_preview_command;

#[test]
fn media_preview_transcode_has_bounded_duration_dimensions_and_frame_rate() {
    let command = media_preview_command(Path::new("/input"), Path::new("/output/result.webm"));
    let arguments: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();

    assert!(arguments.windows(2).any(|pair| pair == ["-t", "30"]));
    assert!(arguments.windows(2).any(|pair| pair == ["-fpsmax", "30"]));
    assert!(arguments.windows(2).any(|pair| {
        pair == [
            "-vf",
            "scale=w=1280:h=1280:force_original_aspect_ratio=decrease",
        ]
    }));
}
