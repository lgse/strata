// SPDX-License-Identifier: MIT

//! Fades for layers too large to animate through CSS. Changing a widget's own opacity
//! repaints from its cached render nodes; a CSS transition re-styles the layer's tree.

use std::cell::Cell;

use gtk::{glib, prelude::*};

use crate::ui::motion;

const DURATION_US: i64 = 200_000;
/// Fades start a frame in so the first painted frame already shows the change.
const FIRST_FRAME_US: i64 = 16_000;
/// A frame stalled by a first render must not use up the fade.
const LONGEST_FRAME_US: i64 = 33_000;

fn progress(elapsed_us: i64) -> f64 {
    (elapsed_us as f64 / DURATION_US as f64).min(1.0)
}

/// Calls `step` with the fade's progress on each frame until it reaches 1 or `step` stops it.
fn on_each_frame(
    widget: &gtk::Widget,
    step: impl Fn(&gtk::Widget, f64) -> glib::ControlFlow + 'static,
) {
    let elapsed = Cell::new(FIRST_FRAME_US);
    let previous = Cell::new(None);
    widget.add_tick_callback(move |widget, clock| {
        let now = clock.frame_time();
        if let Some(previous) = previous.replace(Some(now)) {
            elapsed.set(elapsed.get() + (now - previous).min(LONGEST_FRAME_US));
        }
        step(widget, progress(elapsed.get()))
    });
}

/// The layer is never fully transparent, which would read as already closed.
pub(in crate::ui) fn fade_in(layer: &gtk::Widget) {
    if !motion::animations_enabled() || !layer.is_mapped() {
        layer.set_opacity(1.0);
        return;
    }
    layer.set_opacity(motion::emphasized_deceleration(progress(FIRST_FRAME_US)));
    on_each_frame(layer, |layer, progress| {
        if layer.has_css_class("dismissing") {
            return glib::ControlFlow::Break;
        }
        layer.set_opacity(motion::emphasized_deceleration(progress));
        if progress < 1.0 {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

/// Runs `on_done` once the layer is transparent. Callers restore its opacity after hiding it.
pub(in crate::ui) fn fade_out(layer: &gtk::Widget, on_done: impl FnOnce() + 'static) {
    if !motion::animations_enabled() || !layer.is_mapped() {
        glib::idle_add_local_once(on_done);
        return;
    }
    let from = layer.opacity();
    let on_done = Cell::new(Some(on_done));
    on_each_frame(layer, move |layer, progress| {
        layer.set_opacity(from * (1.0 - motion::emphasized_acceleration(progress)));
        if progress < 1.0 {
            return glib::ControlFlow::Continue;
        }
        if let Some(on_done) = on_done.take() {
            on_done();
        }
        glib::ControlFlow::Break
    });
}
