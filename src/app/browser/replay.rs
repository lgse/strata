// SPDX-License-Identifier: MIT

use std::cell::RefCell;

use crate::{
    model::Location,
    services::{MoveRecord, RenameRecord},
};

use super::Browser;
use super::undo::{UndoEntry, claim_replay, finish_replay, peek_replay};

mod group;
mod locations;
mod rename;

/// One claimed undo or redo entry: which stack it came from and its generation.
#[derive(Clone, Copy)]
struct Replay {
    redo: bool,
    generation: u64,
}

impl Replay {
    fn undo(generation: u64) -> Self {
        Self {
            redo: false,
            generation,
        }
    }

    fn redo(generation: u64) -> Self {
        Self {
            redo: true,
            generation,
        }
    }

    fn finish(self, completed: bool) {
        finish_replay(self.redo, self.generation, completed);
    }
}

impl Browser {
    fn pending_replay_entry(&self, redo: bool) -> Option<(u64, UndoEntry)> {
        if self.current_operation.get().is_some() {
            return None;
        }
        peek_replay(redo)
    }

    fn pending_as<T>(
        &self,
        redo: bool,
        pick: impl FnOnce(UndoEntry) -> Option<T>,
    ) -> Option<(u64, T)> {
        let (generation, entry) = self.pending_replay_entry(redo)?;
        Some((generation, pick(entry)?))
    }

    pub fn pending_undo_move(&self) -> Option<(u64, Vec<MoveRecord>)> {
        self.pending_as(false, move_records_of)
    }

    pub fn pending_undo_copy(&self) -> Option<(u64, Vec<Location>)> {
        self.pending_as(false, copied_locations_of)
    }

    pub fn pending_undo_merge(&self) -> Option<(u64, Vec<Location>, Vec<Location>)> {
        let (generation, (created, overwritten)) = self.pending_as(false, merged_locations_of)?;
        Some((generation, created, overwritten))
    }

    pub fn pending_undo_rename(&self) -> Option<(u64, Location, Location)> {
        let (generation, record) = self.pending_as(false, rename_record_of)?;
        Some((generation, record.current, record.original))
    }

    pub fn pending_undo_trash(&self) -> Option<Vec<Location>> {
        self.pending_as(false, trashed_locations_of)
            .map(|(_, locations)| locations)
    }

    pub fn pending_undo_group(&self) -> Option<(u64, Vec<MoveRecord>)> {
        self.pending_as(false, group_records_of)
    }

    pub fn discard_pending_replay(&self, redo: bool, generation: u64) {
        if claim_replay(redo, Some(generation)).is_some() {
            finish_replay(redo, generation, true);
        }
    }

    pub fn pending_redo_move(&self) -> Option<(u64, Vec<MoveRecord>)> {
        self.pending_as(true, move_records_of)
    }

    pub fn pending_redo_copy(&self) -> Option<(u64, Vec<Location>)> {
        self.pending_as(true, copied_locations_of)
    }

    pub fn pending_redo_rename(&self) -> Option<(u64, Location, Location)> {
        let (generation, record) = self.pending_as(true, rename_record_of)?;
        Some((generation, record.original, record.current))
    }

    pub fn pending_redo_trash(&self) -> Option<(u64, Vec<Location>)> {
        self.pending_as(true, trashed_locations_of)
    }

    pub(in crate::app::browser) fn replay_claim(
        &self,
        redo: bool,
    ) -> &RefCell<Option<(u64, UndoEntry)>> {
        if redo {
            &self.redo_claim
        } else {
            &self.undo_claim
        }
    }
}

fn move_records_of(entry: UndoEntry) -> Option<Vec<MoveRecord>> {
    match entry {
        UndoEntry::Move(records) => Some(records),
        _ => None,
    }
}

fn copied_locations_of(entry: UndoEntry) -> Option<Vec<Location>> {
    match entry {
        UndoEntry::Copy(locations) => Some(locations),
        _ => None,
    }
}

fn merged_locations_of(entry: UndoEntry) -> Option<(Vec<Location>, Vec<Location>)> {
    match entry {
        UndoEntry::Merge {
            created,
            overwritten,
            ..
        } => Some((created, overwritten)),
        _ => None,
    }
}

fn rename_record_of(entry: UndoEntry) -> Option<RenameRecord> {
    match entry {
        UndoEntry::Rename(record) => Some(record),
        _ => None,
    }
}

fn trashed_locations_of(entry: UndoEntry) -> Option<Vec<Location>> {
    match entry {
        UndoEntry::Trash { locations, .. } => Some(locations),
        _ => None,
    }
}

fn group_records_of(entry: UndoEntry) -> Option<Vec<MoveRecord>> {
    match entry {
        UndoEntry::Group { records, .. } => Some(records),
        _ => None,
    }
}
