// SPDX-License-Identifier: MIT

//! Bundled, editable recipes. Applying a recipe never executes it.

use super::{ActionRuntime, ExecutionMode};

const CONTEXT_API: &str = include_str!("../../../data/actions/context-api.txt");

pub(crate) struct ActionExample {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub requirements: &'static str,
    pub inputs: &'static str,
    pub extensions: &'static [&'static str],
    pub folders: bool,
    pub mode: ExecutionMode,
    source: ExampleSource,
}

enum ExampleSource {
    Python(&'static str),
    Bash(&'static str),
    Rename(&'static str),
}

impl ActionExample {
    pub fn runtime(&self) -> ActionRuntime {
        match self.source {
            ExampleSource::Bash(_) => ActionRuntime::Bash,
            _ => ActionRuntime::Python,
        }
    }

    pub fn script(&self) -> String {
        let body = match self.source {
            ExampleSource::Bash(source) => {
                return format!("#!/usr/bin/env bash\n# {}\n\n{source}", self.description);
            }
            ExampleSource::Python(source) => source.to_owned(),
            ExampleSource::Rename(source) => format!(
                "{source}\n{}",
                include_str!("../../../data/actions/rename-engine.py")
            ),
        };
        format!(
            "#!/usr/bin/env python3\n\"\"\"{}\n\n{}\"\"\"\n\n{}",
            self.description, CONTEXT_API, body,
        )
    }
}

pub(crate) const ACTION_EXAMPLES: &[ActionExample] = &[
    ActionExample {
        name: "Log selected paths",
        category: "Files",
        description: "A documented starter that logs paths and reports progress without changing files.",
        requirements: "Python 3 only; no extra packages",
        inputs: "Files and folders · Whole selection",
        extensions: &[],
        folders: true,
        mode: ExecutionMode::WholeSelection,
        source: ExampleSource::Python(include_str!("../../../data/actions/examples/starter.py")),
    },
    ActionExample {
        name: "Batch rename",
        category: "Files",
        description: "Rename selected files with an editable naming function and numbered pattern. Existing names are never overwritten.",
        requirements: "Python 3 + Linux renameat2 support; no extra packages",
        inputs: "Regular files · Whole selection",
        extensions: &[],
        folders: false,
        mode: ExecutionMode::WholeSelection,
        source: ExampleSource::Rename(include_str!("../../../data/actions/examples/rename.py")),
    },
    ActionExample {
        name: "Lowercase file names",
        category: "Files",
        description: "Rename regular files to lowercase with dashes instead of spaces. Existing names are never overwritten.",
        requirements: "Python 3 + Linux renameat2 support; no extra packages",
        inputs: "Regular files · Whole selection",
        extensions: &[],
        folders: false,
        mode: ExecutionMode::WholeSelection,
        source: ExampleSource::Rename(include_str!("../../../data/actions/examples/lowercase.py")),
    },
    ActionExample {
        name: "Count lines",
        category: "Files",
        description: "Log per-file and total line counts for selected text files without modifying them.",
        requirements: "Bash + wc (coreutils); no Python required",
        inputs: "Text files · Whole selection",
        extensions: &[
            "txt", "md", "csv", "log", "json", "toml", "yaml", "yml", "py", "sh", "rs",
        ],
        folders: false,
        mode: ExecutionMode::WholeSelection,
        source: ExampleSource::Bash(include_str!(
            "../../../data/actions/examples/count-lines.sh"
        )),
    },
    ActionExample {
        name: "SHA-256 checksums",
        category: "Files",
        description: "Write a .sha256 checksum beside each selected file. Existing checksum files are never replaced.",
        requirements: "Python 3 only; no extra packages",
        inputs: "Files · Per item",
        extensions: &[],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: ExampleSource::Python(include_str!("../../../data/actions/examples/checksums.py")),
    },
    ActionExample {
        name: "Convert images to WebP",
        category: "Media",
        description: "Convert the first frame to WebP at quality 85 in a new folder beside each original.",
        requirements: "Python 3 + ImageMagick (magick or convert)",
        inputs: "Image files · Per item",
        extensions: &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: ExampleSource::Python(include_str!("../../../data/actions/examples/webp.py")),
    },
    ActionExample {
        name: "Resize images to 1024px",
        category: "Media",
        description: "Fit the first frame within 1024 x 1024 without enlarging it; save a PNG in a new folder.",
        requirements: "Python 3 + ImageMagick (magick or convert)",
        inputs: "Image files · Per item",
        extensions: &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: ExampleSource::Python(include_str!("../../../data/actions/examples/resize.py")),
    },
    ActionExample {
        name: "Strip EXIF metadata",
        category: "Media",
        description: "Remove writable metadata from photos in place, keeping each original in a new private backup folder.",
        requirements: "Bash + ExifTool + coreutils; no Python required",
        inputs: "Image files · Per item",
        extensions: &["jpg", "jpeg", "png", "tif", "tiff", "webp"],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: ExampleSource::Bash(include_str!("../../../data/actions/examples/strip-exif.sh")),
    },
    ActionExample {
        name: "Convert videos to MP4",
        category: "Media",
        description: "Create H.264/AAC MP4 copies in new folders beside the originals. Originals are kept.",
        requirements: "Python 3 + FFmpeg with libx264 and AAC encoders",
        inputs: "Video files · Per item",
        extensions: &["mp4", "mkv", "mov", "webm", "avi", "m4v"],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: ExampleSource::Python(include_str!("../../../data/actions/examples/mp4.py")),
    },
    ActionExample {
        name: "Extract MP3 audio",
        category: "Media",
        description: "Save the first audio stream as MP3 in a new folder beside each original.",
        requirements: "Python 3 + FFmpeg with the libmp3lame encoder",
        inputs: "Audio and video files · Per item",
        extensions: &[
            "mp4", "mkv", "mov", "webm", "avi", "m4v", "mp3", "wav", "flac", "ogg", "m4a", "aac",
        ],
        folders: false,
        mode: ExecutionMode::PerItem,
        source: ExampleSource::Python(include_str!("../../../data/actions/examples/mp3.py")),
    },
];
