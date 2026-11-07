// SPDX-License-Identifier: MIT

use crate::{
    model::Location,
    services::{MoveRecord, RenameRecord},
};

use super::{PendingUndo, UndoEntry, UndoState};

/// What the latest undo entries become while a new folder gathers moved items.
enum Fold {
    Fuse {
        folder: Location,
        records: Vec<MoveRecord>,
        done: bool,
    },
    Clear,
}

impl UndoState {
    /// Fuses "create folder, then move items into it" into one Group entry, and
    /// follows a rename of that folder.
    pub(in crate::app::browser) fn fold_group_tail(&mut self) {
        let Some(created) = self.group_folder.clone() else {
            return;
        };
        match self.group_fold(created) {
            Fold::Fuse {
                folder,
                records,
                done,
            } => {
                let len = self.history.len();
                self.history.truncate(len - 2);
                self.push_entry(false, UndoEntry::Group { folder, records });
                if done {
                    self.group_folder = None;
                }
            }
            Fold::Clear => self.group_folder = None,
        }
    }

    fn unclaimed_tail(&self, offset: usize) -> Option<&PendingUndo> {
        self.history
            .iter()
            .rev()
            .nth(offset)
            .filter(|top| !top.claimed)
    }

    fn group_fold(&self, created: Location) -> Fold {
        match self.unclaimed_tail(0).map(|top| top.entry.clone()) {
            Some(UndoEntry::Move(records)) if self.moves_into_created(&records, &created) => {
                Fold::Fuse {
                    folder: created,
                    records,
                    done: self.group_rename_finished,
                }
            }
            Some(UndoEntry::Rename(record)) => self.renamed_group_fold(&record),
            _ => Fold::Clear,
        }
    }

    /// A move of items into the folder that the copy below it just created.
    fn moves_into_created(&self, records: &[MoveRecord], created: &Location) -> bool {
        !records.is_empty()
            && records
                .iter()
                .all(|record| record.current.parent().as_ref() == Some(created))
            && matches!(
                self.unclaimed_tail(1).map(|top| &top.entry),
                Some(UndoEntry::Copy(locations))
                    if locations.as_slice() == std::slice::from_ref(created)
            )
    }

    fn renamed_group_fold(&self, record: &RenameRecord) -> Fold {
        match self.unclaimed_tail(1).map(|top| &top.entry) {
            Some(UndoEntry::Group { folder, records }) if *folder == record.original => {
                let folder = record.current.clone();
                Fold::Fuse {
                    records: regrouped_records(records, &folder),
                    folder,
                    done: true,
                }
            }
            _ => Fold::Clear,
        }
    }
}

fn regrouped_records(records: &[MoveRecord], folder: &Location) -> Vec<MoveRecord> {
    records
        .iter()
        .map(|moved| MoveRecord {
            original: moved.original.clone(),
            current: moved
                .original
                .file_name()
                .and_then(|name| folder.child(&name))
                .unwrap_or_else(|| moved.current.clone()),
            renamed: Vec::new(),
            replaced: moved.replaced,
        })
        .collect()
}
