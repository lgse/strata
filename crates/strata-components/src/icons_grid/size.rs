// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Chord, ChordKey, ChordMod, KeyEvent, KeyPlatform, Tx, WheelEvent};

use crate::collection::element::{
    Ctx, Effect, Flow, Intent, KeyController, KeyInput, PointerInput,
};

#[cfg(test)]
mod tests;

/// What to do to the size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeCommand {
    /// Move this many steps along the ladder: positive is larger.
    Step(i32),
    /// Back to the size the folder should follow: the host's to say.
    Reset,
}

/// What the host hears when the size should change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeRequest {
    /// From `from` to `to` along the ladder.
    Step {
        /// The size it was.
        from: f32,
        /// The size it steps to.
        to: f32,
    },
    /// Reset: the host sets the folder's default size (Strata's three cases).
    Reset,
}

/// Which keys step and reset the size.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SizeKeys {
    /// The default map's chords for a step larger.
    pub larger: Vec<Chord>,
    /// The default map's chords for a step smaller.
    pub smaller: Vec<Chord>,
    /// The chords for a reset, in both maps.
    pub reset: Vec<Chord>,
    /// The 10xer map's extra keys for a step larger, with the grid focused.
    pub tenxer_larger: Vec<Chord>,
    /// The 10xer map's extra keys for a step smaller, with the grid focused.
    pub tenxer_smaller: Vec<Chord>,
    /// 10xer mode is on: the extra keys apply.
    pub tenxer: bool,
}

impl Default for SizeKeys {
    fn default() -> Self {
        Self::default_map()
    }
}

const fn ctrl_alt(key: ChordKey) -> Chord {
    Chord::new(key).with(ChordMod::Ctrl).with(ChordMod::Alt)
}

impl SizeKeys {
    /// Strata's keys.
    pub fn default_map() -> Self {
        Self {
            larger: vec![ctrl_alt(ChordKey::Char('+')), ctrl_alt(ChordKey::Char('='))],
            smaller: vec![ctrl_alt(ChordKey::Char('-'))],
            reset: vec![ctrl_alt(ChordKey::Char('0'))],
            tenxer_larger: vec![
                Chord::new(ChordKey::Char('+')),
                Chord::new(ChordKey::Char('=')),
            ],
            tenxer_smaller: vec![Chord::new(ChordKey::Char('-'))],
            tenxer: false,
        }
    }

    /// The same keys with 10xer mode on or off.
    pub fn with_tenxer(mut self, on: bool) -> Self {
        self.tenxer = on;
        self
    }

    /// Rebinds `command`'s default chords: `Step(n)` with `n > 0` the larger ones, `n < 0` the smaller, `Reset` the reset.
    pub fn bind(&mut self, command: SizeCommand, chords: Vec<Chord>) {
        match command {
            SizeCommand::Step(n) if n > 0 => self.larger = chords,
            SizeCommand::Step(n) if n < 0 => self.smaller = chords,
            SizeCommand::Step(_) => {}
            SizeCommand::Reset => self.reset = chords,
        }
    }

    fn find(&self, matches: impl Fn(&Chord) -> bool, focused: bool) -> Option<SizeCommand> {
        let any = |chords: &[Chord]| chords.iter().any(&matches);
        let tenxer = focused && self.tenxer;
        if any(&self.larger) || (tenxer && any(&self.tenxer_larger)) {
            Some(SizeCommand::Step(1))
        } else if any(&self.smaller) || (tenxer && any(&self.tenxer_smaller)) {
            Some(SizeCommand::Step(-1))
        } else if any(&self.reset) {
            Some(SizeCommand::Reset)
        } else {
            None
        }
    }

    /// The command a window key is: the default map's chords only.
    pub fn window_command(&self, event: &KeyEvent, tx: &Tx) -> Option<SizeCommand> {
        let platform = KeyPlatform::current();
        self.find(|c| c.matches_tx_on(event, tx, platform), false)
    }

    /// The command a key on the focused grid is: in 10xer mode, its plain keys too.
    pub fn listing_command(&self, input: &KeyInput<'_>) -> Option<SizeCommand> {
        self.find(|c| input.matches(c), true)
    }
}

/// The command an intent from `SizeKeyed` carries.
pub fn size_command(intent: &Intent) -> Option<SizeCommand> {
    match intent {
        Intent::Other(any) => any.downcast_ref::<SizeCommand>().copied(),
        _ => None,
    }
}

/// A keyboard controller that takes the size keys first, then hands every other key to the controller it wraps.
pub struct SizeKeyed {
    inner: Box<dyn KeyController>,
}

impl std::fmt::Debug for SizeKeyed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SizeKeyed").finish_non_exhaustive()
    }
}

impl SizeKeyed {
    /// `inner` with the size keys taken first.
    pub fn new(inner: Box<dyn KeyController>) -> Self {
        Self { inner }
    }
}

impl KeyController for SizeKeyed {
    fn key(&mut self, input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow {
        let fallback = SizeKeys::default_map();
        let keys = cx.settings::<SizeKeys>().unwrap_or(&fallback);
        match keys.listing_command(input) {
            Some(command) => {
                cx.emit(Effect::Intent(Intent::Other(Rc::new(command))));
                Flow::Handled
            }
            None => self.inner.key(input, cx),
        }
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        self.inner.tick(cx)
    }

    fn pointer_seen(&mut self, input: &PointerInput) {
        self.inner.pointer_seen(input);
    }

    fn reset(&mut self) {
        self.inner.reset();
    }
}

/// Turns Ctrl+Shift+wheel into whole steps, as Strata's `WheelZoom` does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelSteps {
    pending: f32,
    direction: i8,
    precise: Option<bool>,
}

/// Points of precise scrolling per step: Strata's 10 surface pixels.
pub const PRECISE_STEP: f32 = 10.0;

impl WheelSteps {
    /// Whether `e` is a size wheel: Ctrl and Shift held, and neither Alt nor Super.
    pub fn claims(e: &WheelEvent) -> bool {
        let m = e.mods;
        m.ctrl && m.shift && !m.alt && !m.logo
    }

    /// Forgets what was left over.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The steps `e` makes: positive is larger.
    pub fn steps(&mut self, e: &WheelEvent) -> i32 {
        let delta = if e.dy == 0.0 { e.dx } else { e.dy };
        if delta == 0.0 || !delta.is_finite() {
            return 0;
        }
        let direction = if delta > 0.0 { 1 } else { -1 };
        if self.precise != Some(e.precise) || self.direction != direction {
            self.pending = 0.0;
        }
        self.precise = Some(e.precise);
        self.direction = direction;
        if !e.precise {
            return i32::from(direction);
        }
        let total = self.pending + delta / PRECISE_STEP;
        let steps = total.trunc();
        self.pending = total - steps;
        steps as i32
    }
}
