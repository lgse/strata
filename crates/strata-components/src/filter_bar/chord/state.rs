//! [`ChordState`]: the armed 10xer chord and its key map, as data.

use rune::{Chord, Key, KeyEvent, Tx};

/// One line of a chord's panel: a keycap and what it does, and the chords that complete it.
///
/// Used by: hosts.
#[derive(Clone, Debug)]
pub struct ChordOption<A> {
    /// The keycap, as Strata labels the key: `g`, `G`, `1–9`, `+ / −`, `Space`, `Shift`.
    pub keycap: String,
    /// What it does: "First item", "Reverse".
    pub label: String,
    /// The second keys that complete the chord this way, each with the host's action. Empty
    /// for a line that only says something (`Shift` Reverse, a held modifier).
    pub chords: Vec<(Chord, A)>,
}

impl<A> ChordOption<A> {
    /// A line completed by `chords`.
    pub fn new(
        keycap: impl Into<String>,
        label: impl Into<String>,
        chords: Vec<(Chord, A)>,
    ) -> Self {
        Self {
            keycap: keycap.into(),
            label: label.into(),
            chords,
        }
    }

    /// A line no key completes.
    pub fn mention(keycap: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(keycap, label, Vec::new())
    }
}

/// An armed chord's key map: its footer mark and the panel's lines, in panel order (down the
/// first column, then the second).
///
/// Used by: hosts.
#[derive(Clone, Debug)]
pub struct ChordMap<A> {
    /// The pill's text: `g-`.
    pub mark: String,
    /// The panel's accessible name: "Keys after g".
    pub name: String,
    /// The lines.
    pub options: Vec<ChordOption<A>>,
}

/// What a key did to an armed chord.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChordOutcome<A> {
    /// A modifier alone: the chord stays armed and the key goes on.
    Pass,
    /// Escape: the chord is cancelled, quietly.
    Cancelled,
    /// A second key the map names: run `A`.
    Completed(A),
    /// Any other key: the chord is cancelled, and the host flashes "Unknown chord".
    Unknown,
}

/// The armed chord, if any. While one is armed, the next key belongs to it alone: the host
/// calls [`ChordState::key`] before any other handler (its pane's `on_key`, ahead of the
/// collection), and takes the key unless it is [`ChordOutcome::Pass`]. Arming shows the pill
/// and the panel in the same frame; nothing times a chord out (Strata's).
///
/// The host clears it ([`ChordState::clear`]) when a prompt opens, Settings opens, the mode
/// is left or the window closes.
///
/// Used by: hosts.
#[derive(Clone, Debug)]
pub struct ChordState<A> {
    armed: Option<ChordMap<A>>,
}

impl<A> Default for ChordState<A> {
    fn default() -> Self {
        Self { armed: None }
    }
}

impl<A: Clone> ChordState<A> {
    /// Nothing armed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The armed chord's map.
    pub fn armed(&self) -> Option<&ChordMap<A>> {
        self.armed.as_ref()
    }

    /// Whether a chord is armed.
    pub fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    /// Arms the chord `map` describes: its prefix key was pressed.
    pub fn arm(&mut self, map: ChordMap<A>) {
        self.armed = Some(map);
    }

    /// Drops the armed chord. True when one was armed.
    pub fn clear(&mut self) -> bool {
        self.armed.take().is_some()
    }

    /// The next key, `None` with nothing armed. It disarms the chord unless it is a modifier
    /// alone. Chords match as Rune's `Chord::matches_tx` does, following the layout.
    pub fn key(&mut self, ev: &KeyEvent, tx: &Tx) -> Option<ChordOutcome<A>> {
        if matches!(ev.key, Key::Modifier { .. }) {
            return self.armed.as_ref().map(|_| ChordOutcome::Pass);
        }
        let map = self.armed.take()?;
        if ev.key == Key::Escape {
            return Some(ChordOutcome::Cancelled);
        }
        let done = map
            .options
            .iter()
            .flat_map(|o| o.chords.iter())
            .find(|(c, _)| c.matches_tx(ev, tx))
            .map(|(_, a)| a.clone());
        Some(done.map_or(ChordOutcome::Unknown, ChordOutcome::Completed))
    }
}
