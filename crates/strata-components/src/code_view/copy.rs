// SPDX-License-Identifier: MIT

//! What copying a selection takes: the text between its two ends, from the model, so rows
//! split from one long line join with no break and lines keep their own breaks. From Rune's
//! report only the ends are taken: the lowest and highest selected bytes over every selected
//! text.
//!
//! A selection reaching more than 10,000 unbuilt rows is cut short there by Rune (R4, C11), so
//! its far end is wrong; Ctrl+A is copied whole from the model instead.

use rune::{SelectedText, TextSelection};

use super::state::{Content, ExpectedSelect};

/// The row of a selected text, from its path (`…/{key}-l{row}/…/t`).
pub(crate) fn locate(key: &str, path: &str) -> Option<usize> {
    let mut row = None;
    let mut last = "";
    for seg in path.split('/') {
        if let Some(n) = seg
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix("-l"))
            .and_then(|n| n.parse().ok())
        {
            row = Some(n);
        }
        last = seg;
    }
    (last == "t").then_some(row?)
}

fn row_bounds(c: &Content, t: &SelectedText) -> Option<(usize, usize)> {
    if !t.is_selected() {
        return None;
    }
    let r = locate(&c.key, t.path()).filter(|&r| r < c.rows.len())?;
    let row = c.rows.row(r);
    let len = row.end - row.start;
    let range = t.range();
    Some((
        row.start + range.start.min(len),
        row.start + range.end.min(len),
    ))
}

/// The text bytes a selection covers, if any of it is the view's.
///
/// Rune hands over selected texts in tree order, so inspecting only the first and last texts
/// belonging to this view gives the exact covered byte span in O(1) without scanning interior
/// rows.
pub(crate) fn span(c: &Content, sel: &TextSelection) -> Option<(usize, usize)> {
    let texts = sel.texts();
    let (first_a, first_b) = texts.iter().find_map(|t| row_bounds(c, t))?;
    let (last_a, last_b) = texts.iter().rev().find_map(|t| row_bounds(c, t))?;
    Some((first_a.min(last_a), first_b.max(last_b)))
}

/// Whether `sel` still matches the Rune selection requested by
/// [`super::state::CodeState::apply_caret`] or [`super::state::CodeState::apply_gutter_lines`]
/// for `c.keyboard_range`.
pub(crate) fn matches_expected(c: &Content, sel: Option<&TextSelection>) -> bool {
    if c.keyboard_range.get().is_none() {
        return false;
    }
    match c.expected_select.get() {
        None => sel.is_none_or(|s| span(c, s).is_none()),
        Some(ExpectedSelect::Row(r, a, b)) => single_selected(sel).is_some_and(|first| {
            locate(&c.key, first.path()) == Some(r) && first.range() == (a..b)
        }),
        Some(ExpectedSelect::GutterSentinel) => single_selected(sel).is_some_and(|first| {
            first
                .key()
                .is_some_and(|k| k.strip_prefix(&c.key) == Some("-gutter-sel"))
                && first.range() == (0..1)
        }),
    }
}

fn single_selected(sel: Option<&TextSelection>) -> Option<&SelectedText> {
    let mut selected = sel?.texts().iter().filter(|t| t.is_selected());
    let first = selected.next()?;
    selected.next().is_none().then_some(first)
}

pub(crate) fn copy(c: &Content, sel: &TextSelection) -> String {
    if c.all.get() && reads_whole(c, sel) {
        return c.rows.text.to_string();
    }
    if matches_expected(c, Some(sel))
        && let Some((a, b)) = c
            .keyboard_range
            .get()
            .filter(|(a, b)| a < b && *b <= c.rows.text.len())
    {
        return c.rows.text[a..b].to_owned();
    }
    match span(c, sel) {
        Some((a, b)) if a <= b => c.rows.text[a..b].to_owned(),
        _ => sel.text().to_owned(),
    }
}

/// Rune's collection cap for a selection across unbuilt rows (R4): a report this long may be
/// cut short.
const RUNE_ROWS: usize = 10_000;
const RUNE_BYTES: usize = 4 << 20;

/// Whether `sel` reads as the whole text: every selected text whole, from the first row with
/// text to the last, or to where Rune's collection stops (R4).
pub(crate) fn reads_whole(c: &Content, sel: &TextSelection) -> bool {
    let Some((head, tail)) = c.ends else {
        return false;
    };
    let mut first_path = None;
    let mut last_path = None;
    let mut bytes = 0usize;
    for t in sel.texts().iter().filter(|t| t.is_selected()) {
        if !t.is_whole() {
            return false;
        }
        if first_path.is_none() {
            first_path = Some(t.path());
        }
        last_path = Some(t.path());
        bytes += t.len();
    }
    let (Some(first_path), Some(last_path)) = (first_path, last_path) else {
        return false;
    };
    if locate(&c.key, first_path) != Some(head) {
        return false;
    }
    let Some(end) = locate(&c.key, last_path) else {
        return false;
    };
    // Rune's cap counts rows, empty ones too, which have no selected text.
    let cut = end - head + 1 >= RUNE_ROWS || bytes >= RUNE_BYTES;
    end == tail || cut
}
