//! Where a scroll bar's thumb sits and which offset a thumb position means. Pure maths, along
//! one axis, in points.

/// The thumb's start and length on a track `track` points long, for a viewport `shown` points
/// long scrolled `offset` of `max`: as long as the shown part is of the whole, never shorter
/// than `min` (nor longer than the track).
pub fn thumb(offset: f32, max: f32, shown: f32, track: f32, min: f32) -> (f32, f32) {
    let track = track.max(0.0);
    let len = (track * shown / (shown + max).max(1.0)).max(min).min(track);
    let start = if max > 0.0 {
        (track - len) * (offset / max).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (start, len)
}

/// The offset that puts the thumb's start at `start` on the track, clamped to the content.
pub fn offset_at(start: f32, max: f32, shown: f32, track: f32, min: f32) -> f32 {
    let (_, len) = thumb(0.0, max, shown, track, min);
    let travel = track - len;
    if travel <= 0.0 || max <= 0.0 {
        return 0.0;
    }
    (start / travel * max).clamp(0.0, max)
}
