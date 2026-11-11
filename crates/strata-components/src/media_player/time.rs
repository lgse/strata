//! Strata's media arithmetic and wording: the clock text, the last frame a seek may reach,
//! the volume grid, the decode size and the captions.

use std::time::Duration;

/// Strata decodes at 30 ticks a second, and a seek lands on a tick (`src/media.rs`, `FPS`).
pub const SEEK_TICKS_PER_SECOND: u64 = 30;
/// A volume step: Strata's `MEDIA_VOLUME_STEP`.
pub const VOLUME_STEP: f32 = 0.1;
/// A volume page: Strata's `MEDIA_VOLUME_PAGE`.
pub const VOLUME_PAGE: f32 = 0.2;
/// A seek step: Strata's `MEDIA_SEEK_STEP_US`.
pub const SEEK_STEP: Duration = Duration::from_secs(5);
/// A seek page: Strata's `MEDIA_SEEK_PAGE_US`.
pub const SEEK_PAGE: Duration = Duration::from_secs(30);
/// Hand-off keeps the position only this far from either end: `HANDOFF_MARGIN_US`.
pub const HANDOFF_MARGIN: Duration = Duration::from_secs(1);
/// The largest decode edge, in device pixels: `MediaPreviewSize::MAX_EDGE`.
pub const MAX_DECODE_EDGE: u32 = 1280;
/// The smallest decode edge: `MediaPreviewSize::new`'s clamp.
pub const MIN_DECODE_EDGE: u32 = 16;
/// The growth that restarts decoding: `RESIZE_GROWTH` (`src/ui/media.rs`).
pub const RESIZE_GROWTH: f64 = 0.08;

/// Strata's clock text (`audio.rs`, `clock`): `m:ss`, or `h:mm:ss` from an hour.
pub fn clock(t: Duration) -> String {
    let s = t.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Seeking exactly to the duration would end the stream, so keys and sliders stop on the last
/// tick before it (`media_controls.rs`, `media_last_frame`, over `media::seek_tick`).
pub fn last_frame(duration: Duration) -> Duration {
    let us = duration.as_micros() as u64;
    let tick = us.saturating_sub(1) * SEEK_TICKS_PER_SECOND / 1_000_000;
    Duration::from_micros(tick * 1_000_000 / SEEK_TICKS_PER_SECOND)
}

/// A position one tick long: the smallest move a wheel seek makes (Strata ignores touchpad
/// jumps within the current frame).
pub fn tick() -> Duration {
    Duration::from_micros(1_000_000 / SEEK_TICKS_PER_SECOND)
}

/// Stepped volumes land on the 0.1 grid, so repeated steps reach 0 and mute
/// (`snapped_volume`).
pub fn snapped_volume(volume: f32) -> f32 {
    ((volume / VOLUME_STEP).round() * VOLUME_STEP).clamp(0.0, 1.0)
}

/// The decode size Strata asks for: the frame's points times the scale, each edge between 16
/// and 1280 device pixels (`MediaPreviewSize::for_viewport` and `new`).
pub fn decode_request(frame: [f32; 2], scale: f32) -> [u32; 2] {
    let edge =
        |x: f32| ((x * scale).round().max(0.0) as u32).clamp(MIN_DECODE_EDGE, MAX_DECODE_EDGE);
    [edge(frame[0]), edge(frame[1])]
}

/// Whether decoding should restart for `wanted`, Strata's `needs_resize` (`playback.rs`):
/// `decoded` is the frame the decoder makes, `loaded` the size it was asked for. The frame
/// must grow by [`RESIZE_GROWTH`] or more, and by at least 2 pixels; a shrink never restarts
/// (the frame is drawn smaller instead), and a source smaller than its request (the decoder
/// never enlarges) only restarts when the request grows past it.
pub fn needs_restart(decoded: [u32; 2], loaded: [u32; 2], wanted: [u32; 2]) -> bool {
    let [w, h] = decoded.map(f64::from);
    if w == 0.0 || h == 0.0 {
        return false;
    }
    let mut scale = (f64::from(wanted[0]) / w).min(f64::from(wanted[1]) / h);
    if decoded[0] + 1 < loaded[0] && decoded[1] + 1 < loaded[1] {
        scale = scale.min(1.0);
    }
    scale >= 1.0 + RESIZE_GROWTH && (w * (scale - 1.0) >= 2.0 || h * (scale - 1.0) >= 2.0)
}

/// The position a hand-off keeps (`drawer.rs`, `prepare_handoff`): none within 1 s of either
/// end. With no known duration only the start counts.
pub fn hand_off_at(position: Duration, duration: Option<Duration>) -> Option<Duration> {
    let inside =
        position > HANDOFF_MARGIN && duration.is_none_or(|d| position + HANDOFF_MARGIN < d);
    inside.then_some(position)
}

/// Strata's track caption (`audio.rs`, `track_caption`): the tagged number first, else the
/// file's place in the listing.
pub fn track_caption(
    track: Option<u32>,
    total: Option<u32>,
    position: Option<crate::viewers::layout::Position>,
    labels: &super::labels::Labels,
) -> Option<String> {
    match (track, total, position) {
        (Some(t), Some(n), _) => Some(
            labels
                .track_of
                .replace("{track}", &t.to_string())
                .replace("{total}", &n.to_string()),
        ),
        (Some(t), None, _) => Some(labels.track.replace("{track}", &t.to_string())),
        (None, _, Some(p)) => Some(p.caption()),
        (None, _, None) => None,
    }
}

/// A file name without its extension: the title of a track with no title tag.
pub fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(0) | None => name,
        Some(i) => &name[..i],
    }
}

/// The byline under an audio title (`audio.rs`, `show_tags`): "Artist — Album", or either.
pub fn byline(artist: Option<&str>, album: Option<&str>) -> String {
    match (artist, album) {
        (Some(a), Some(b)) => format!("{a} — {b}"),
        (Some(a), None) => a.to_owned(),
        (None, Some(b)) => b.to_owned(),
        (None, None) => String::new(),
    }
}
