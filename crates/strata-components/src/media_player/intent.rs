// SPDX-License-Identifier: MIT

//! What the player asks of its host.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaIntent {
    Play,
    Pause,
    /// Seek to this position, already clamped to the last frame.
    Seek(Duration),
    SetVolume {
        volume: f32,
        muted: bool,
    },
    Step(i8),
    HandOff {
        at: Option<Duration>,
    },
    StoryboardWanted,
    DecodeSize([u32; 2]),
}
