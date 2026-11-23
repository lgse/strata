// SPDX-License-Identifier: MIT

use rune::TextEdit;

use super::value::{Hsv, Srgba};
use crate::controls::slider::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChange {
    /// A drag moved it: apply it (a live preview), but don't save yet.
    Live,
    Commit,
    Cancel,
}

/// A draggable part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Plane,
    Hue,
    Alpha,
}

/// Hue in whole degrees, a step a degree and a page ten (Rune's `on_step` page).
pub(super) fn hue_range() -> Range {
    Range::new(0.0, 360.0, 1.0)
}

/// Alpha, saturation and value in whole percents.
pub(super) fn percent_range() -> Range {
    Range::new(0.0, 100.0, 1.0)
}

/// A fraction `0..=1` as a whole percent.
pub(super) fn percent(x: f32) -> u32 {
    (x.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// A picker's state, held by the host and reached through accessors.
#[derive(Clone, Debug)]
pub struct ColorPickerState {
    hsv: Hsv,
    alpha: bool,
    /// The colour a drag started from.
    grab: Option<(Part, Hsv)>,
    /// The plane's laid-out size and the strips' widths, in points (0 until laid out).
    plane: [f32; 2],
    hue_w: f32,
    alpha_w: f32,
    /// The hex field's text. Rune sets `edit.focused` while it has the keyboard.
    pub(super) edit: TextEdit,
    invalid: bool,
    /// The field's menu has the keyboard: losing it to the menu commits nothing.
    held: bool,
    /// The field has had the keyboard since it last committed on a blur.
    seen: bool,
    /// The colour the popover opened with, while it is open.
    opened: Option<Hsv>,
}

impl ColorPickerState {
    /// A picker at `value`, without alpha: `value`'s alpha is dropped.
    pub fn new(value: Srgba) -> ColorPickerState {
        let value = value.opaque();
        ColorPickerState {
            hsv: Hsv::of(value),
            alpha: false,
            grab: None,
            plane: [0.0; 2],
            hue_w: 0.0,
            alpha_w: 0.0,
            edit: TextEdit::new(value.hex()),
            invalid: false,
            held: false,
            seen: false,
            opened: None,
        }
    }

    pub fn alpha(mut self, on: bool) -> ColorPickerState {
        self.alpha = on;
        if !on {
            self.hsv.a = 1.0;
        }
        self.show_value();
        self
    }

    /// Whether the picker has an alpha strip.
    pub fn has_alpha(&self) -> bool {
        self.alpha
    }

    pub fn with_value(mut self, value: Srgba) -> ColorPickerState {
        self.set_value(value);
        self
    }

    /// The colour.
    pub fn value(&self) -> Srgba {
        let c = self.hsv.to_srgba();
        if self.alpha { c } else { c.opaque() }
    }

    /// The colour as HSV, with the hue and saturation it keeps through greys and black.
    pub fn hsv(&self) -> Hsv {
        self.hsv
    }

    pub fn set_value(&mut self, value: Srgba) {
        let value = if self.alpha { value } else { value.opaque() };
        self.hsv = Hsv::of_keeping(value, self.hsv);
        self.show_value();
    }

    /// While a drag is in progress.
    pub fn dragging(&self) -> bool {
        self.grab.is_some()
    }

    /// The hex field's text isn't a hex colour (typed, not yet committed).
    pub fn invalid(&self) -> bool {
        self.invalid
    }

    pub fn edit(&self) -> &TextEdit {
        &self.edit
    }

    pub fn edit_mut(&mut self) -> &mut TextEdit {
        &mut self.edit
    }

    pub fn edited(&mut self) {
        self.invalid = self.parse().is_none();
    }

    pub fn hold(&mut self, held: bool) {
        self.held = held;
    }

    /// While the popover is open ([`super::ColorPicker::button`]).
    pub fn is_open(&self) -> bool {
        self.opened.is_some()
    }

    // ---- Geometry ----

    pub(super) fn plane_size(&self) -> [f32; 2] {
        self.plane
    }

    pub(super) fn strip_width(&self, part: Part) -> f32 {
        match part {
            Part::Alpha => self.alpha_w,
            _ => self.hue_w,
        }
    }

    /// Records a part's laid-out size; true when it changed.
    pub(super) fn set_size(&mut self, part: Part, w: f32, h: f32) -> bool {
        let slot = match part {
            Part::Plane => {
                let moved = (self.plane[0] - w).abs() > 0.01 || (self.plane[1] - h).abs() > 0.01;
                self.plane = [w, h];
                return moved;
            }
            Part::Hue => &mut self.hue_w,
            Part::Alpha => &mut self.alpha_w,
        };
        let moved = (*slot - w).abs() > 0.01;
        *slot = w;
        moved
    }

    // ---- Pointer ----

    pub(super) fn press(&mut self, part: Part, x: f32, y: f32, margin: f32) -> bool {
        self.grab = Some((part, self.hsv));
        self.move_part(part, x, y, margin)
    }

    /// The pointer moved to `(x, y)` while dragging. True when the colour moved.
    pub(super) fn drag(&mut self, x: f32, y: f32, margin: f32) -> bool {
        let Some((part, _)) = self.grab else {
            return false;
        };
        self.move_part(part, x, y, margin)
    }

    pub(super) fn release(&mut self, cancelled: bool) -> Option<ColorChange> {
        let (_, start) = self.grab.take()?;
        let moved = self.value() != self.value_of(start);
        if cancelled {
            self.hsv = start;
            self.show_value();
            return moved.then_some(ColorChange::Cancel);
        }
        moved.then_some(ColorChange::Commit)
    }

    fn move_part(&mut self, part: Part, x: f32, y: f32, margin: f32) -> bool {
        let before = self.value();
        let along =
            |at: f32, len: f32| ((at - margin) / (len - 2.0 * margin).max(1.0)).clamp(0.0, 1.0);
        match part {
            Part::Plane => {
                let [w, h] = self.plane;
                self.hsv.s = along(x, w);
                self.hsv.v = 1.0 - along(y, h);
            }
            Part::Hue => {
                let r = hue_range();
                self.hsv.h = r.clamp(r.at(f64::from(along(x, self.hue_w)))) as f32;
            }
            Part::Alpha => {
                let r = percent_range();
                self.hsv.a = (r.clamp(r.at(f64::from(along(x, self.alpha_w)))) / 100.0) as f32;
            }
        }
        self.show_value();
        self.value() != before
    }

    // ---- Keys and assistive actions ----

    pub(super) fn plane_step(&mut self, ds: i32, dv: i32) -> bool {
        let before = self.value();
        let r = percent_range();
        let step = |x: f32, d: i32| (r.stepped(f64::from(x) * 100.0, d) / 100.0) as f32;
        if ds != 0 {
            self.hsv.s = step(self.hsv.s, ds);
        }
        if dv != 0 {
            self.hsv.v = step(self.hsv.v, dv);
        }
        self.show_value();
        self.value() != before
    }

    pub(super) fn strip_step(&mut self, part: Part, steps: i32) -> bool {
        let before = self.value();
        match part {
            Part::Hue => self.hsv.h = hue_range().stepped(f64::from(self.hsv.h), steps) as f32,
            Part::Alpha => {
                let a = percent_range().stepped(f64::from(self.hsv.a) * 100.0, steps);
                self.hsv.a = (a / 100.0) as f32;
            }
            Part::Plane => return false,
        }
        self.show_value();
        self.value() != before
    }

    /// A strip to its start (`false`) or its end. True when the colour moved.
    pub(super) fn strip_end(&mut self, part: Part, end: bool) -> bool {
        let before = self.value();
        match part {
            Part::Hue => self.hsv.h = if end { 360.0 } else { 0.0 },
            Part::Alpha => self.hsv.a = if end { 1.0 } else { 0.0 },
            Part::Plane => return false,
        }
        self.show_value();
        self.value() != before
    }

    pub(super) fn pick(&mut self, c: Srgba) -> bool {
        let before = self.value();
        self.set_value(c);
        self.value() != before
    }

    // ---- The hex field ----

    /// The text read as a colour: opaque without alpha (eight digits drop theirs).
    fn parse(&self) -> Option<Srgba> {
        let c = Srgba::parse_hex(&self.edit.text)?;
        Some(if self.alpha { c } else { c.opaque() })
    }

    /// The text differs from the colour's own.
    pub(super) fn dirty(&self) -> bool {
        self.edit.text != self.value().hex()
    }

    pub(super) fn commit_text(&mut self) -> bool {
        let moved = match self.parse() {
            Some(c) => self.pick(c),
            None => false,
        };
        self.show_value();
        moved
    }

    pub(super) fn commit_pending(&mut self) -> bool {
        self.dirty() && self.commit_text()
    }

    pub(super) fn revert_text(&mut self) -> bool {
        if !self.dirty() {
            return false;
        }
        self.show_value();
        true
    }

    pub(super) fn blurred(&mut self) -> Option<bool> {
        if self.edit.focused {
            self.seen = true;
            return None;
        }
        if !self.seen || self.held {
            return None;
        }
        self.seen = false;
        let dirty = self.dirty();
        let moved = self.commit_text();
        (moved || dirty).then_some(moved)
    }

    /// The text shows the colour, caret at its end, nothing marked invalid.
    fn show_value(&mut self) {
        let text = self.value().hex();
        if self.edit.text != text {
            self.edit.replace_text(text);
        }
        self.invalid = false;
    }

    // ---- The popover ----

    pub fn open(&mut self) {
        if self.opened.is_none() {
            self.opened = Some(self.hsv);
        }
    }

    pub(super) fn close(&mut self, cancel: bool) -> (bool, bool) {
        let Some(start) = self.opened.take() else {
            return (false, false);
        };
        let before = self.value();
        self.grab = None;
        self.edit.focused = false;
        self.seen = false;
        self.held = false;
        if cancel {
            self.hsv = start;
            self.show_value();
        } else {
            self.commit_pending();
        }
        let now = self.value();
        (now != self.value_of(start), now != before)
    }

    /// The colour `hsv` gives, opaque without alpha.
    fn value_of(&self, hsv: Hsv) -> Srgba {
        let c = hsv.to_srgba();
        if self.alpha { c } else { c.opaque() }
    }
}
