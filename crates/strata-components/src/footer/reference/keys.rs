//! The keys inside the reference's panel.
//!
//! Rune hands a focused text field's keys to its `on_input_key`, then its editor, then the
//! window's `on_key`, never to the field's ancestors (probe P1). Keys on other focused elements
//! reach the panel content's `on_key` before focus navigation. So the search field routes its
//! own keys, and the content routes the categories' and the results'. Both take every key the
//! panel doesn't use (bar Tab and Escape, which Rune's modal handles), so no browsing shortcut
//! runs under the reference.

use std::rc::Rc;

use rune::{Chord, Key, KeyEvent, Mods, Tx};

use super::search::Lines;
use super::state::{ReferenceKeys, ReferenceState};

/// What the key handlers need: the state's accessor, the keys, the element keys and the lines
/// on screen.
pub(super) struct Route<S> {
    pub get_mut: fn(&mut S) -> &mut ReferenceState,
    pub keys: ReferenceKeys,
    pub categories: String,
    pub search: String,
    pub chosen_category: String,
    pub results: String,
    pub line_key: Rc<dyn Fn(usize) -> String>,
    pub lines: Rc<Lines>,
}

/// Where focus is in the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Area {
    Search,
    Categories,
    Results,
    Other,
}

fn any(chords: &[Chord], ev: &KeyEvent, tx: &Tx) -> bool {
    chords.iter().any(|c| c.matches_tx(ev, tx))
}

/// Sends `key` through the window's routing on the next frame: the category group's own
/// arrows, for the keys that stand for them.
fn send(tx: &mut Tx, key: Key, shift: bool) {
    tx.send_key(KeyEvent {
        key,
        mods: Mods {
            shift,
            ..Mods::default()
        },
    });
}

impl<S: 'static> Route<S> {
    /// Where focus is, for the panel's handler: never the search field, whose keys go to
    /// [`Route::field_key`] (probe P1).
    fn area(&self, tx: &Tx) -> Area {
        match tx.focused() {
            Some(f) if f == self.results => Area::Results,
            Some(f) if f.starts_with(&self.categories) => Area::Categories,
            _ => Area::Other,
        }
    }

    /// Moves focus to `to`, one of the panel's three stops (search, the chosen category, the
    /// results), with its ring.
    fn focus(&self, from: Area, to: Area, tx: &mut Tx) {
        if from == to {
            return;
        }
        let key = match to {
            Area::Search => &self.search,
            Area::Categories => &self.chosen_category,
            Area::Results => &self.results,
            Area::Other => return,
        };
        tx.focus(key.clone(), true);
    }

    /// The focus keys (Ctrl+B, Ctrl+F, Ctrl+L): true when `ev` is one.
    fn area_key(&self, from: Area, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let to = if any(&self.keys.categories, ev, tx) {
            Area::Categories
        } else if any(&self.keys.search, ev, tx) {
            Area::Search
        } else if any(&self.keys.list, ev, tx) {
            Area::Results
        } else {
            return false;
        };
        self.focus(from, to, tx);
        true
    }

    /// Scrolls the results for Up and Down (with Shift too, as Strata's `scroll_motion` ignores
    /// it), Page Up and Page Down; from the results also `j`, `k`, Left, Right, Home and End
    /// (`results`). True when `ev` is one of them. A line step reveals the next line out of view
    /// (Rune has no line scroll: draft U-C19-1).
    fn scroll(&self, s: &mut S, ev: &KeyEvent, tx: &mut Tx, results: bool) -> bool {
        let m = ev.mods;
        if m.cmd || m.ctrl || m.alt || m.logo {
            return false;
        }
        let visible = (self.get_mut)(s).visible.clone();
        let n = self.lines.lines.len();
        let typed = |c: &str| results && !m.shift && matches!(&ev.key, Key::Text(t) if t == c);
        let down = ev.key == Key::Down || (results && ev.key == Key::Right) || typed("j");
        let up = ev.key == Key::Up || (results && ev.key == Key::Left) || typed("k");
        if down {
            if visible.end < n {
                tx.scroll_into_view((self.line_key)(visible.end));
            }
        } else if up {
            if visible.start > 0 {
                tx.scroll_into_view((self.line_key)(visible.start - 1));
            }
        } else if ev.key == Key::PageDown {
            tx.scroll_page(self.results.clone(), 1);
        } else if ev.key == Key::PageUp {
            tx.scroll_page(self.results.clone(), -1);
        } else if results && ev.key == Key::Home {
            tx.scroll_to_instant(self.results.clone(), 0.0, 0.0);
        } else if results && ev.key == Key::End {
            tx.scroll_to_bottom_instant(self.results.clone());
        } else {
            return false;
        }
        true
    }

    /// The search field's keys, before its editor. False lets the editor (or Rune's focus
    /// navigation and modal) have the key; true takes it.
    pub fn field_key(&self, s: &mut S, ev: &KeyEvent, tx: &mut Tx) -> bool {
        // `~` typed closes; a key that reads as `~` only through the layout types itself.
        if self.keys.toggles_typed(ev) || ev.key == Key::Escape {
            (self.get_mut)(s).close();
            return true;
        }
        if self.area_key(Area::Search, ev, tx) {
            return true;
        }
        if self.scroll(s, ev, tx, false) {
            return true;
        }
        !edits(ev)
    }

    /// The categories' and the results' keys, before focus navigation.
    pub fn panel_key(&self, s: &mut S, ev: &KeyEvent, tx: &mut Tx) -> bool {
        // Escape with any modifiers closes, as Strata's `ESCAPE_ANY`.
        if self.keys.toggles(ev, tx) || ev.key == Key::Escape {
            (self.get_mut)(s).close();
            return true;
        }
        let area = self.area(tx);
        if self.area_key(area, ev, tx) {
            return true;
        }
        if tab(ev) {
            return false;
        }
        match area {
            Area::Categories => self.category_key(ev, tx),
            Area::Results => {
                self.scroll(s, ev, tx, true);
                true
            }
            // Nothing in the panel focused: no key goes on to the window.
            _ => true,
        }
    }

    /// The categories' keys: false leaves the focus group's arrows, Home and End and the
    /// press keys (Space, Enter) to Rune; `h j k l`, and the arrows, Home and End with Shift
    /// (Strata's `category_step` ignores Shift, Rune's focus group doesn't), go as the plain
    /// arrows. Everything else is taken.
    fn category_key(&self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let m = ev.mods;
        if m.cmd || m.ctrl || m.alt || m.logo {
            return true;
        }
        let step = match &ev.key {
            Key::Text(t) if !m.shift && t == "h" => Some(Key::Left),
            Key::Text(t) if !m.shift && t == "j" => Some(Key::Down),
            Key::Text(t) if !m.shift && t == "k" => Some(Key::Up),
            Key::Text(t) if !m.shift && t == "l" => Some(Key::Right),
            k @ (Key::Up | Key::Down | Key::Left | Key::Right | Key::Home | Key::End)
                if m.shift =>
            {
                Some(k.clone())
            }
            _ => None,
        };
        if let Some(arrow) = step {
            send(tx, arrow, false);
            return true;
        }
        let group = matches!(
            ev.key,
            Key::Up | Key::Down | Key::Left | Key::Right | Key::Home | Key::End | Key::Enter
        ) || matches!(&ev.key, Key::Text(t) if t == " ");
        !(group && !m.shift)
    }
}

/// Tab and Shift+Tab, which Rune's modal keeps inside the panel.
fn tab(ev: &KeyEvent) -> bool {
    let m = ev.mods;
    ev.key == Key::Tab && !(m.cmd || m.ctrl || m.alt || m.logo)
}

/// Whether the search field's editor edits with `ev`: typing (AltGr's characters included), the
/// copy, cut, undo, redo and select-all chords (Shift only with `z`: Rune's editor takes
/// Ctrl+Shift+Z alone), and the caret and word keys. Everything else, an unused Ctrl, Alt or
/// Super chord among them (Ctrl+Shift+A is Strata's Unselect all; Alt+Space the window menu's),
/// is taken before it reaches the window
/// (`app/shortcut-reference`: "…reaches nothing else: Ctrl+, opens no Settings"). Ctrl+V is
/// taken too: Rune reads the clipboard into the focused field before it routes the key
/// (`window/events.rs`, `runtime/input/physical.rs` at the pin), so a Ctrl+V that reaches the
/// field is one no paste followed, and the editor has no use for it.
pub(super) fn edits(ev: &KeyEvent) -> bool {
    let m = ev.mods;
    if tab(ev) {
        return true;
    }
    match &ev.key {
        Key::Text(t) => !(m.cmd || m.ctrl || m.logo) && !(m.alt && t == " "),
        Key::Command(c) => !m.alt && !m.logo && (*c == 'z' || (!m.shift && "acxy".contains(*c))),
        Key::Left | Key::Right | Key::Home | Key::End | Key::Backspace | Key::Delete => {
            !m.alt && !m.logo
        }
        _ => false,
    }
}
