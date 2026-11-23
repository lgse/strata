// SPDX-License-Identifier: MIT

use rune::TextEdit;

use crate::controls::slider::{Range, wheel_notches};

#[derive(Clone, Debug)]
pub struct NumberFieldState {
    range: Range,
    suffix: String,
    value: f64,
    /// The field's text. Rune sets `edit.focused` while it has the keyboard.
    pub(super) edit: TextEdit,
    invalid: bool,
    /// The field's menu has the keyboard: losing it to the menu commits nothing.
    held: bool,
    /// The field has had the keyboard since it last committed on a blur.
    seen: bool,
    wheel: f32,
}

impl NumberFieldState {
    /// A field over `range` at `value` (clamped).
    pub fn new(value: f64, range: Range) -> NumberFieldState {
        let value = range.clamp(value);
        NumberFieldState {
            range,
            suffix: String::new(),
            value,
            edit: TextEdit::new(range.format(value)),
            invalid: false,
            held: false,
            seen: false,
            wheel: 0.0,
        }
    }

    pub fn suffix(mut self, suffix: impl Into<String>) -> NumberFieldState {
        self.suffix = suffix.into();
        self
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn range(&self) -> Range {
        self.range
    }

    pub fn unit(&self) -> &str {
        &self.suffix
    }

    /// The text doesn't read as a value in the range.
    pub fn invalid(&self) -> bool {
        self.invalid
    }

    /// The value as it shows: the number and its unit ("60%").
    pub fn value_text(&self) -> String {
        format!("{}{}", self.range.format(self.value), self.suffix)
    }

    pub fn set_value(&mut self, value: f64) {
        self.value = self.range.clamp(value);
        self.show_value();
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

    pub(super) fn parse(&self) -> Option<f64> {
        let mut t = self.edit.text.trim();
        let unit = self.suffix.trim();
        if !unit.is_empty()
            && let Some(rest) = t.strip_suffix(unit)
        {
            t = rest.trim_end();
        }
        let digits = t.strip_prefix(['-', '+']).unwrap_or(t);
        let negative = t.starts_with('-');
        let points = digits.chars().filter(|&c| c == '.').count();
        let numeric = !digits.is_empty()
            && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
            && digits.chars().any(|c| c.is_ascii_digit())
            && points <= usize::from(self.range.decimal_places() > 0);
        if !numeric || (negative && self.range.min() >= 0.0) {
            return None;
        }
        let v = self.range.round(t.parse::<f64>().ok()?);
        (v >= self.range.min() && v <= self.range.max()).then_some(v)
    }

    pub(super) fn commit_text(&mut self) -> Option<f64> {
        let moved = match self.parse() {
            Some(v) if v != self.value => {
                self.value = v;
                true
            }
            _ => false,
        };
        self.show_value();
        moved.then_some(self.value)
    }

    pub(super) fn step(&mut self, steps: i32) -> Option<f64> {
        let before = self.value;
        self.commit_text();
        self.value = self.range.stepped(self.value, steps);
        self.show_value();
        (self.value != before).then_some(self.value)
    }

    /// The typed text committed, then the minimum (`false`) or the maximum.
    pub(super) fn jump_to_end(&mut self, max: bool) -> Option<f64> {
        let before = self.value;
        self.commit_text();
        self.value = if max {
            self.range.max()
        } else {
            self.range.min()
        };
        self.show_value();
        (self.value != before).then_some(self.value)
    }

    pub(super) fn wheel(&mut self, dy: f32, precise: bool) -> Option<f64> {
        let notches = wheel_notches(&mut self.wheel, dy, precise);
        if notches == 0.0 {
            return None;
        }
        self.step(notches as i32)
    }

    pub(super) fn blurred(&mut self) -> Option<Option<f64>> {
        if self.edit.focused {
            self.seen = true;
            return None;
        }
        if !self.seen || self.held {
            return None;
        }
        self.seen = false;
        let text = self.edit.text.clone();
        let moved = self.commit_text();
        (moved.is_some() || text != self.edit.text).then_some(moved)
    }

    /// The text shows the value, with the caret at its end and nothing marked invalid.
    fn show_value(&mut self) {
        let text = self.range.format(self.value);
        if self.edit.text != text {
            self.edit.replace_text(text);
        }
        self.invalid = false;
    }
}
