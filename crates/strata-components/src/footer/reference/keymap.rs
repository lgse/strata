//! Rows from a C1c [`KeyMap`], and chord labels as Strata writes them.

use rune::{Chord, ChordKey, ChordMod, KeyPlatform};

use super::data::{Keys, Shortcut};
use crate::collection::keyboard::{Command, KeyMap, Letters, Target};
use crate::contract::Direction;

/// The rows of `map`, in the order its commands are first bound: one row per command that
/// `describe` names, listing every chord bound to it (`label` writes each). A command with no
/// chord has no row; one `describe` leaves out (`Command::Swallow`) neither.
///
/// Aliases (`h j k l` for the arrows) don't join their commands' rows: with
/// [`Letters::Arrows`], the only setting they fire under, they make one row of their own,
/// named `aliases` ("Same as ← ↓ ↑ → (type-to-search off)", Strata's wording), placed after
/// the row of the first command they stand in for, in the order of the arrows they stand for. Without `aliases`, or with other
/// [`Letters`], they show nowhere. Tolerated chords (Super with Page Down) aren't listed, as in
/// Strata. An alias whose chord [`KeyMap::unbind_chord`] didn't remove still fires, so it stays
/// listed: the reference shows what runs.
///
/// Used by: hosts building the navigation section of the reference from the map they run.
pub fn keymap_rows(
    map: &KeyMap,
    letters: Letters,
    describe: impl Fn(Command) -> Option<String>,
    aliases: Option<&str>,
    label: impl Fn(&Chord) -> String,
) -> Vec<Shortcut> {
    let mut commands: Vec<Command> = Vec::new();
    for b in map.bindings() {
        if !commands.contains(&b.command) {
            commands.push(b.command);
        }
    }
    let alias_chords = |command: Command| -> Vec<Chord> {
        let bound: Vec<Chord> = map
            .bindings()
            .iter()
            .filter(|b| b.command == command)
            .map(|b| b.chord)
            .collect();
        map.chords_for(command)
            .into_iter()
            .filter(|c| !bound.contains(c))
            .collect()
    };
    let mut aliases_found: Vec<(Chord, Command)> = Vec::new();
    let mut alias_after: Option<usize> = None;
    let mut rows = Vec::new();
    for command in commands {
        let Some(action) = describe(command) else {
            continue;
        };
        let mut chords: Vec<String> = Vec::new();
        for b in map.bindings().iter().filter(|b| b.command == command) {
            let l = label(&b.chord);
            if !chords.contains(&l) {
                chords.push(l);
            }
        }
        if chords.is_empty() {
            continue;
        }
        rows.push(Shortcut::new(action, Keys::any(chords)));
        for chord in alias_chords(command) {
            alias_after.get_or_insert(rows.len());
            if !aliases_found.iter().any(|(c, _)| *c == chord) {
                aliases_found.push((chord, command));
            }
        }
    }
    if let (Some(name), Some(at), Letters::Arrows) = (aliases, alias_after, letters) {
        // In the order of the arrows they stand for, as the row's words read them: ← ↓ ↑ →.
        aliases_found.sort_by_key(|(_, command)| arrow_rank(*command));
        let keys = Keys::any(aliases_found.iter().map(|(c, _)| label(c)));
        rows.insert(at, Shortcut::new(name, keys));
    }
    rows
}

/// Where an alias of `command` sits in the alias row: by the arrow its step goes, ← ↓ ↑ →
/// (`h j k l`), then the rest.
fn arrow_rank(command: Command) -> usize {
    let step = match command {
        Command::Select(Target::Step(d))
        | Command::Extend(Target::Step(d))
        | Command::Cursor(Target::Step(d)) => Some(d),
        Command::Nudge(d) => Some(d),
        _ => None,
    };
    match step {
        Some(Direction::Left) => 0,
        Some(Direction::Down) => 1,
        Some(Direction::Up) => 2,
        Some(Direction::Right) => 3,
        None => 4,
    }
}

/// A chord as Strata labels it in English (`app/keyboard-shortcuts`, Labels): modifiers in the
/// order Ctrl, Alt, Shift, Super; a plain letter as typed (`j`), Shift with a letter alone as
/// the capital (`J`), a letter after another modifier as a capital (`Ctrl+Shift+N`); a symbol
/// that needs Shift as the symbol (`~`); minus as `−`; the arrows as `↑ ↓ ← →`; Escape and
/// Space by name. Strata's labels follow the interface language (German Strg, Bild auf); a
/// host in another language passes its own.
pub fn chord_label(chord: &Chord) -> String {
    let shift = chord.has(ChordMod::Shift);
    let others = chord
        .mods_on(KeyPlatform::Linux)
        .into_iter()
        .any(|m| m != ChordMod::Shift);
    if let ChordKey::Char(c) = chord.key()
        && !others
    {
        if c.is_alphabetic() {
            return if shift {
                c.to_uppercase().collect()
            } else {
                c.to_string()
            };
        }
        if shift && let Some(s) = us_shifted(c) {
            return s.to_string();
        }
    }
    let mut caps = chord.keycaps_on(KeyPlatform::Linux);
    if let Some(last) = caps.last_mut() {
        *last = match last.as_str() {
            "Esc" => "Escape".to_owned(),
            "-" => "\u{2212}".to_owned(),
            _ => last.clone(),
        };
    }
    caps.join("+")
}

/// The character Shift types with `c` on a US layout.
fn us_shifted(c: char) -> Option<char> {
    const PAIRS: &[(char, char)] = &[
        ('`', '~'),
        ('1', '!'),
        ('2', '@'),
        ('3', '#'),
        ('4', '$'),
        ('5', '%'),
        ('6', '^'),
        ('7', '&'),
        ('8', '*'),
        ('9', '('),
        ('0', ')'),
        ('-', '_'),
        ('=', '+'),
        ('[', '{'),
        (']', '}'),
        ('\\', '|'),
        (';', ':'),
        ('\'', '"'),
        (',', '<'),
        ('.', '>'),
        ('/', '?'),
    ];
    PAIRS.iter().find(|p| p.0 == c).map(|p| p.1)
}
