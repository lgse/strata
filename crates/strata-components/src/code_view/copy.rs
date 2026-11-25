// SPDX-License-Identifier: MIT

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

pub(crate) fn span(c: &Content, sel: &TextSelection) -> Option<(usize, usize)> {
    let texts = sel.texts();
    let (first_a, first_b) = texts.iter().find_map(|t| row_bounds(c, t))?;
    let (last_a, last_b) = texts.iter().rev().find_map(|t| row_bounds(c, t))?;
    Some((first_a.min(last_a), first_b.max(last_b)))
}

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

pub(crate) fn reads_whole(c: &Content, sel: &TextSelection) -> bool {
    let Some((head, tail)) = c.ends else {
        return false;
    };
    let mut first_path = None;
    let mut last_path = None;
    for t in sel.texts().iter().filter(|t| t.is_selected()) {
        if !t.is_whole() {
            return false;
        }
        if first_path.is_none() {
            first_path = Some(t.path());
        }
        last_path = Some(t.path());
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
    end == tail || sel.truncated()
}
