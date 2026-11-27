// SPDX-License-Identifier: MIT

use strata_core::model::Location;
use strata_fs::{
    adapters::{TransferTargetCollision, probes::Presence},
    app::Browser,
    services::{MoveRecord, TransferConflict, UndoMoveItem},
};

use crate::conflict_dialog::{
    ConflictChoice, ConflictPrompt, compact_display_path, covered_by_answer,
    unreplaceable_explanation,
};

/// Which way a replay runs, and the history entry it is (`ReplayKey`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Replay {
    pub(super) redo: bool,
    pub(super) generation: u64,
}

impl Replay {
    pub(super) fn undo(generation: u64) -> Self {
        Self {
            redo: false,
            generation,
        }
    }

    pub(super) fn redo(generation: u64) -> Self {
        Self {
            redo: true,
            generation,
        }
    }

    /// Drops the entry: nothing of it is left to replay.
    pub(super) fn discard(self, browser: &Browser) {
        browser.discard_pending_replay(self.redo, self.generation);
    }

    /// Where a record's item is now, and where this replay moves it.
    pub(super) fn ends(self, record: &MoveRecord) -> (Location, Location) {
        if self.redo {
            (record.original.clone(), record.current.clone())
        } else {
            (record.current.clone(), record.original.clone())
        }
    }
}

/// The locations the check found something at.
pub(super) fn found(locations: Vec<Location>, presence: Vec<Presence>) -> Vec<Location> {
    locations
        .into_iter()
        .zip(presence)
        .filter(|(_, presence)| !matches!(presence, Presence::Absent))
        .map(|(location, _)| location)
        .collect()
}

#[derive(Clone, Debug)]
pub(super) struct ReplayCollision {
    pub(super) record: MoveRecord,
    pub(super) kinds: TransferTargetCollision,
}

impl ReplayCollision {
    fn between(record: MoveRecord, item: &Presence, existing_folder: bool) -> Self {
        let item_folder = item.folder().unwrap_or(existing_folder);
        Self {
            record,
            kinds: TransferTargetCollision::between(item_folder, existing_folder),
        }
    }

    fn replacing(self) -> UndoMoveItem {
        UndoMoveItem {
            record: self.record,
            conflict: TransferConflict::ReplaceExisting,
        }
    }
}

/// A move's records by what the check found at their two ends (`sort_replay_records`).
#[derive(Debug, Default)]
pub(super) struct Sorted {
    /// The item is there and nothing is where it goes.
    pub(super) accepted: Vec<UndoMoveItem>,
    /// Something is where the item goes: the prompt's to decide.
    pub(super) collisions: Vec<ReplayCollision>,
    /// The item is gone.
    pub(super) missing: usize,
}

impl Sorted {
    /// `presence` holds two answers for each record, its item's and its destination's.
    pub(super) fn of(records: Vec<MoveRecord>, presence: &[Presence]) -> Self {
        let mut sorted = Self::default();
        for (record, ends) in records.into_iter().zip(presence.as_chunks::<2>().0) {
            match ends {
                [Presence::Absent, _] => sorted.missing += 1,
                [item, Presence::Present { folder }] => sorted
                    .collisions
                    .push(ReplayCollision::between(record, item, *folder)),
                _ => sorted.accepted.push(UndoMoveItem {
                    record,
                    conflict: TransferConflict::FailIfExists,
                }),
            }
        }
        sorted
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct GroupReplay {
    pub(super) had_missing: bool,
    pub(super) skipped: bool,
}

#[derive(Clone, Debug)]
pub(super) struct PendingReplay {
    pub(super) replay: Replay,
    pub(super) collisions: Vec<ReplayCollision>,
    pub(super) accepted: Vec<UndoMoveItem>,
    pub(super) group: Option<GroupReplay>,
}

impl PendingReplay {
    pub(super) fn apply_choice(
        mut self,
        current: &ReplayCollision,
        choice: ConflictChoice,
        apply_to_all: bool,
    ) -> Self {
        match choice {
            ConflictChoice::Replace => {
                self.accepted.push(current.clone().replacing());
                if apply_to_all {
                    let (covered, rest) = covered_by_answer(
                        self.collisions,
                        TransferConflict::ReplaceExisting,
                        |collision| collision.kinds,
                    );
                    self.accepted
                        .extend(covered.into_iter().map(ReplayCollision::replacing));
                    self.collisions = rest;
                }
            }
            ConflictChoice::Merge | ConflictChoice::KeepBoth => {
                unreachable!("merge and keep-both are not offered for replay conflicts")
            }
            ConflictChoice::Skip => {
                if apply_to_all {
                    self.collisions.clear();
                }
                if let Some(group) = self.group.as_mut() {
                    group.skipped = true;
                }
            }
        }
        self
    }
}

pub(super) fn replay_destination(record: &MoveRecord, redo: bool) -> &Location {
    if redo {
        &record.current
    } else {
        &record.original
    }
}

pub(super) fn replay_conflict_explanation(collision: &ReplayCollision, redo: bool) -> String {
    let destination = replay_destination(&collision.record, redo);
    let name = destination.display_name();
    let parent = destination.parent().unwrap_or_else(|| destination.clone());
    let folder = compact_display_path(&parent);
    if !collision.kinds.replaceable {
        unreplaceable_explanation(&name, &folder)
    } else if redo {
        rust_i18n::t!(
            "An item named “%{name}” already exists in %{folder}. Redoing the move will overwrite its contents.",
            name = name,
            folder = folder
        )
        .into_owned()
    } else {
        rust_i18n::t!(
            "An item named “%{name}” already exists in %{folder}. Undoing the move will overwrite its contents.",
            name = name,
            folder = folder
        )
        .into_owned()
    }
}

pub(super) fn replay_prompt_visibility(
    remaining: &[ReplayCollision],
    accepted: usize,
) -> (bool, bool) {
    let applicable = accepted > 0 || remaining.iter().any(|later| later.kinds.replaceable);
    let (apply_to_all, skip) = ConflictPrompt::visibility(remaining.len(), accepted);
    (apply_to_all && applicable, skip && applicable)
}
