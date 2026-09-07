// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    CrossVolumeDropStrategy, DropActionInput, DropCommit, DropOverride, TransferKind,
    VolumeIdentity, VolumeRelation, drop_commit, drop_is_noop, volume_relation,
};
use crate::model::Location;

fn identity(id: &str, is_remote: bool) -> VolumeIdentity {
    VolumeIdentity {
        filesystem_id: id.into(),
        is_remote,
    }
}

fn input(
    can_copy: bool,
    can_move: bool,
    volume: VolumeRelation,
    override_with: DropOverride,
    strategy: CrossVolumeDropStrategy,
) -> DropActionInput {
    DropActionInput {
        can_copy,
        can_move,
        volume,
        override_with,
        strategy,
    }
}

fn kind(
    can_copy: bool,
    can_move: bool,
    volume: VolumeRelation,
    override_with: DropOverride,
) -> TransferKind {
    drop_commit(input(
        can_copy,
        can_move,
        volume,
        override_with,
        CrossVolumeDropStrategy::Ask,
    ))
    .transfer_kind()
}

fn commit(
    can_copy: bool,
    can_move: bool,
    volume: VolumeRelation,
    override_with: DropOverride,
    strategy: CrossVolumeDropStrategy,
) -> DropCommit {
    drop_commit(input(can_copy, can_move, volume, override_with, strategy))
}

#[test]
fn volume_relation_treats_empty_sources_as_unknown() {
    let dest = identity("dev:1", false);
    assert_eq!(volume_relation(Some(&dest), &[]), VolumeRelation::Unknown);
}

#[test]
fn volume_relation_is_unknown_when_dest_or_any_source_is_missing() {
    let dest = identity("dev:1", false);
    let same = identity("dev:1", false);
    assert_eq!(
        volume_relation(None, &[Some(same.clone())]),
        VolumeRelation::Unknown
    );
    assert_eq!(
        volume_relation(Some(&dest), &[Some(same), None]),
        VolumeRelation::Unknown
    );
}

#[test]
fn volume_relation_is_different_if_any_source_disagrees() {
    let dest = identity("dev:1", false);
    let same = identity("dev:1", false);
    let other = identity("dev:2", false);
    assert_eq!(
        volume_relation(Some(&dest), &[Some(same.clone())]),
        VolumeRelation::Same
    );
    assert_eq!(
        volume_relation(Some(&dest), &[Some(same), Some(other)]),
        VolumeRelation::Different
    );
}

#[test]
fn volume_relation_treats_remote_flag_mismatch_as_different() {
    let local = identity("same-id", false);
    let remote = identity("same-id", true);
    assert_eq!(
        volume_relation(Some(&local), &[Some(remote)]),
        VolumeRelation::Different
    );
}

#[test]
fn preferred_kind_copies_across_volumes_and_unknown() {
    assert_eq!(
        kind(true, true, VolumeRelation::Same, DropOverride::None),
        TransferKind::Move
    );
    assert_eq!(
        kind(true, true, VolumeRelation::Different, DropOverride::None),
        TransferKind::Copy
    );
    assert_eq!(
        kind(true, true, VolumeRelation::Unknown, DropOverride::None),
        TransferKind::Copy
    );
}

#[test]
fn preferred_kind_honors_ctrl_copy_and_shift_move() {
    assert_eq!(
        kind(true, true, VolumeRelation::Same, DropOverride::ForceCopy),
        TransferKind::Copy
    );
    assert_eq!(
        kind(
            true,
            true,
            VolumeRelation::Different,
            DropOverride::ForceMove
        ),
        TransferKind::Move
    );
}

#[test]
fn preferred_kind_ignores_override_that_is_not_offered() {
    assert_eq!(
        kind(false, true, VolumeRelation::Same, DropOverride::ForceCopy),
        TransferKind::Move
    );
    assert_eq!(
        kind(
            true,
            false,
            VolumeRelation::Different,
            DropOverride::ForceMove
        ),
        TransferKind::Copy
    );
}

#[test]
fn preferred_kind_falls_back_when_only_one_action_is_offered() {
    assert_eq!(
        kind(false, true, VolumeRelation::Different, DropOverride::None),
        TransferKind::Move
    );
    assert_eq!(
        kind(true, false, VolumeRelation::Same, DropOverride::None),
        TransferKind::Copy
    );
    assert_eq!(
        kind(false, false, VolumeRelation::Same, DropOverride::None),
        TransferKind::Forbidden
    );
}

#[test]
fn drop_commit_follows_cross_volume_strategy() {
    use CrossVolumeDropStrategy::{Ask, Copy, Move};
    use DropOverride::None;
    use VolumeRelation::{Different, Same, Unknown};

    let both = (true, true);
    let copy_only = (true, false);
    let move_only = (false, true);
    let neither = (false, false);
    let ask_copy = DropCommit::Ask {
        default: TransferKind::Copy,
    };

    let cases = [
        (both, Same, None, Ask, DropCommit::Move),
        (both, Same, None, Copy, DropCommit::Move),
        (both, Same, None, Move, DropCommit::Move),
        (both, Different, None, Ask, ask_copy),
        (both, Unknown, None, Ask, ask_copy),
        (both, Different, None, Copy, DropCommit::Copy),
        (both, Unknown, None, Copy, DropCommit::Copy),
        (both, Different, None, Move, DropCommit::Move),
        (both, Unknown, None, Move, DropCommit::Move),
        (
            both,
            Different,
            DropOverride::ForceCopy,
            Ask,
            DropCommit::Copy,
        ),
        (
            both,
            Different,
            DropOverride::ForceMove,
            Ask,
            DropCommit::Move,
        ),
        (
            both,
            Different,
            DropOverride::ForceCopy,
            Move,
            DropCommit::Copy,
        ),
        (
            both,
            Different,
            DropOverride::ForceMove,
            Copy,
            DropCommit::Move,
        ),
        (both, Same, DropOverride::ForceCopy, Move, DropCommit::Copy),
        (copy_only, Different, None, Ask, DropCommit::Copy),
        (move_only, Different, None, Ask, DropCommit::Move),
        (copy_only, Different, None, Move, DropCommit::Copy),
        (move_only, Different, None, Copy, DropCommit::Move),
        (
            copy_only,
            Different,
            DropOverride::ForceMove,
            Ask,
            DropCommit::Copy,
        ),
        (
            move_only,
            Different,
            DropOverride::ForceCopy,
            Ask,
            DropCommit::Move,
        ),
        (neither, Different, None, Ask, DropCommit::Forbidden),
        (neither, Same, None, Copy, DropCommit::Forbidden),
    ];

    for ((can_copy, can_move), volume, override_with, strategy, expected) in cases {
        assert_eq!(
            commit(can_copy, can_move, volume, override_with, strategy),
            expected,
            "copy={can_copy} move={can_move} volume={volume:?} override={override_with:?} strategy={strategy:?}"
        );
        assert_eq!(
            drop_commit(input(can_copy, can_move, volume, override_with, strategy)).transfer_kind(),
            expected.transfer_kind()
        );
    }
}

#[test]
fn cross_volume_strategy_parses_stored_values() {
    assert_eq!(
        CrossVolumeDropStrategy::parse("always-copy"),
        CrossVolumeDropStrategy::Copy
    );
    assert_eq!(
        CrossVolumeDropStrategy::parse("always-move"),
        CrossVolumeDropStrategy::Move
    );
    assert_eq!(
        CrossVolumeDropStrategy::parse("always-ask"),
        CrossVolumeDropStrategy::Ask
    );
    assert_eq!(
        CrossVolumeDropStrategy::parse("unknown"),
        CrossVolumeDropStrategy::Ask
    );
    assert_eq!(CrossVolumeDropStrategy::Copy.as_str(), "always-copy");
    assert_eq!(CrossVolumeDropStrategy::Move.as_str(), "always-move");
    assert_eq!(CrossVolumeDropStrategy::Ask.as_str(), "always-ask");
}

#[test]
fn drop_is_noop_for_self_same_name_and_descendant() {
    let source = Location::local("/fixture/source");
    let parent = Location::local("/fixture");
    let nested = Location::local("/fixture/source/nested");
    let elsewhere = Location::local("/elsewhere");

    assert!(drop_is_noop(&parent, std::slice::from_ref(&source)));
    assert!(drop_is_noop(&source, std::slice::from_ref(&source)));
    assert!(drop_is_noop(&nested, std::slice::from_ref(&source)));
    assert!(!drop_is_noop(&elsewhere, std::slice::from_ref(&source)));
    assert!(!drop_is_noop(&parent, &[]));
}
