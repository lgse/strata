// SPDX-License-Identifier: MIT

//! Read-only keyboard caret, Shift+arrow selection, and gutter-margin drag selection for
//! [`super::CodeView`].
//!
//! Workaround for Rune packet entry 34 (`docs/rune/upstream/rune/34-read-only-caret.md`):
//! Rune's `El::selectable` text handles only `Cmd+A` and `Cmd+C` from the keyboard and starts
//! pointer selections only on the text node itself (not in a line-number gutter). Tracking byte
//! offsets in [`CodeState`] and driving `Tx::select_text` / `Tx::copy` gives source previews
//! `Ctrl+Home` / `Ctrl+End`, `Home` / `End`, `Shift+Arrow` / `Ctrl+Shift+Arrow` selection, and
//! margin-to-text drag selection.

use rune::{Drag, Key, KeyEvent, Phase, PointerButton, TextStyle, Tx};

use super::state::CodeState;

/// Handles caret and keyboard-selection keys inside a focused [`super::CodeView`].
///
/// Returns `Some(handled)` when the key belongs to caret navigation or selection, or `None` when
/// the caller should run its own fallback (such as plain `Left` leaving the preview).
pub(super) fn on_key(st: &mut CodeState, ev: &KeyEvent, tx: &mut Tx) -> Option<bool> {
    let m = ev.mods;
    if m.alt {
        return None;
    }
    let ctrl = m.ctrl || m.cmd;
    match ev.key {
        Key::Home | Key::End => {
            st.sync_caret_from_selection(tx.text_selection());
            Some(home_or_end(st, ev.key == Key::End, (ctrl, m.shift), tx))
        }
        Key::Left | Key::Right if m.shift => {
            st.sync_caret_from_selection(tx.text_selection());
            Some(step_horizontal(st, ev.key == Key::Right, ctrl, tx))
        }
        Key::Left | Key::Right if !ctrl => {
            st.sync_caret_from_selection(tx.text_selection());
            plain_horizontal(st, ev.key == Key::Right, tx)
        }
        Key::Up | Key::Down if m.shift && !ctrl => {
            st.sync_caret_from_selection(tx.text_selection());
            Some(step_vertical(st, ev.key == Key::Down, tx))
        }
        _ => None,
    }
}

fn home_or_end(st: &mut CodeState, to_end: bool, (ctrl, shift): (bool, bool), tx: &mut Tx) -> bool {
    let text = st.text().clone();
    let (anchor, head) = st.caret_or_selection(tx.text_selection());
    let target = if ctrl {
        let p = st.position.clone();
        p.go(if to_end { f64::INFINITY } else { 0.0 }, tx);
        if to_end { text.len() } else { 0 }
    } else {
        let (start, end) = line_bounds(&text, head);
        if to_end { end } else { start }
    };
    let next_anchor = if shift { anchor } else { target };
    st.apply_caret(next_anchor, target, tx);
    true
}

fn step_horizontal(st: &mut CodeState, forward: bool, by_word: bool, tx: &mut Tx) -> bool {
    let text = st.text().clone();
    let (anchor, head) = st.caret_or_selection(tx.text_selection());
    let target = match (forward, by_word) {
        (true, false) => next_char(&text, head),
        (false, false) => prev_char(&text, head),
        (true, true) => next_word(&text, head),
        (false, true) => prev_word(&text, head),
    };
    st.apply_caret(anchor, target, tx);
    true
}

fn plain_horizontal(st: &mut CodeState, forward: bool, tx: &mut Tx) -> Option<bool> {
    if let Some((lo, hi)) = st.keyboard_selection() {
        let collapsed = if forward { hi } else { lo };
        st.apply_caret(collapsed, collapsed, tx);
        return Some(true);
    }
    if !forward {
        return None;
    }
    let (_, head) = st.caret()?;
    let next = next_char(st.text(), head);
    st.apply_caret(next, next, tx);
    Some(true)
}

fn step_vertical(st: &mut CodeState, down: bool, tx: &mut Tx) -> bool {
    let text = st.text().clone();
    let (anchor, head) = st.caret_or_selection(tx.text_selection());
    let target = step_line(&text, head, down);
    st.apply_caret(anchor, target, tx);
    true
}

#[derive(Clone, Copy)]
pub(crate) struct GutterHit {
    pub row: usize,
    pub text_x: f32,
    pub mono: TextStyle,
}

/// Handles a primary pointer drag starting in row `hit.row`'s left gutter / padding
/// (`0..hit.text_x`), extending the selection from the start of the row to `d.x - hit.text_x`.
pub(crate) fn gutter_drag(st: &mut CodeState, hit: GutterHit, d: &mut Drag, tx: &mut Tx) {
    if tx.button() != PointerButton::Primary {
        tx.live_only();
        return;
    }
    let Some(row) = st.rows().get(hit.row) else {
        tx.live_only();
        return;
    };
    match d.phase {
        Phase::Down => {
            st.apply_caret(row.start, row.start, tx);
            tx.live_only();
        }
        Phase::Move if !d.cancelled() => {
            let x = d.x - hit.text_x;
            let end = if x <= 0.0 {
                row.start
            } else {
                row.start + byte_at_x(st.rows().source(hit.row), x, hit.mono)
            };
            if st.caret() != Some((row.start, end)) {
                st.apply_caret(row.start, end, tx);
            }
            tx.live_only();
        }
        Phase::Move | Phase::Up => {
            tx.live_only();
        }
    }
}

/// Largest valid UTF-8 character boundary in `line` at or before `byte`.
fn floor_boundary(line: &str, byte: usize) -> usize {
    let mut b = byte.min(line.len());
    while b > 0 && !line.is_char_boundary(b) {
        b -= 1;
    }
    b
}

/// Byte offset in `line` whose glyph midpoint is nearest `x` points in `mono`.
pub(crate) fn byte_at_x(line: &str, x: f32, mono: TextStyle) -> usize {
    if x <= 0.0 || line.is_empty() {
        return 0;
    }
    let total = rune::text::advance(line, mono);
    if x >= total {
        return line.len();
    }
    let mut lo = 0usize;
    let mut hi = line.len();
    let mut w_lo = 0.0_f32;
    let mut w_hi = total;
    loop {
        let next = next_char(line, lo);
        if next >= hi {
            break;
        }
        let mut mid = floor_boundary(line, lo + (hi - lo) / 2);
        if mid <= lo {
            mid = next;
        }
        let w_mid = rune::text::advance(&line[..mid], mono);
        if w_mid <= x {
            lo = mid;
            w_lo = w_mid;
        } else {
            hi = mid;
            w_hi = w_mid;
        }
    }
    if x < (w_lo + w_hi) * 0.5 { lo } else { hi }
}

pub(crate) fn prev_char(text: &str, at: usize) -> usize {
    let clamped = at.min(text.len());
    text[..clamped]
        .char_indices()
        .next_back()
        .map_or(0, |(idx, _)| idx)
}

pub(crate) fn next_char(text: &str, at: usize) -> usize {
    let clamped = at.min(text.len());
    text[clamped..]
        .chars()
        .next()
        .map_or(text.len(), |ch| clamped + ch.len_utf8())
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn prev_word(text: &str, at: usize) -> usize {
    let mut i = at.min(text.len());
    while i > 0 {
        let prev = prev_char(text, i);
        let Some(ch) = text[prev..i].chars().next() else {
            break;
        };
        if is_word_char(ch) {
            i = prev;
            break;
        }
        i = prev;
    }
    while i > 0 {
        let prev = prev_char(text, i);
        let Some(ch) = text[prev..i].chars().next() else {
            break;
        };
        if !is_word_char(ch) {
            break;
        }
        i = prev;
    }
    i
}

fn next_word(text: &str, at: usize) -> usize {
    let mut i = at.min(text.len());
    while i < text.len() {
        let Some(ch) = text[i..].chars().next() else {
            break;
        };
        i += ch.len_utf8();
        if is_word_char(ch) {
            break;
        }
    }
    while i < text.len() {
        let Some(ch) = text[i..].chars().next() else {
            break;
        };
        if !is_word_char(ch) {
            break;
        }
        i += ch.len_utf8();
    }
    i
}

pub(crate) fn line_bounds(text: &str, at: usize) -> (usize, usize) {
    let clamped = at.min(text.len());
    let start = text[..clamped].rfind('\n').map_or(0, |i| i + 1);
    let nl = text[clamped..]
        .find('\n')
        .map_or(text.len(), |i| clamped + i);
    let end = if nl > start && text.as_bytes().get(nl - 1) == Some(&b'\r') {
        nl - 1
    } else {
        nl
    };
    (start, end)
}

fn step_line(text: &str, at: usize, down: bool) -> usize {
    let (start, end) = line_bounds(text, at);
    let col = text[start..at.min(end)].chars().count();
    let target_line_start = if down {
        let Some(nl) = text[end..].find('\n') else {
            return text.len();
        };
        end + nl + 1
    } else {
        if start == 0 {
            return 0;
        }
        let prev_pos = start - 1;
        let prev_at = if prev_pos > 0 && text.as_bytes().get(prev_pos - 1) == Some(&b'\r') {
            prev_pos - 1
        } else {
            prev_pos
        };
        line_bounds(text, prev_at).0
    };
    let (_, target_end) = line_bounds(text, target_line_start);
    let slice = &text[target_line_start..target_end];
    slice
        .char_indices()
        .nth(col)
        .map_or(target_end, |(idx, _)| target_line_start + idx)
}
