// SPDX-License-Identifier: MIT

//! A replay of the history and what the check found at its locations, as the GTK view sorts
//! them (`src/ui/browser/transfer/replay.rs`, `found` in `src/ui/browser/transfer.rs`).

use strata_core::model::Location;
use strata_fs::{
    adapters::probes::Presence,
    app::Browser,
    services::{MoveRecord, TransferConflict, UndoMoveItem},
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

/// A move's records by what the check found at their two ends (`sort_replay_records`).
#[derive(Debug, Default)]
pub(super) struct Sorted {
    /// The item is there and nothing is where it goes.
    pub(super) accepted: Vec<UndoMoveItem>,
    /// Something is where the item goes: the prompt's to decide.
    pub(super) collided: usize,
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
                [_, Presence::Present { .. }] => sorted.collided += 1,
                _ => sorted.accepted.push(UndoMoveItem {
                    record,
                    conflict: TransferConflict::FailIfExists,
                }),
            }
        }
        sorted
    }
}
