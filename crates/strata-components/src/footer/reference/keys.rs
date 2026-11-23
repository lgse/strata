// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Chord, Key, KeyEvent, Mods, Tx};

use super::search::Lines;
use super::state::{ReferenceKeys, ReferenceState};

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
    fn area(&self, tx: &Tx) -> Area {
        match tx.focused() {
            Some(f) if f == self.results => Area::Results,
            Some(f) if f.starts_with(&self.categories) => Area::Categories,
            _ => Area::Other,
        }
    }

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
