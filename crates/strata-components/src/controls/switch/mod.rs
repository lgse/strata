// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Cursor, El, Tx, el};
use strata_kit::look::Look;

#[cfg(test)]
mod tests;

/// Track outer width (44 pt inner + 1 pt border each side).
pub const TRACK_WIDTH: f32 = 46.0;
/// Track outer height (22 pt inner + 1 pt border each side).
pub const TRACK_HEIGHT: f32 = 24.0;
/// Square knob edge length.
pub const KNOB_SIZE: f32 = 20.0;
/// Distance from the track's outer edge to the knob (1 pt border + 1 pt margin).
pub const KNOB_INSET: f32 = 2.0;
pub const TRACK_BASE_RADIUS: f32 = 24.0;
pub const KNOB_BASE_RADIUS: f32 = 20.0;

const CHECKED_TRACK_ALPHA: f32 = 0.25;
const CHECKED_TRACK_HOVER_ALPHA: f32 = 0.32;
const CHECKED_BORDER_ALPHA: f32 = 0.65;
const DISABLED_OPACITY: f32 = 0.45;

type ToggleFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;

pub fn radii(look: &Look) -> (f32, f32) {
    (
        look.round(TRACK_BASE_RADIUS).min(TRACK_HEIGHT / 2.0),
        look.round(KNOB_BASE_RADIUS).min(KNOB_SIZE / 2.0),
    )
}

/// A toggle switch styled from Strata's [`Look`].
pub struct Switch<S> {
    checked: bool,
    label: Option<String>,
    description: Option<String>,
    disabled: bool,
    on_toggle: Option<ToggleFn<S>>,
}

impl<S: 'static> Switch<S> {
    pub fn new(checked: bool) -> Self {
        Self {
            checked,
            label: None,
            description: None,
            disabled: false,
            on_toggle: None,
        }
    }

    /// The switch's accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The switch's accessible description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// A switch that ignores input and cannot take focus.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called when the switch is pressed by pointer or keyboard.
    pub fn on_toggle(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(move |s, _| f(s)));
        self
    }

    /// Called with the change's `Tx` when the switch is pressed by pointer or keyboard.
    pub fn on_toggle_tx(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(f));
        self
    }

    pub fn build(self, look: &Look) -> El {
        let kit = &look.kit;
        let c = &kit.c;
        let (track_radius, knob_radius) = radii(look);
        let (track_fill, track_border, track_hover, knob_fill) = if self.checked {
            (
                kit.alpha(c.bg_accent, CHECKED_TRACK_ALPHA),
                kit.alpha(c.bg_accent, CHECKED_BORDER_ALPHA),
                kit.alpha(c.bg_accent, CHECKED_TRACK_HOVER_ALPHA),
                c.bg_accent,
            )
        } else {
            (look.fill.muted, c.border_strong, c.hover, c.text_muted)
        };
        let knob_x = if self.checked {
            TRACK_WIDTH - KNOB_INSET - KNOB_SIZE
        } else {
            KNOB_INSET
        };

        let mut knob = el()
            .key("thumb")
            .size(KNOB_SIZE, KNOB_SIZE)
            .absolute(knob_x, KNOB_INSET)
            .radius(knob_radius)
            .fill(knob_fill)
            .a11y_hidden();
        if look.animations {
            knob = knob.motion(kit.motion.control);
        }

        let mut track = el()
            .role("switch")
            .checked(self.checked)
            .size(TRACK_WIDTH, TRACK_HEIGHT)
            .radius(track_radius)
            .fill(track_fill)
            .border(1.0, track_border)
            .focus_ring(2.0, 2.0, c.border_focus)
            .focusable(!self.disabled)
            .disabled(self.disabled)
            .child(knob);

        if look.animations {
            track = track.motion(kit.motion.control);
        }
        if let Some(label) = self.label {
            track = track.label(label);
        }
        if let Some(description) = self.description.filter(|d| !d.is_empty()) {
            track = track.description(description);
        }

        if self.disabled {
            track = track.layer_opacity(DISABLED_OPACITY);
        } else {
            track = track.cursor(Cursor::Pointer).hover(track_hover);
            if let Some(on_toggle) = self.on_toggle {
                track = track.on_press_tx(move |s: &mut S, tx: &mut Tx| on_toggle(s, tx));
            }
        }

        track
    }
}
