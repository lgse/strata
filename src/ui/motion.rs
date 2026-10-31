// SPDX-License-Identifier: MIT

use std::cell::Cell;

thread_local! {
    static REDUCE_MOTION: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn set_reduce_motion(reduced: bool) {
    REDUCE_MOTION.set(reduced);
}

pub(super) fn animations_enabled() -> bool {
    !REDUCE_MOTION.get()
        && gtk::Settings::default()
            .map(|settings| settings.is_gtk_enable_animations())
            .unwrap_or(true)
}

pub(super) fn emphasized_deceleration(progress: f64) -> f64 {
    cubic_bezier(progress, (0.16, 1.0), (0.3, 1.0))
}

/// The mirror of [`emphasized_deceleration`] for things leaving the screen:
/// slow to start, fast to finish, so an exit doesn't front-load its motion.
pub(super) fn emphasized_acceleration(progress: f64) -> f64 {
    cubic_bezier(progress, (0.3, 0.0), (0.8, 0.15))
}

fn cubic_bezier(progress: f64, first: (f64, f64), second: (f64, f64)) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    let mut lower = 0.0;
    let mut upper = 1.0;
    for _ in 0..16 {
        let time = (lower + upper) / 2.0;
        if cubic_coordinate(time, first.0, second.0) < progress {
            lower = time;
        } else {
            upper = time;
        }
    }
    cubic_coordinate((lower + upper) / 2.0, first.1, second.1)
}

fn cubic_coordinate(time: f64, first: f64, second: f64) -> f64 {
    let inverse = 1.0 - time;
    3.0 * inverse * inverse * time * first
        + 3.0 * inverse * time * time * second
        + time * time * time
}

#[cfg(test)]
mod tests;
