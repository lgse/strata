//! The player's words, in English by default. A host translates them.

/// Names, tooltips and fixed text. Tooltips that name a key take it from the key map, so
/// these hold the words around it: `"Play/Pause ({keys})"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Labels {
    /// The timeline slider's name.
    pub timeline: String,
    /// The play button's name in the audio and video views.
    pub play: String,
    /// The play button's tooltip; `{keys}` is the shortcut. The plain bar's button takes it
    /// as its name too (Strata's).
    pub play_tooltip: String,
    pub previous_video: String,
    pub next_video: String,
    pub previous_audio: String,
    pub next_audio: String,
    /// `{label} ({keys})`: a previous or next button's tooltip.
    pub step_tooltip: String,
    /// The mute button's name and tooltip, `{keys}` the shortcut.
    pub mute: String,
    /// The volume slider's name.
    pub volume: String,
    pub video_frame: String,
    pub spectrum: String,
    pub artwork: String,
    /// `{elapsed} of {duration}`: the timeline's spoken value.
    pub value: String,
    /// The total when the duration isn't known.
    pub unknown_total: String,
    /// Badges: `{rate} fps`, the channel names, `{channels} ch` for other counts, and the
    /// captions badge for one track (`CC`) or more (`CC ×{count}`).
    pub fps: String,
    pub mono: String,
    pub stereo: String,
    pub channels: String,
    pub captions: String,
    pub captions_many: String,
    /// The audio caption from tags: `Track {track} of {total}`, or `Track {track}`.
    pub track_of: String,
    pub track: String,
}

impl Default for Labels {
    fn default() -> Self {
        Labels {
            timeline: "Playback position".into(),
            play: "Play or pause".into(),
            play_tooltip: "Play/Pause ({keys})".into(),
            previous_video: "Previous video".into(),
            next_video: "Next video".into(),
            previous_audio: "Previous audio file".into(),
            next_audio: "Next audio file".into(),
            step_tooltip: "{label} ({keys})".into(),
            mute: "Mute/unmute ({keys})".into(),
            volume: "Volume".into(),
            video_frame: "Video frame".into(),
            spectrum: "Audio spectrum".into(),
            artwork: "Album art".into(),
            value: "{elapsed} of {duration}".into(),
            unknown_total: "--:--".into(),
            fps: "{rate} fps".into(),
            mono: "Mono".into(),
            stereo: "Stereo".into(),
            channels: "{channels} ch".into(),
            captions: "CC".into(),
            captions_many: "CC ×{count}".into(),
            track_of: "Track {track} of {total}".into(),
            track: "Track {track}".into(),
        }
    }
}

/// `template` with `{keys}` replaced, or with " ({keys})" dropped when there are none.
pub(crate) fn with_keys(template: &str, keys: Option<&str>) -> String {
    match keys {
        Some(k) => template.replace("{keys}", k),
        None => template.replace(" ({keys})", "").replace("{keys}", ""),
    }
}
