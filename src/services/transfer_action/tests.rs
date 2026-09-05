// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    DropActionInput, DropOverride, TransferKind, VolumeIdentity, VolumeRelation, drop_is_noop,
    preferred_transfer_kind, volume_relation,
};
use crate::model::Location;

fn identity(id: &str, is_remote: bool) -> VolumeIdentity {
    VolumeIdentity {
        filesystem_id: id.into(),
        is_remote,
    }
}

fn kind(
    can_copy: bool,
    can_move: bool,
    volume: VolumeRelation,
    override_with: DropOverride,
) -> TransferKind {
    preferred_transfer_kind(DropActionInput {
        can_copy,
        can_move,
        volume,
        override_with,
    })
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
