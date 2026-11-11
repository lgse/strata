//! The key map: which chords run which [`Command`]s, and which keys stand in for the arrows.

use rune::{Chord, ChordKey, ChordMod, KeyPlatform};

use super::command::{Command, RangeKind, Target, Vertical};
use super::settings::Letters;
use crate::collection::element::KeyInput;
use crate::contract::Direction;

/// Which chords run which commands, and which other keys act as an arrow.
///
/// [`KeyMap::default_map`] and [`KeyMap::tenxer`] are Strata's shipped tables
/// (`src/ui/keys/bindings/navigation.rs` and `tenxer.rs` at `upstream/main` 35ee677ab). A host
/// changes them as its users rebind keys: [`KeyMap::bind`], [`KeyMap::unbind`],
/// [`KeyMap::alias`]. Checking a binding against the rest of the table (Strata's refused and
/// noted keys) is the host's.
///
/// Keys match with [`KeyInput::matches`]: Rune's layout-aware chords, so a letter follows the
/// key's label under the active layout and a key that types a non-Latin character falls back
/// to the US key in its place. Modifiers match exactly, except those a binding tolerates
/// ([`KeyMap::bind_tolerating`]), as Strata's matchers do (`src/ui/window/keyboard/items.rs`):
/// in the default map, Shift+Up and Shift+Down extend with Ctrl, Alt or Super held too, the
/// page keys ignore Super, and Escape ignores Shift. Ctrl+Up and Ctrl+Down tolerate nothing,
/// so with Alt or Super they don't jump, and with Shift they extend. A binding of the exact
/// chord always wins over a tolerated match, so a host that rebinds Shift+Escape or
/// Ctrl+Shift+Up gets its own command, and [`KeyMap::unbind_chord`] removes tolerated matches
/// too.
///
/// Bindings apply whatever [`Letters`] says. An alias stands in for an arrow, as `h j k l` do
/// in the default map; one on a plain printable key (no Ctrl, Alt or Super) applies only with
/// [`Letters::Arrows`], and only without modifiers, so Shift+J doesn't extend.
///
/// Used by: C19 (the shortcut reference), C20 (keyboard settings), and hosts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyMap {
    bindings: Vec<Binding>,
    aliases: Vec<(Chord, ChordKey)>,
}

/// One binding: a chord, the modifiers it matches with or without, and its command.
///
/// Used by: C19, C20 (through [`KeyMap::bindings`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// The chord, as labels show it.
    pub chord: Chord,
    /// Modifiers that may also be held: Strata's `ignoring(..)`.
    pub tolerates: Vec<ChordMod>,
    /// Chords with tolerated modifiers that no longer match: what [`KeyMap::unbind_chord`]
    /// took off this binding.
    pub excludes: Vec<Chord>,
    /// What it runs.
    pub command: Command,
}

impl Binding {
    /// The chord with each combination of tolerated modifiers held too, the chord itself
    /// left out, and without the excluded ones.
    fn variants(&self) -> impl Iterator<Item = Chord> + '_ {
        let n = self.tolerates.len();
        (1..1u32 << n)
            .map(move |subset| {
                (0..n)
                    .filter(|i| subset & (1 << i) != 0)
                    .fold(self.chord, |c, i| c.with(self.tolerates[i]))
            })
            .filter(|v| !self.excludes.iter().any(|x| same(x, v)))
    }
}

/// Whether two chords are the same keys here ("mod+a" and "ctrl+a" are, off macOS).
fn same(a: &Chord, b: &Chord) -> bool {
    a.same_keys(b, KeyPlatform::current())
}

impl KeyMap {
    /// No bindings and no aliases.
    pub fn new() -> Self {
        Self::default()
    }

    /// Strata's default map, in every view and the file chooser. `h j k l` alias the arrows.
    pub fn default_map() -> Self {
        let mut map = Self::new();
        // Strata's `selection_navigation`: Shift+Up and Shift+Down extend even with Ctrl, Alt
        // or Super held; Shift+Left and Shift+Right are GTK's, with no extra modifiers.
        const EXTENDS: [ChordMod; 3] = [ChordMod::Ctrl, ChordMod::Alt, ChordMod::Super];
        for d in DIRECTIONS {
            let k = arrow(d);
            map.bind(plain(k), Command::Select(Target::Step(d)));
            let extend = Command::Extend(Target::Step(d));
            match d {
                Direction::Up | Direction::Down => map.bind_tolerating(shift(k), &EXTENDS, extend),
                Direction::Left | Direction::Right => map.bind(shift(k), extend),
            };
        }
        for (k, target) in [
            (ChordKey::Home, Target::First),
            (ChordKey::End, Target::Last),
        ] {
            map.bind(plain(k), Command::Select(target));
            map.bind(shift(k), Command::Extend(target));
        }
        // `page_key`: the page keys ignore Super.
        let pages = [
            (ChordKey::PageUp, Target::Page(Vertical::Up)),
            (ChordKey::PageDown, Target::Page(Vertical::Down)),
        ];
        for (k, target) in pages {
            map.bind_tolerating(plain(k), &[ChordMod::Super], Command::Select(target));
            map.bind_tolerating(shift(k), &[ChordMod::Super], Command::Extend(target));
        }
        // Ctrl+Left and Ctrl+Right move the cursor, and stop silently at an edge.
        map.bind(ctrl(ChordKey::Left), Command::Nudge(Direction::Left));
        map.bind(ctrl(ChordKey::Right), Command::Nudge(Direction::Right));
        map.bind(ctrl(ChordKey::Up), Command::Select(Target::First));
        map.bind(ctrl(ChordKey::Down), Command::Select(Target::Last));
        map.bind(ctrl(ChordKey::Char(' ')), Command::Toggle);
        map.bind(ctrl(ChordKey::Char('a')), Command::SelectAll);
        // `ESCAPE_ANY_SHIFT`.
        map.bind_tolerating(plain(ChordKey::Escape), &[ChordMod::Shift], Command::Clear);
        map.bind(plain(ChordKey::Enter), Command::Activate);
        for (c, d) in VIM {
            map.alias(plain(ChordKey::Char(c)), arrow(d));
        }
        map
    }

    /// Strata's 10xer map. Every move keeps the selection; `h j k l` are bindings of their own,
    /// so they move whatever [`Letters`] says. Escape is left to the host's Escape ladder
    /// (filter, find highlights, range, preview, then the selection), which ends a range with
    /// [`KeyHandle::end_range`](super::KeyHandle::end_range).
    pub fn tenxer() -> Self {
        let mut map = Self::new();
        for d in DIRECTIONS {
            map.bind(plain(arrow(d)), Command::Cursor(Target::Step(d)));
            // `v` and `V` are the only range keys.
            map.bind(shift(arrow(d)), Command::Swallow);
        }
        for (c, d) in VIM {
            map.bind(plain(ChordKey::Char(c)), Command::Cursor(Target::Step(d)));
        }
        let up = Target::Page(Vertical::Up);
        let down = Target::Page(Vertical::Down);
        let moves = [
            (plain(ChordKey::Home), Target::First),
            (plain(ChordKey::End), Target::Last),
            (shift(ChordKey::Char('g')), Target::Last),
            // Recorded decision: in 10xer, Ctrl+Up and Ctrl+Down move the cursor only.
            (ctrl(ChordKey::Up), Target::First),
            (ctrl(ChordKey::Down), Target::Last),
            (plain(ChordKey::PageUp), up),
            (plain(ChordKey::PageDown), down),
            (ctrl(ChordKey::Char('b')), up),
            (ctrl(ChordKey::Char('f')), down),
            (ctrl(ChordKey::PageUp), up),
            (ctrl(ChordKey::PageDown), down),
            (ctrl(ChordKey::Char('u')), Target::HalfPage(Vertical::Up)),
            (ctrl(ChordKey::Char('d')), Target::HalfPage(Vertical::Down)),
        ];
        for (chord, target) in moves {
            map.bind(chord, Command::Cursor(target));
        }
        map.bind(plain(ChordKey::Char(' ')), Command::ToggleAdvance);
        map.bind(ctrl(ChordKey::Char(' ')), Command::Toggle);
        map.bind(
            plain(ChordKey::Char('v')),
            Command::Range(RangeKind::Select),
        );
        map.bind(shift(ChordKey::Char('v')), Command::Range(RangeKind::Unset));
        map.bind(ctrl(ChordKey::Char('a')), Command::SelectAll);
        map.bind(ctrl(ChordKey::Char('r')), Command::Invert);
        map.bind(plain(ChordKey::Enter), Command::Activate);
        map.bind(plain(ChordKey::Char('o')), Command::Activate);
        map
    }

    /// Binds `chord` to `command`, after any binding it already has. A chord bound twice runs
    /// its first command.
    pub fn bind(&mut self, chord: Chord, command: Command) -> &mut Self {
        self.bind_tolerating(chord, &[], command)
    }

    /// Binds `chord` to `command`, matching with or without any of `tolerates` held too.
    pub fn bind_tolerating(
        &mut self,
        chord: Chord,
        tolerates: &[ChordMod],
        command: Command,
    ) -> &mut Self {
        self.bindings.push(Binding {
            chord,
            tolerates: tolerates.to_vec(),
            excludes: Vec::new(),
            command,
        });
        self
    }

    /// Removes every chord bound to `command`.
    pub fn unbind(&mut self, command: Command) -> &mut Self {
        self.bindings.retain(|b| b.command != command);
        self
    }

    /// Removes the binding of `chord`, whatever it runs: a binding of exactly that chord goes,
    /// and a binding that matched it only through a tolerated modifier (Super+Page Down, for
    /// Page Down tolerating Super) stops matching it, and keeps its other chords.
    pub fn unbind_chord(&mut self, chord: Chord) -> &mut Self {
        self.bindings.retain(|b| !same(&b.chord, &chord));
        for b in &mut self.bindings {
            if b.variants().any(|v| same(&v, &chord)) {
                b.excludes.push(chord);
            }
        }
        self
    }

    /// Makes `chord` act as the plain key `arrow` (an arrow, or any key the map binds plainly):
    /// it runs what that key is bound to.
    pub fn alias(&mut self, chord: Chord, arrow: ChordKey) -> &mut Self {
        self.aliases.push((chord, arrow));
        self
    }

    /// Removes every alias.
    pub fn clear_aliases(&mut self) -> &mut Self {
        self.aliases.clear();
        self
    }

    /// The chords that run `command`, bindings first, then aliases of the keys bound to it.
    pub fn chords_for(&self, command: Command) -> Vec<Chord> {
        let mut chords: Vec<Chord> = self
            .bindings
            .iter()
            .filter(|b| b.command == command)
            .map(|b| b.chord)
            .collect();
        for (alias, key) in &self.aliases {
            if self.bound(plain(*key)) == Some(command) {
                chords.push(*alias);
            }
        }
        chords
    }

    /// Every binding, in order.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The command `input` runs.
    ///
    /// A binding of the exact chord wins over one that matches only through a tolerated
    /// modifier, wherever each sits in the table: binding Shift+Escape to another command
    /// takes it from Escape, which tolerates Shift.
    pub(super) fn lookup(&self, input: &KeyInput<'_>, letters: Letters) -> Option<Command> {
        let exact = self.bindings.iter().find(|b| input.matches(&b.chord));
        let tolerated = || {
            self.bindings
                .iter()
                .find(|b| b.variants().any(|v| input.matches(&v)))
        };
        if let Some(b) = exact.or_else(tolerated) {
            return Some(b.command);
        }
        self.aliases
            .iter()
            .filter(|(c, _)| letters == Letters::Arrows || !typed(c))
            .find(|(c, _)| input.matches(c))
            .and_then(|(_, key)| self.bound(plain(*key)))
    }

    fn bound(&self, chord: Chord) -> Option<Command> {
        self.bindings
            .iter()
            .find(|b| b.chord == chord)
            .map(|b| b.command)
    }
}

const DIRECTIONS: [Direction; 4] = [
    Direction::Up,
    Direction::Down,
    Direction::Left,
    Direction::Right,
];

const VIM: [(char, Direction); 4] = [
    ('h', Direction::Left),
    ('j', Direction::Down),
    ('k', Direction::Up),
    ('l', Direction::Right),
];

const fn arrow(d: Direction) -> ChordKey {
    match d {
        Direction::Up => ChordKey::Up,
        Direction::Down => ChordKey::Down,
        Direction::Left => ChordKey::Left,
        Direction::Right => ChordKey::Right,
    }
}

const fn plain(key: ChordKey) -> Chord {
    Chord::new(key)
}

const fn shift(key: ChordKey) -> Chord {
    Chord::new(key).with(ChordMod::Shift)
}

const fn ctrl(key: ChordKey) -> Chord {
    Chord::new(key).with(ChordMod::Ctrl)
}

/// Whether `chord` is a plain printable key: one that types in a text field, and that
/// type-to-search claims.
fn typed(chord: &Chord) -> bool {
    let held = [
        ChordMod::Primary,
        ChordMod::Ctrl,
        ChordMod::Alt,
        ChordMod::Super,
    ];
    matches!(chord.key(), ChordKey::Char(_)) && !held.iter().any(|m| chord.has(*m))
}
