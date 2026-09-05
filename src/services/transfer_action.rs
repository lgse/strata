// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use gio::prelude::*;

use crate::model::Location;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VolumeIdentity {
    pub filesystem_id: String,
    pub is_remote: bool,
}

impl VolumeIdentity {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        !self.filesystem_id.is_empty()
            && self.filesystem_id == other.filesystem_id
            && self.is_remote == other.is_remote
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VolumeRelation {
    Same,
    Different,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DropOverride {
    None,
    ForceCopy,
    ForceMove,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransferKind {
    Copy,
    Move,
    Forbidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DropActionInput {
    pub can_copy: bool,
    pub can_move: bool,
    pub volume: VolumeRelation,
    pub override_with: DropOverride,
}

/// Unknown if dest or any source identity is missing (URI hover, or a timed-out
/// remote query). Same only when sources is non-empty and every source matches.
pub(crate) fn volume_relation(
    dest: Option<&VolumeIdentity>,
    sources: &[Option<VolumeIdentity>],
) -> VolumeRelation {
    if sources.is_empty() {
        return VolumeRelation::Unknown;
    }
    let Some(dest) = dest else {
        return VolumeRelation::Unknown;
    };
    let mut any_different = false;
    for source in sources {
        let Some(source) = source else {
            return VolumeRelation::Unknown;
        };
        if !dest.matches(source) {
            any_different = true;
        }
    }
    if any_different {
        VolumeRelation::Different
    } else {
        VolumeRelation::Same
    }
}

pub(crate) fn preferred_transfer_kind(input: DropActionInput) -> TransferKind {
    if !input.can_copy && !input.can_move {
        return TransferKind::Forbidden;
    }
    match input.override_with {
        DropOverride::ForceCopy if input.can_copy => return TransferKind::Copy,
        DropOverride::ForceMove if input.can_move => return TransferKind::Move,
        DropOverride::ForceCopy | DropOverride::ForceMove | DropOverride::None => {}
    }
    match input.volume {
        VolumeRelation::Same => {
            if input.can_move {
                TransferKind::Move
            } else {
                TransferKind::Copy
            }
        }
        VolumeRelation::Different | VolumeRelation::Unknown => {
            if input.can_copy {
                TransferKind::Copy
            } else {
                TransferKind::Move
            }
        }
    }
}

pub(crate) fn drop_is_noop(dest: &Location, sources: &[Location]) -> bool {
    let destination = gio_file(dest);
    sources.iter().any(|source| {
        let source = gio_file(source);
        let Some(name) = source.basename() else {
            return false;
        };
        let target = destination.child(name);
        source.equal(&target) || source.equal(&destination) || destination.has_prefix(&source)
    })
}

fn gio_file(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}
