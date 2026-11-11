//! What copying a selection takes: the text between its two ends, from the model, so rows
//! split from one long line join with no break and lines keep their own breaks. From Rune's
//! report only the ends are taken: the lowest and highest selected bytes over every selected
//! text.
//!
//! A selection reaching more than 10,000 unbuilt rows is cut short there by Rune (R4, C11), so
//! its far end is wrong; Ctrl+A is copied whole from the model instead.

use rune::TextSelection;

use super::state::Content;

/// The row of a selected text, from its path (`…/{key}-l{row}/…/t`).
pub(crate) fn locate(key: &str, path: &str) -> Option<usize> {
    let prefix = format!("{key}-l");
    let mut row = None;
    let mut last = "";
    for seg in path.split('/') {
        if let Some(n) = seg.strip_prefix(&prefix).and_then(|n| n.parse().ok()) {
            row = Some(n);
        }
        last = seg;
    }
    (last == "t").then_some(row?)
}

/// The text bytes a selection covers, if any of it is the view's.
fn span(c: &Content, sel: &TextSelection) -> Option<(usize, usize)> {
    let mut out: Option<(usize, usize)> = None;
    for t in sel.texts().iter().filter(|t| t.is_selected()) {
        let Some(r) = locate(&c.key, t.path()).filter(|&r| r < c.rows.len()) else {
            continue;
        };
        let row = c.rows.row(r);
        let len = row.end - row.start;
        let range = t.range();
        let a = row.start + range.start.min(len);
        let b = row.start + range.end.min(len);
        out = Some(out.map_or((a, b), |(lo, hi)| (lo.min(a), hi.max(b))));
    }
    out
}

pub(crate) fn copy(c: &Content, sel: &TextSelection) -> String {
    if c.all.get() {
        return c.rows.text.to_string();
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
    let selected: Vec<_> = sel.texts().iter().filter(|t| t.is_selected()).collect();
    let (Some(first), Some(last)) = (selected.first(), selected.last()) else {
        return false;
    };
    if !selected.iter().all(|t| t.is_whole()) {
        return false;
    }
    let Some((head, tail)) = c.ends else {
        return false;
    };
    if locate(&c.key, first.path()) != Some(head) {
        return false;
    }
    let bytes: usize = selected.iter().map(|t| t.len()).sum();
    let Some(end) = locate(&c.key, last.path()) else {
        return false;
    };
    // Rune's cap counts rows, empty ones too, which have no selected text.
    let cut = end - head + 1 >= RUNE_ROWS || bytes >= RUNE_BYTES;
    end == tail || cut
}
