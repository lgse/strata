use rune::Mods;

use super::*;

const COPY_MOVE: Actions = Actions::COPY.union(Actions::MOVE);
const ALL: Actions = Actions::COPY
    .union(Actions::MOVE)
    .union(Actions::LINK)
    .union(Actions::ASK);

fn mods(ctrl: bool, shift: bool) -> Option<Mods> {
    Some(Mods {
        ctrl,
        shift,
        ..Mods::default()
    })
}

#[test]
fn ctrl_copies_shift_moves_and_both_link() {
    assert_eq!(
        drop_action(None, ALL, mods(true, false)),
        Some(DragAction::Copy)
    );
    assert_eq!(
        drop_action(None, ALL, mods(false, true)),
        Some(DragAction::Move)
    );
    assert_eq!(
        drop_action(None, ALL, mods(true, true)),
        Some(DragAction::Link)
    );
}

#[test]
fn a_modifier_for_an_action_the_source_lacks_is_ignored() {
    // Shift asks for move, which a copy-only source lacks: the plain action stands.
    assert_eq!(
        drop_action(None, Actions::COPY, mods(false, true)),
        Some(DragAction::Copy)
    );
    // Ctrl+Shift asks for a link; Wayland sources never offer one.
    assert_eq!(
        drop_action(None, COPY_MOVE, mods(true, true)),
        Some(DragAction::Copy)
    );
}

#[test]
fn a_plain_drop_takes_copy_then_move_then_link() {
    assert_eq!(
        drop_action(None, ALL, mods(false, false)),
        Some(DragAction::Copy)
    );
    assert_eq!(drop_action(None, ALL, None), Some(DragAction::Copy));
    let move_link = Actions::MOVE | Actions::LINK;
    assert_eq!(drop_action(None, move_link, None), Some(DragAction::Move));
    assert_eq!(
        drop_action(None, Actions::ASK, None),
        None,
        "ask alone isn't plain"
    );
    assert_eq!(drop_action(None, Actions::NONE, None), None);
}

#[test]
fn the_targets_pick_wins_over_the_modifiers() {
    assert_eq!(
        drop_action(Some(Some(DragAction::Move)), ALL, mods(true, false)),
        Some(DragAction::Move)
    );
    assert_eq!(
        drop_action(Some(Some(DragAction::Ask)), ALL, None),
        Some(DragAction::Ask)
    );
    assert_eq!(
        drop_action(Some(None), ALL, None),
        None,
        "the target refused"
    );
    // A pick the source lacks is ignored: the rule decides.
    assert_eq!(
        drop_action(Some(Some(DragAction::Ask)), COPY_MOVE, mods(false, true)),
        Some(DragAction::Move)
    );
}

#[test]
fn modifiers_at_the_start_narrow_what_a_drag_offers() {
    let m = |c, s| mods(c, s).expect("mods");
    assert_eq!(
        source_actions(ALL, m(true, false)),
        Actions::COPY | Actions::ASK
    );
    assert_eq!(source_actions(COPY_MOVE, m(false, true)), Actions::MOVE);
    assert_eq!(source_actions(COPY_MOVE, m(false, false)), COPY_MOVE);
    // A link the item doesn't allow: everything it allows stays.
    assert_eq!(source_actions(COPY_MOVE, m(true, true)), COPY_MOVE);
}

#[test]
fn an_x11_source_proposes_the_modifiers_action() {
    let m = |c, s| mods(c, s).expect("mods");
    assert_eq!(
        proposed_action(COPY_MOVE, m(false, true)),
        Some(DragAction::Move)
    );
    assert_eq!(
        proposed_action(COPY_MOVE, m(false, false)),
        Some(DragAction::Copy)
    );
    assert_eq!(
        proposed_action(Actions::MOVE, m(true, false)),
        Some(DragAction::Move)
    );
}
