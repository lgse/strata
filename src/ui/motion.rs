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
    let progress = progress.clamp(0.0, 1.0);
    let mut lower = 0.0;
    let mut upper = 1.0;
    for _ in 0..16 {
        let time = (lower + upper) / 2.0;
        if cubic_coordinate(time, 0.16, 0.3) < progress {
            lower = time;
        } else {
            upper = time;
        }
    }
    cubic_coordinate((lower + upper) / 2.0, 1.0, 1.0)
}

fn cubic_coordinate(time: f64, first: f64, second: f64) -> f64 {
    let inverse = 1.0 - time;
    3.0 * inverse * inverse * time * first
        + 3.0 * inverse * time * time * second
        + time * time * time
}

#[cfg(test)]
mod tests;
