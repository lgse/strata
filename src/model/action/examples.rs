// SPDX-License-Identifier: MIT

//! Bundled, editable Python recipes. Applying a recipe never executes it.

use super::ExecutionMode;

const CONTEXT_API: &str = include_str!("../../../data/actions/context-api.txt");

pub(crate) struct ActionExample {
    pub name: &'static str,
    pub description: &'static str,
    pub requirements: &'static str,
    pub inputs: &'static str,
    pub extensions: &'static [&'static str],
    pub folders: bool,
    pub mode: ExecutionMode,
    source: &'static str,
}

impl ActionExample {
    pub fn script(&self) -> String {
        format!(
            "#!/usr/bin/env python3\n\"\"\"{}\n\n{}\"\"\"\n\n{}",
            self.description, CONTEXT_API, self.source,
        )
    }
}

pub(crate) const ACTION_EXAMPLES: &[ActionExample] = &[
    ActionExample {
        name: "Log selected paths",
        description: "A documented starter that logs paths and reports progress without changing files.",
        requirements: "Python 3 only; no extra packages",
        inputs: "Files and folders · Whole selection",
        extensions: &[],
        folders: true,
        mode: ExecutionMode::WholeSelection,
        source: include_str!("../../../data/actions/examples/starter.py"),
    },
    ActionExample {
        name: "Convert images to WebP",
        description: "Convert the first frame to WebP at quality 85 in a new folder beside each original.",
        requirements: "Python 3 + ImageMagick (magick or convert)",
        inputs: "Image files · Per item",
        extensions: &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: include_str!("../../../data/actions/examples/webp.py"),
    },
    ActionExample {
        name: "Resize images to 1024px",
        description: "Fit the first frame within 1024 x 1024 without enlarging it; save a PNG in a new folder.",
        requirements: "Python 3 + ImageMagick (magick or convert)",
        inputs: "Image files · Per item",
        extensions: &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: include_str!("../../../data/actions/examples/resize.py"),
    },
    ActionExample {
        name: "Convert videos to MP4",
        description: "Create H.264/AAC MP4 copies in new folders beside the originals. Originals are kept.",
        requirements: "Python 3 + FFmpeg with libx264 and AAC encoders",
        inputs: "Video files · Per item",
        extensions: &["mp4", "mkv", "mov", "webm", "avi", "m4v"],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: include_str!("../../../data/actions/examples/mp4.py"),
    },
    ActionExample {
        name: "Extract MP3 audio",
        description: "Save the first audio stream as MP3 in a new folder beside each original.",
        requirements: "Python 3 + FFmpeg with the libmp3lame encoder",
        inputs: "Audio and video files · Per item",
        extensions: &[
            "mp4", "mkv", "mov", "webm", "avi", "m4v", "mp3", "wav", "flac", "ogg", "m4a", "aac",
        ],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: include_str!("../../../data/actions/examples/mp3.py"),
    },
    ActionExample {
        name: "SHA-256 checksums",
        description: "Write a .sha256 checksum beside each selected file. Existing checksum files are never replaced.",
        requirements: "Python 3 only; no extra packages",
        inputs: "Files · Per item",
        extensions: &[],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: include_str!("../../../data/actions/examples/checksums.py"),
    },
];
