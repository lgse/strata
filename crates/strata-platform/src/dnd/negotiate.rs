// SPDX-License-Identifier: MIT

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

pub fn source_actions(item: Actions, mods: Mods) -> Actions {
    let ask = item.intersect(Actions::ASK);
    match modifier_action(Some(mods), item) {
        Some(a) => Actions::of(a) | ask,
        None => item,
    }
}

pub fn proposed_action(allowed: Actions, mods: Mods) -> Option<DragAction> {
    modifier_action(Some(mods), allowed).or_else(|| plain_action(allowed))
}

#[cfg(test)]
mod tests;
