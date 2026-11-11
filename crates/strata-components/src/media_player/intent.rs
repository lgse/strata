//! What the player asks of its host.

use std::time::Duration;

/// A request from the player. The host applies it: to the clock (play, pause, seek), to its
/// preferences (volume), to its listing (steps), or to the desktop (hand-off).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaIntent {
    Play,
    Pause,
    /// Seek to this position, already clamped to the last frame.
    Seek(Duration),
    /// The volume to save and apply, already snapped where Strata snaps it. A volume stepped
    /// or dragged to 0 comes as `muted` with the volume before it, which unmuting restores.
    SetVolume {
        volume: f32,
        muted: bool,
    },
    /// The previous (-1) or next (1) file of the clip's kind. Which files count, and whether
    /// the next keeps playing, is the host's.
    Step(i8),
    /// Open the file in the default player, at `at` when the position is worth keeping:
    /// `None` within 1 s of either end (Strata's `HANDOFF_MARGIN_US`). Comes after a
    /// [`MediaIntent::Pause`]. Which players take a position is the host's.
    HandOff {
        at: Option<Duration>,
    },
    /// The storyboard is wanted: sent once per clip, when its first frame shows (Strata's
    /// trigger). Which clips get one (none under 4 s, say) is the host's.
    StoryboardWanted,
    /// Decode at this size in device pixels: the frame grew by 8 % or more past what the
    /// decoder makes (Strata's `RESIZE_GROWTH`). Never sent for a shrink, nor for audio. It
    /// comes on the first tick the frame has grown, once per growth; waiting for the resize
    /// to settle before restarting the decoder is the host's (Strata's `RESIZE_DELAY`).
    DecodeSize([u32; 2]),
}
