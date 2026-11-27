// SPDX-License-Identifier: MIT

use rune::{TextEdit, Tx};

/// The unit a field's limit counts in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Measure {
    /// Unicode scalar values.
    #[default]
    Chars,
    /// UTF-8 bytes.
    Bytes,
    /// UTF-16 code units.
    Utf16,
}

impl Measure {
    /// `c`'s size in this unit.
    fn of_char(self, c: char) -> usize {
        match self {
            Measure::Chars => 1,
            Measure::Bytes => c.len_utf8(),
            Measure::Utf16 => c.len_utf16(),
        }
    }

    /// `s`'s size in this unit.
    pub fn of(self, s: &str) -> usize {
        match self {
            Measure::Chars => s.chars().count(),
            Measure::Bytes => s.len(),
            Measure::Utf16 => s.encode_utf16().count(),
        }
    }
}

pub fn fit(text: &str, room: usize, measure: Measure) -> &str {
    let mut used = 0;
    for (i, c) in text.char_indices() {
        used += measure.of_char(c);
        if used > room {
            return &text[..i];
        }
    }
    text
}

/// How much of `room` is left after `text`, in `measure`: the "N chars remaining" count.
pub fn remaining(text: &str, room: usize, measure: Measure) -> usize {
    room.saturating_sub(measure.of(text))
}

pub fn trim_to_fit(edit: &mut TextEdit, room: usize, measure: Measure) {
    let head = fit(&edit.text, room, measure).len();
    if head < edit.text.len() {
        edit.set_selection(head, edit.text.len());
        edit.insert("");
    }
}

pub fn trim_insertion(edit: &mut TextEdit, room: usize, measure: Measure) {
    let mut over = measure.of(&edit.text).saturating_sub(room);
    if over == 0 {
        return;
    }
    let caret = edit.focus.max(edit.anchor).min(edit.text.len());
    debug_assert!(
        edit.text.is_char_boundary(caret),
        "the caret sits on a character boundary"
    );
    let mut start = caret;
    for (i, c) in edit.text[..caret].char_indices().rev() {
        if over == 0 {
            break;
        }
        over = over.saturating_sub(measure.of_char(c));
        start = i;
    }
    edit.set_selection(start, caret);
    edit.insert("");
    if over > 0 {
        // Nothing before the caret was left to cut (a field set from outside): keep the head.
        trim_to_fit(edit, room, measure);
    }
}

pub fn limit_on_change<S: 'static>(
    access: impl Fn(&mut S) -> &mut TextEdit + 'static,
    limit: impl Fn(&S) -> (usize, Measure) + 'static,
) -> impl Fn(&mut S, &mut Tx) + 'static {
    move |s, _tx| {
        let (room, measure) = limit(s);
        trim_insertion(access(s), room, measure);
    }
}
