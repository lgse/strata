//! Which action a drag offers and a drop takes: GTK's modifier rule.
//!
//! - Ctrl copies, Shift moves, Ctrl+Shift links. A modifier asking for an action the source
//!   doesn't allow is ignored, as if no modifier were held.
//! - A plain drop takes the first of copy, move and link the source allows (GTK's order).
//! - A target's own [`DropTarget::action`](super::DropTarget::action) wins over the rule;
//!   an action it picks that the source doesn't allow is ignored, and the rule decides.
//!
//! Where the modifiers are known is a platform limit, recorded in the module docs: on
//! Wayland a drag from another client gives none, so its action is the compositor's.

use rune::Mods;

use super::api::{Actions, DragAction};

/// The action the modifiers ask for, if the source allows it.
pub fn modifier_action(mods: Option<Mods>, allowed: Actions) -> Option<DragAction> {
    let mods = mods?;
    let asked = match (mods.ctrl, mods.shift) {
        (true, true) => DragAction::Link,
        (true, false) => DragAction::Copy,
        (false, true) => DragAction::Move,
        (false, false) => return None,
    };
    allowed.has(asked).then_some(asked)
}

/// A plain drop's action: the first of copy, move and link the source allows.
pub fn plain_action(allowed: Actions) -> Option<DragAction> {
    allowed.iter().find(|a| *a != DragAction::Ask)
}

/// The action a drop takes: the target's pick if the source allows it (ask only if the
/// source allows ask), else the modifiers', else the plain one. None: the drop is refused.
pub fn drop_action(
    target: Option<Option<DragAction>>,
    allowed: Actions,
    mods: Option<Mods>,
) -> Option<DragAction> {
    match target {
        // The target refused the drag.
        Some(None) => None,
        Some(Some(a)) if allowed.has(a) => Some(a),
        _ => modifier_action(mods, allowed).or_else(|| plain_action(allowed)),
    }
}

/// The actions a drag offers at its start: the modifiers narrow the item's to the one they
/// ask for, if the item allows it. Ask stays offered either way.
pub fn source_actions(item: Actions, mods: Mods) -> Actions {
    let ask = item.intersect(Actions::ASK);
    match modifier_action(Some(mods), item) {
        Some(a) => Actions::of(a) | ask,
        None => item,
    }
}

/// The action a source proposes on X11 (`XdndPosition` carries one): the modifiers', else
/// the plain one.
pub fn proposed_action(allowed: Actions, mods: Mods) -> Option<DragAction> {
    modifier_action(Some(mods), allowed).or_else(|| plain_action(allowed))
}

#[cfg(test)]
mod tests;
