// SPDX-License-Identifier: MIT

use std::time::Duration;

use rune::{Chord, ChordKey, ChordMod, KeyEvent, KeyPlatform, Tx};

use super::time::{SEEK_PAGE, SEEK_STEP};

/// A seek a key asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Seek {
    Back(Duration),
    Forward(Duration),
    /// The start.
    Start,
    /// The last frame, so playback doesn't stop ([`last_frame`](super::time::last_frame)).
    End,
}

/// What a media key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaCommand {
    PlayPause,
    Seek(Seek),
    /// The volume up or down a step (0.1); reaching 0 mutes.
    VolumeUp,
    VolumeDown,
    /// Mute, or unmute to the kept volume.
    Mute,
    /// The previous or next file of the clip's kind.
    Previous,
    Next,
    /// Open in the default player (video).
    HandOff,
}

/// Where a binding applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyScope {
    Window,
    Focused,
}

/// One binding: a scope, a chord, the modifiers it matches with or without, its command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaBinding {
    pub scope: KeyScope,
    pub chord: Chord,
    /// Modifiers that may also be held: Strata's `ignoring(..)`.
    pub tolerates: Vec<ChordMod>,
    pub command: MediaCommand,
}

impl MediaBinding {
    fn variants(&self) -> impl Iterator<Item = Chord> + '_ {
        let n = self.tolerates.len();
        (1..1u32 << n).map(move |subset| {
            (0..n)
                .filter(|i| subset & (1 << i) != 0)
                .fold(self.chord, |c, i| c.with(self.tolerates[i]))
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaKeys {
    bindings: Vec<MediaBinding>,
}

const fn key(k: ChordKey) -> Chord {
    Chord::new(k)
}

const fn ctrl_alt(k: ChordKey) -> Chord {
    Chord::new(k).with(ChordMod::Ctrl).with(ChordMod::Alt)
}

const fn shift(k: ChordKey) -> Chord {
    Chord::new(k).with(ChordMod::Shift)
}

impl Default for MediaKeys {
    fn default() -> Self {
        Self::default_map()
    }
}

impl MediaKeys {
    /// No bindings.
    pub fn new() -> MediaKeys {
        MediaKeys {
            bindings: Vec::new(),
        }
    }

    /// Strata's default map.
    pub fn default_map() -> MediaKeys {
        use KeyScope::{Focused, Window};
        use MediaCommand::*;
        let mut m = MediaKeys::new();
        m.bind(Window, ctrl_alt(ChordKey::Char(' ')), PlayPause)
            .bind(
                Window,
                ctrl_alt(ChordKey::Left),
                Seek(self::Seek::Back(SEEK_STEP)),
            )
            .bind(
                Window,
                ctrl_alt(ChordKey::Right),
                Seek(self::Seek::Forward(SEEK_STEP)),
            )
            .bind(Window, ctrl_alt(ChordKey::Up), VolumeUp)
            .bind(Window, ctrl_alt(ChordKey::Down), VolumeDown)
            .bind(Window, ctrl_alt(ChordKey::Char('m')), Mute)
            .bind(Window, ctrl_alt(ChordKey::Char('<')), Previous)
            .bind(Window, ctrl_alt(ChordKey::Char('>')), Next)
            .bind(
                Focused,
                key(ChordKey::Left),
                Seek(self::Seek::Back(SEEK_STEP)),
            )
            .bind(
                Focused,
                key(ChordKey::Right),
                Seek(self::Seek::Forward(SEEK_STEP)),
            )
            .bind(
                Focused,
                shift(ChordKey::Left),
                Seek(self::Seek::Back(SEEK_PAGE)),
            )
            .bind(
                Focused,
                shift(ChordKey::Right),
                Seek(self::Seek::Forward(SEEK_PAGE)),
            )
            .bind(
                Focused,
                key(ChordKey::PageDown),
                Seek(self::Seek::Back(SEEK_PAGE)),
            )
            .bind(
                Focused,
                key(ChordKey::PageUp),
                Seek(self::Seek::Forward(SEEK_PAGE)),
            )
            // AZERTY types 0 with Shift.
            .bind_tolerating(
                Focused,
                key(ChordKey::Char('0')),
                &[ChordMod::Shift],
                Seek(self::Seek::Start),
            )
            .bind(Focused, key(ChordKey::Home), Seek(self::Seek::Start))
            .bind(Focused, shift(ChordKey::Char('g')), Seek(self::Seek::End))
            .bind(Focused, key(ChordKey::End), Seek(self::Seek::End))
            .bind(Focused, key(ChordKey::Char('+')), VolumeUp)
            .bind(Focused, key(ChordKey::Char('=')), VolumeUp)
            .bind(Focused, key(ChordKey::Char('-')), VolumeDown)
            .bind(Focused, key(ChordKey::Char(' ')), PlayPause)
            .bind(Focused, key(ChordKey::Up), VolumeUp)
            .bind(Focused, key(ChordKey::Down), VolumeDown)
            .bind(Focused, key(ChordKey::Enter), HandOff);
        m
    }

    /// Strata's 10xer map: the default map plus plain `m`, `<` and `>`.
    pub fn tenxer() -> MediaKeys {
        use KeyScope::{Focused, Window};
        use MediaCommand::*;
        let mut m = MediaKeys::default_map();
        m.bind(Focused, key(ChordKey::Char('m')), Mute)
            .bind(Focused, key(ChordKey::Char('<')), Previous)
            .bind(Focused, key(ChordKey::Char('>')), Next)
            .bind(Window, key(ChordKey::Char('<')), Previous)
            .bind(Window, key(ChordKey::Char('>')), Next);
        m
    }

    /// Binds `chord` to `command` in `scope`, after its other bindings.
    pub fn bind(&mut self, scope: KeyScope, chord: Chord, command: MediaCommand) -> &mut Self {
        self.bind_tolerating(scope, chord, &[], command)
    }

    /// Binds `chord`, matching with or without any of `tolerates` held too.
    pub fn bind_tolerating(
        &mut self,
        scope: KeyScope,
        chord: Chord,
        tolerates: &[ChordMod],
        command: MediaCommand,
    ) -> &mut Self {
        self.bindings.push(MediaBinding {
            scope,
            chord,
            tolerates: tolerates.to_vec(),
            command,
        });
        self
    }

    /// Removes every chord bound to `command`, in every scope.
    pub fn unbind(&mut self, command: MediaCommand) -> &mut Self {
        self.bindings.retain(|b| b.command != command);
        self
    }

    /// Removes the bindings of exactly `chord` in `scope`.
    pub fn unbind_chord(&mut self, scope: KeyScope, chord: Chord) -> &mut Self {
        let p = KeyPlatform::current();
        self.bindings
            .retain(|b| !(b.scope == scope && b.chord.same_keys(&chord, p)));
        self
    }

    /// Every binding, in order.
    pub fn bindings(&self) -> &[MediaBinding] {
        &self.bindings
    }

    /// The chords that run `command` in `scope`, in order: tooltips show the first.
    pub fn chords_for(&self, scope: KeyScope, command: MediaCommand) -> Vec<Chord> {
        self.bindings
            .iter()
            .filter(|b| b.scope == scope && b.command == command)
            .map(|b| b.chord)
            .collect()
    }

    pub fn window_command(&self, event: &KeyEvent, tx: &Tx) -> Option<MediaCommand> {
        self.lookup(KeyScope::Window, &|c| {
            c.matches_tx_on(event, tx, KeyPlatform::current())
        })
    }

    /// The command `event` runs while the player has the keys.
    pub fn focused_command(&self, event: &KeyEvent, tx: &Tx) -> Option<MediaCommand> {
        self.lookup(KeyScope::Focused, &|c| {
            c.matches_tx_on(event, tx, KeyPlatform::current())
        })
    }

    pub fn command_on(
        &self,
        scope: KeyScope,
        event: &KeyEvent,
        platform: KeyPlatform,
    ) -> Option<MediaCommand> {
        self.lookup(scope, &|c| c.matches_on(event, platform))
    }

    fn lookup(&self, scope: KeyScope, matches: &dyn Fn(&Chord) -> bool) -> Option<MediaCommand> {
        let scoped = || self.bindings.iter().filter(|b| b.scope == scope);
        scoped()
            .find(|b| matches(&b.chord))
            .or_else(|| scoped().find(|b| b.variants().any(|v| matches(&v))))
            .map(|b| b.command)
    }

    /// The first chord for `command` in `scope`, as a label ("Ctrl+Alt+Space"), for tooltips.
    pub fn label(&self, scope: KeyScope, command: MediaCommand) -> Option<String> {
        self.chords_for(scope, command).first().map(chord_label)
    }
}

/// A chord as Strata's tooltips write it: "Ctrl+Alt+Space", "Shift+G", "Ctrl+Alt+<".
pub fn chord_label(chord: &Chord) -> String {
    let p = KeyPlatform::current();
    let mut parts: Vec<String> = chord
        .mods_on(p)
        .into_iter()
        .map(|m| {
            match m {
                ChordMod::Primary | ChordMod::Ctrl => "Ctrl",
                ChordMod::Alt => "Alt",
                ChordMod::Shift => "Shift",
                ChordMod::Super => "Super",
            }
            .to_owned()
        })
        .collect();
    let key = match chord.key() {
        ChordKey::Char(' ') => "Space".to_owned(),
        ChordKey::Char(c) => c.to_uppercase().collect(),
        ChordKey::Enter => "Return".to_owned(),
        ChordKey::PageUp => "Page Up".to_owned(),
        ChordKey::PageDown => "Page Down".to_owned(),
        ChordKey::Function(n) => format!("F{n}"),
        other => format!("{other:?}"),
    };
    parts.push(key);
    parts.join("+")
}
