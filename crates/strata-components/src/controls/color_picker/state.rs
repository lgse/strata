//! The picker's model: the colour as HSV, the drag in progress, the parts' laid-out sizes, the
//! hex field's text and the popover's opening colour. Pure logic; `view/` wires it to Rune.

use rune::TextEdit;

use super::value::{Hsv, Srgba};
use crate::controls::slider::Range;

/// What a change to the colour is, as C20a's `SliderChange` says it for a slider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChange {
    /// A drag moved it: apply it (a live preview), but don't save yet.
    Live,
    /// The colour settled: a drag released, a key, a hex commit, a swatch, an assistive
    /// action, or the popover closing on a press outside. Save it.
    Commit,
    /// A drag was cancelled, or the popover closed on Escape: the colour is back where the
    /// drag started or the popover opened. Apply it and save it. After a drag that is the
    /// colour last saved (its moves were only `Live`); in the popover, keys, swatches, hex
    /// commits and releases sent `Commit`s before Escape, so the opening colour it reports
    /// replaces what they saved.
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

    /// With an alpha strip, and eight hex digits for a translucent colour. Without it (Strata's
    /// folder colours and theme editor, as GTK's `use_alpha(false)`) the colour stays opaque.
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

    /// Starts at `value` with its alpha (with the strip on), as [`ColorPickerState::new`] and
    /// [`ColorPickerState::alpha`] then [`ColorPickerState::set_value`] would.
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

    /// The colour the host holds (another window changed it, a preference loaded). A grey
    /// keeps the hue the picker had, so the plane doesn't jump to red. The hex text follows,
    /// replacing any text being typed. A drag in progress keeps going from here.
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

    /// The hex field's text, for C21's `FieldMenu`.
    pub fn edit(&self) -> &TextEdit {
        &self.edit
    }

    /// The hex field's text, for C21's `FieldMenu`. Call [`ColorPickerState::edited`] after
    /// changing it.
    pub fn edit_mut(&mut self) -> &mut TextEdit {
        &mut self.edit
    }

    /// The text changed outside the field's own typing (the field menu's Paste or Delete):
    /// check it again.
    pub fn edited(&mut self) {
        self.invalid = self.parse().is_none();
    }

    /// While the field's menu has the keyboard (from its open until its close), losing the
    /// keyboard to it commits nothing.
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

    /// A press at `(x, y)` in `part`'s box, whose drawn area is inset `margin` on each side
    /// (half a cursor, so the cursor stays inside the box): the cursor jumps there, as GTK's
    /// do. True when the colour moved.
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

    /// The drag ended: a commit when the colour moved, a cancel (back to the start, hue and
    /// all) when it was cancelled after moving.
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

    /// Saturation `ds` and value `dv` whole percents along (the plane's arrows), from the
    /// nearest whole percent (C20a's `Range::stepped`), holding at the ends. From an off-grid
    /// start (a drag, a swatch) Right then Left lands on a whole percent, not back on the start.
    /// True when the colour moved.
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

    /// A strip `steps` steps along: a degree of hue, a percent of alpha. True when the colour
    /// moved.
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

    /// A swatch: the colour becomes `c` (opaque without alpha), keeping the hue for a grey.
    /// True when the colour moved.
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

    /// Commits the typed text: a colour moves the picker there, anything else reverts it. The
    /// text shows the colour either way. True when the colour moved.
    pub(super) fn commit_text(&mut self) -> bool {
        let moved = match self.parse() {
            Some(c) => self.pick(c),
            None => false,
        };
        self.show_value();
        moved
    }

    /// Commits typed text the field still holds, before another part changes the colour: as
    /// GTK applies its entry when the entry loses focus, ahead of the press that took it. True
    /// when the colour moved.
    pub(super) fn commit_pending(&mut self) -> bool {
        self.dirty() && self.commit_text()
    }

    /// Escape in the field: typed text reverts to the colour's (true), else nothing does and
    /// Escape goes on (false).
    pub(super) fn revert_text(&mut self) -> bool {
        if !self.dirty() {
            return false;
        }
        self.show_value();
        true
    }

    /// Notices the field losing the keyboard, from the host's poll (no focus-change hook,
    /// upstream #106), as C20a's NumberField does. On the first frame without it, after having
    /// had it, typed text commits. Losing it to the field's own menu commits nothing. `Some`
    /// when it committed, holding whether the colour moved.
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

    /// Opens the popover, as a press on its button does ([`super::ColorPicker::button`]),
    /// remembering the colour Escape goes back to. Already open, it keeps the first.
    pub fn open(&mut self) {
        if self.opened.is_none() {
            self.opened = Some(self.hsv);
        }
    }

    /// The popover closes. `cancel` (Escape) puts the opening colour back. Returns whether the
    /// colour now differs from the opening one, and whether closing itself moved it (typed
    /// text committed, or the cancel).
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
