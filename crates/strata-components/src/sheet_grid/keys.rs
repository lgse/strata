//! The grid's keys, with focus on it.
//!
//! Strata's (`src/ui/table_view/selection.rs`): Ctrl+C copies the range, or every row without
//! one; Ctrl+Shift+C, Ctrl+Alt+C and Ctrl+Super+C copy nothing. Ctrl+A and Ctrl+Shift+A clear
//! the range, Ctrl+Alt+A leaves it, and Escape with any modifiers, or an arrow, Home or End
//! with Ctrl, Alt or Super or none, clear it too. Strata clears and passes the key on; here a
//! key that clears a range is taken (so Escape with a range doesn't also close the preview),
//! and one that clears nothing passes on.
//!
//! New against Strata (D6, D8): the arrows, Home, End, Page Up and Page Down move a cell
//! cursor (Ctrl+Home and Ctrl+End to the first and last cell), with Shift they grow the range
//! from its anchor, and Ctrl+Page Up and Ctrl+Page Down switch sheets.

use rune::{Key, KeyEvent, Tx};

use super::range::CellPos;
use super::state::SheetGridState;

/// What a key did.
pub(crate) enum Outcome {
    Ignored,
    Taken,
    /// Taken, and it showed a sheet the host must load.
    Load(usize),
}

/// Whether a command chord carries a third modifier besides the primary one (and Shift):
/// Super off macOS (Ctrl+Super+C), Control on macOS, where Super is the primary.
fn other_modifier(m: rune::Mods) -> bool {
    m.alt
        || if cfg!(target_os = "macos") {
            m.ctrl
        } else {
            m.logo
        }
}

/// A key that only clears the range: taken when there was one, so nothing else acts on it
/// (Strata clears and passes it on; here Escape with a range doesn't also close the preview).
fn clear(st: &mut SheetGridState) -> Outcome {
    if st.clear_range() {
        Outcome::Taken
    } else {
        Outcome::Ignored
    }
}

pub(crate) fn key_in(st: &mut SheetGridState, ev: &KeyEvent, tx: &mut Tx) -> Outcome {
    let m = ev.mods;
    let ctrl = m.cmd || m.ctrl;
    if ev.is_command('c') {
        if m.shift || other_modifier(m) {
            return Outcome::Ignored;
        }
        if let Some(text) = st.copy_text() {
            tx.copy(text);
        }
        return Outcome::Taken;
    }
    if ev.is_command('a') {
        if other_modifier(m) {
            return Outcome::Ignored;
        }
        st.clear_range();
        return Outcome::Taken;
    }
    if ctrl && matches!(ev.key, Key::PageUp | Key::PageDown) && !m.shift && !m.alt {
        let n = st.sheet_count();
        let i = st.active();
        let to = match ev.key {
            Key::PageUp => i.checked_sub(1),
            _ => (i + 1 < n).then_some(i + 1),
        };
        return match to {
            Some(to) if st.show(to, tx) => Outcome::Load(to),
            _ => Outcome::Taken,
        };
    }
    // Strata's `ESCAPE_ANY`, and the arrows, Home and End ignoring Ctrl, Alt, Shift and Super:
    // each clears the range. Here Shift with an arrow, Home or End grows it instead (D6), and
    // Ctrl+Home and Ctrl+End move the cursor (which clears it too).
    let nav = matches!(
        ev.key,
        Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End
    );
    let arrow = matches!(ev.key, Key::Left | Key::Right | Key::Up | Key::Down);
    if ev.key == Key::Escape || (nav && (m.alt || m.logo)) || (arrow && ctrl) {
        return clear(st);
    }
    if m.alt || m.logo {
        return Outcome::Ignored;
    }
    let Some(v) = st.view() else {
        return Outcome::Ignored;
    };
    let (rows, cols) = (v.rows(), v.columns());
    if rows == 0 || cols == 0 {
        return Outcome::Ignored;
    }
    let page = ((st.feed.get().viewport[1] / st.row_h.get().max(1.0)).floor() as usize)
        .saturating_sub(1)
        .max(1);
    let at = st.cursor();
    let c = at.unwrap_or_default();
    let to = match (&ev.key, ctrl) {
        (Key::Up, false) => CellPos::new(c.row.saturating_sub(1), c.col),
        (Key::Down, false) if at.is_some() => CellPos::new((c.row + 1).min(rows - 1), c.col),
        (Key::Left, false) => CellPos::new(c.row, c.col.saturating_sub(1)),
        (Key::Right, false) if at.is_some() => CellPos::new(c.row, (c.col + 1).min(cols - 1)),
        // The first arrow without a cursor lands on the first cell.
        (Key::Down | Key::Right, false) => c,
        (Key::Home, false) => CellPos::new(c.row, 0),
        (Key::End, false) => CellPos::new(c.row, cols - 1),
        (Key::Home, true) => CellPos::new(0, 0),
        (Key::End, true) => CellPos::new(rows - 1, cols - 1),
        (Key::PageUp, false) => CellPos::new(c.row.saturating_sub(page), c.col),
        (Key::PageDown, false) => CellPos::new((c.row + page).min(rows - 1), c.col),
        _ => return Outcome::Ignored,
    };
    st.move_cursor(to, m.shift, tx);
    Outcome::Taken
}
