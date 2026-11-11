//! What the host knows about the file, as data.

use std::time::Duration;

use rune::Pixels;

use crate::viewers::layout::Position;

pub use super::badges::VideoDetails;

/// Which view the player builds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MediaKind {
    /// A video: the frame, its details and badges, the waveform timeline with storyboard.
    #[default]
    Video,
    /// Audio: art (or the record), tags, the spectrum and the waveform timeline.
    Audio,
    /// Media that is neither: the plain media bar (play, time, a seek slider, volume) under
    /// the frame, as for an animated GIF.
    Plain,
}

/// The file the player shows. Setting a new one ([`MediaState::set_clip`]) clears
/// everything that belonged to the last.
///
/// [`MediaState::set_clip`]: super::MediaState::set_clip
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    pub kind: MediaKind,
    /// The file's name, shown without its extension when there's no title tag.
    pub name: String,
    /// Its place among the listing's files of its kind (the host counts).
    pub position: Option<Position>,
    pub has_previous: bool,
    pub has_next: bool,
}

/// A chapter: where it starts and its title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    pub start: Duration,
    pub title: String,
}

/// An audio file's tags.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
}

/// A playback error, shown inside the view: play is off, previous and next still work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaError {
    pub title: String,
    pub detail: String,
}

/// A video's probe, for its badges.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Details {
    /// The probe hasn't answered: empty pills hold the row.
    #[default]
    Loading,
    /// The probe failed: the row stays empty.
    Failed,
    Ready(VideoDetails),
}

/// A track's art, with an id the host gives each distinct picture: the same id keeps the art
/// in place across tracks, a new one crossfades.
#[derive(Clone, Debug)]
pub struct Art {
    pub id: u64,
    pub pixels: Pixels,
    /// Its size in pixels.
    pub size: [u32; 2],
}
