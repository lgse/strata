//! The keyboard machine on C1e's seam: [`Keyboard`] is the collection's `KeyController`.

use std::rc::Rc;

use super::machine::{KeyMachine, Outcome, Surface};
use super::settings::{KeyHandle, KeySettings};
use rune::PointerButton;

use crate::collection::element::{
    Activation, Ctx, Effect, Flow, Intent, KeyController, KeyInput, PointerInput, PointerPhase,
};
use crate::collection::selection::{Changed, Selection};
use crate::contract::{CollectionGeometry, EntrySource};

/// What a 10xer selection key in an empty folder sends the host, as `Intent::Other`: the host
/// flashes `Nothing to select` (C18). Downcast the intent's value to this type.
///
/// Used by: C18, hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NothingToSelect;

/// Keyboard navigation as the collection's keyboard controller: install it in
/// `CollectionState::controllers.keyboard`, pass [`KeySettings`] with `Collection::settings`,
/// and keep a clone of its [`KeyHandle`] for the 10xer range and queued commands.
///
/// ```ignore
/// state.controllers.keyboard = Some(Box::new(Keyboard::new(handle.clone())));
/// Collection::new(/* … */).settings(KeySettings::tenxer())
/// ```
///
/// What it sends the element:
///
/// | Machine | Element |
/// | --- | --- |
/// | [`Outcome::Reveal`] | `Effect::Reveal` |
/// | [`Outcome::Activate`] | `Intent::Activate` with `Activation::Key` |
/// | [`Outcome::Edge`] | `Intent::Edge` |
/// | [`Outcome::NothingToSelect`] | `Intent::Other` holding [`NothingToSelect`] |
/// | [`Outcome::Keyboard`] | `Effect::KeyboardMode(true)` |
///
/// A key that moves the cursor turns keyboard mode on in the element itself. Ctrl+A and
/// Escape, which move none, say so with [`Outcome::Keyboard`], and a queued command that moves
/// the cursor (from [`KeyHandle::run`], at `tick`) does too. The adapter sends at most one
/// `KeyboardMode` an input, and only with a selection change or a cursor move, which commit
/// the view anyway, so the mode costs no extra commit. A primary press ends a 10xer range
/// (`pointer_seen`); a folder change (`reset`) drops the range, the type-ahead prefix and any
/// queued commands.
///
/// **Type-ahead needs the collection's clock** (`Collection::clock`): its timeout compares the
/// keys' times, and without a clock every key comes at 0, so a prefix never times out.
///
/// Used by: hosts, C2, C3, C4.
#[derive(Debug, Default)]
pub struct Keyboard {
    machine: KeyMachine,
    /// The settings when the host passed none.
    defaults: KeySettings,
}

impl Keyboard {
    /// A controller sharing `handle` with the host.
    pub fn new(handle: KeyHandle) -> Self {
        Self {
            machine: KeyMachine::new(handle),
            defaults: KeySettings::default(),
        }
    }

    /// The handle it shares with the host.
    pub fn handle(&self) -> &KeyHandle {
        self.machine.handle()
    }
}

impl KeyController for Keyboard {
    fn key(&mut self, input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow {
        let mut s = CtxSurface {
            cx,
            defaults: &self.defaults,
            mode_sent: false,
        };
        if self.machine.key(input, &mut s) {
            Flow::Handled
        } else {
            Flow::Pass
        }
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        let before = cx.selection().cursor();
        let mut s = CtxSurface {
            cx: &mut *cx,
            defaults: &self.defaults,
            mode_sent: false,
        };
        if !self.machine.tick(&mut s) {
            return Flow::Pass;
        }
        if s.cx.selection().cursor() != before {
            s.keyboard_mode();
        }
        Flow::Handled
    }

    fn pointer_seen(&mut self, input: &PointerInput) {
        if input.phase == PointerPhase::Down && input.button == PointerButton::Primary {
            self.machine.pressed();
        }
    }

    fn reset(&mut self) {
        self.machine.reset();
    }
}

/// C1e's `Ctx` as the machine's surface.
struct CtxSurface<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    defaults: &'c KeySettings,
    /// Whether this input already sent `KeyboardMode(true)`.
    mode_sent: bool,
}

impl CtxSurface<'_, '_> {
    fn keyboard_mode(&mut self) {
        if !self.mode_sent {
            self.mode_sent = true;
            self.cx.emit(Effect::KeyboardMode(true));
        }
    }
}

impl Surface for CtxSurface<'_, '_> {
    fn settings(&self) -> &KeySettings {
        self.cx.settings::<KeySettings>().unwrap_or(self.defaults)
    }

    fn source(&self) -> &dyn EntrySource {
        self.cx.source
    }

    fn geometry(&self) -> &dyn CollectionGeometry {
        self.cx.geometry
    }

    fn selection(&self) -> &Selection {
        self.cx.selection()
    }

    fn apply(
        &mut self,
        op: &mut dyn FnMut(&mut Selection, &dyn EntrySource) -> Changed,
    ) -> Changed {
        self.cx.apply(|sel, src| op(sel, src))
    }

    fn emit(&mut self, outcome: Outcome) {
        let effect = match outcome {
            Outcome::Reveal(i) => Effect::Reveal(i),
            Outcome::Activate { id } => Effect::Intent(Intent::Activate {
                id,
                how: Activation::Key,
            }),
            Outcome::Edge(d) => Effect::Intent(Intent::Edge(d)),
            Outcome::NothingToSelect => Effect::Intent(Intent::Other(Rc::new(NothingToSelect))),
            Outcome::Keyboard => return self.keyboard_mode(),
        };
        self.cx.emit(effect);
    }
}
