//! The pointer machine on the collection element's seam (C1e): [`PointerIntent`] is the
//! `PointerController` a host installs, and a `Ctx` is the machine's [`Surface`].

use rune::PointerButton;

use super::machine::{ActivateBy, Button, Outcome, PointerMachine, Press, Release, Spot, Surface};
use super::settings::PointerSettings;
use crate::collection::element::seam::{
    Activation, Ctx, Effect, Flow, Intent, KeyInput, PointerController, PointerInput, PointerPhase,
    Region,
};
use crate::collection::selection::{Changed, Selection};
use crate::contract::{EntrySource, Rect};

/// Pointer intent for one collection: install it as the collection's pointer controller.
///
/// ```ignore
/// coll.controllers.pointer = Some(Box::new(PointerIntent::new()));
/// // and, as the view builds:
/// Collection::new(..).settings(PointerSettings::default().modes(modes))
/// ```
///
/// It reads its [`PointerSettings`] from `Collection::settings` on every input, the default
/// when the host gave none, and turns a read-only collection's drags and renames off.
///
/// It needs the element's clock (`Collection::clock`): it times double clicks and the
/// slow-click rename by it. With none, every input reads time 0, so any second press on an
/// entry is a double click and a rename never comes due.
///
/// Used by: C2, C3, C4, C14b, and hosts.
#[derive(Debug, Default)]
pub struct PointerIntent {
    machine: PointerMachine,
}

impl PointerIntent {
    /// A controller with no gesture held.
    pub fn new() -> Self {
        Self::default()
    }
}

impl PointerController for PointerIntent {
    fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
        let taken = match input.phase {
            PointerPhase::Down => {
                let Some(button) = button(input.button) else {
                    return Flow::Pass;
                };
                let press = Press {
                    button,
                    at: input.point,
                    window: input.window,
                    index: input.hit.index,
                    spot: spot(input.hit.region),
                    mods: input.mods,
                    now: input.now,
                };
                self.machine.press(&press, cx)
            }
            PointerPhase::Move => self.machine.moved(input.travel, cx),
            PointerPhase::Up => {
                let release = Release {
                    travel: input.travel,
                    mods: input.mods,
                    now: input.now,
                    cancelled: input.cancelled,
                };
                self.machine.release(&release, cx)
            }
        };
        if taken { Flow::Handled } else { Flow::Pass }
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        let now = cx.now;
        if self.machine.tick(now, cx) {
            Flow::Handled
        } else {
            Flow::Pass
        }
    }

    fn key_seen(&mut self, _input: &KeyInput<'_>) {
        self.machine.interrupt();
    }

    fn reset(&mut self) {
        self.machine.reset();
    }
}

fn button(button: PointerButton) -> Option<Button> {
    match button {
        PointerButton::Primary => Some(Button::Primary),
        PointerButton::Middle => Some(Button::Middle),
        PointerButton::Secondary => Some(Button::Secondary),
        _ => None,
    }
}

fn spot(region: Region) -> Spot {
    match region {
        Region::Name => Spot::Name,
        Region::Content => Spot::Content,
        _ => Spot::Gutter,
    }
}

/// The machine's outcome as the element's effect.
fn effect(outcome: Outcome) -> Effect {
    let intent = match outcome {
        Outcome::BeginMarquee { at, mods } => return Effect::BeginMarquee { at, mods },
        Outcome::WakeAfter(d) => return Effect::WakeAfter(d),
        Outcome::Activate { id, how } => Intent::Activate {
            id,
            how: match how {
                ActivateBy::Click => Activation::Click,
                ActivateBy::DoubleClick => Activation::DoubleClick,
            },
        },
        Outcome::Clicked { id } => Intent::Clicked { id },
        Outcome::OpenInNew { id, window } => Intent::OpenInNew { id, window },
        Outcome::BeginDrag { ids, at } => Intent::BeginDrag { ids, window: at },
        Outcome::BeginRename { id } => Intent::BeginRename { id },
        Outcome::ContextMenu { target, at } => Intent::ContextMenu {
            target,
            anchor: Rect::new(at.x, at.y, 0.0, 0.0),
        },
    };
    Effect::Intent(intent)
}

impl Surface for Ctx<'_> {
    fn settings(&self) -> PointerSettings {
        let mut settings = Ctx::settings::<PointerSettings>(self)
            .copied()
            .unwrap_or_default();
        if self.read_only() {
            settings.rename = false;
            settings.drags = false;
        }
        settings
    }

    fn source(&self) -> &dyn EntrySource {
        self.source
    }

    fn selection(&self) -> &Selection {
        Ctx::selection(self)
    }

    fn apply(
        &mut self,
        op: &mut dyn FnMut(&mut Selection, &dyn EntrySource) -> Changed,
    ) -> Changed {
        Ctx::apply(self, |sel, src| op(sel, src))
    }

    fn emit(&mut self, outcome: Outcome) {
        Ctx::emit(self, effect(outcome));
    }
}
