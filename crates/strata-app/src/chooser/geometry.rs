// SPDX-License-Identifier: MIT

pub(crate) const MIN_CHOOSER_WIDTH: i32 = 640;
pub(crate) const MIN_CHOOSER_HEIGHT: i32 = 460;
pub(crate) const MAX_CHOOSER_WIDTH: i32 = 1000;
pub(crate) const MAX_CHOOSER_HEIGHT: i32 = 680;
pub(crate) const FALLBACK_CHOOSER_WIDTH: i32 = 920;
pub(crate) const FALLBACK_CHOOSER_HEIGHT: i32 = 580;

pub(crate) fn chooser_default_dimensions_for_monitor(
    monitor_width: i32,
    monitor_height: i32,
) -> (i32, i32) {
    if monitor_width <= 0 || monitor_height <= 0 {
        return (FALLBACK_CHOOSER_WIDTH, FALLBACK_CHOOSER_HEIGHT);
    }
    let target_width = (monitor_width.saturating_mul(80) / 100)
        .min(monitor_width.saturating_sub(120))
        .clamp(MIN_CHOOSER_WIDTH.min(monitor_width), MAX_CHOOSER_WIDTH);
    let target_height = (monitor_height.saturating_mul(78) / 100)
        .min(monitor_height.saturating_sub(100))
        .clamp(MIN_CHOOSER_HEIGHT.min(monitor_height), MAX_CHOOSER_HEIGHT);
    (target_width, target_height)
}

pub(crate) fn chooser_initial_dimensions(
    monitor: Option<(i32, i32)>,
    parent_size_hint: Option<(i32, i32)>,
) -> (i32, i32) {
    let monitor = monitor.filter(|(width, height)| *width > 0 && *height > 0);
    let parent = parent_size_hint.filter(|(width, height)| *width > 0 && *height > 0);
    let bounds = match (monitor, parent) {
        (Some((mw, mh)), Some((pw, ph))) => Some((mw.min(pw), mh.min(ph))),
        (monitor, parent) => parent.or(monitor),
    };
    bounds.map_or(
        (FALLBACK_CHOOSER_WIDTH, FALLBACK_CHOOSER_HEIGHT),
        |(width, height)| chooser_default_dimensions_for_monitor(width, height),
    )
}
