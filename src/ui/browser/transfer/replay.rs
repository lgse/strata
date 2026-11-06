// SPDX-License-Identifier: MIT

use super::collisions::{covered_by_answer, unreplaceable_explanation};
use super::conflict_dialog::{ConflictActions, ConflictChoice, ConflictPrompt};
use super::{Preparing, folder_row, found, preparing_row};
use crate::adapters::TransferTargetCollision;
use crate::adapters::probes::{self, Presence};
use crate::app::Browser;
use crate::model::Location;
use crate::services::{MoveRecord, TransferConflict, UndoMoveItem};
use crate::ui::browser::ViewState;
use crate::ui::browser::paths::compact_display_path;
use gtk::{gio, glib};
use std::rc::Rc;

#[derive(Clone, Copy)]
struct GroupReplay {
    had_missing: bool,
    skipped: bool,
}

/// Which way a replay runs and the history entry it belongs to.
#[derive(Clone, Copy)]
struct ReplayKey {
    redo: bool,
    generation: u64,
}

impl ReplayKey {
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

    fn discard(self, browser: &Browser) {
        browser.discard_pending_replay(self.redo, self.generation);
    }

    /// The location a record moves from and to in this direction.
    fn ends(self, record: &MoveRecord) -> (Location, Location) {
        if self.redo {
            (record.original.clone(), record.current.clone())
        } else {
            (record.current.clone(), record.original.clone())
        }
    }
}

#[derive(Clone)]
struct ReplayCollision {
    record: MoveRecord,
    kinds: TransferTargetCollision,
}

impl ReplayCollision {
    /// An item whose kind is unknown is taken to match, so only a proven file meeting a
    /// folder, or a folder meeting a file, loses Replace.
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

/// Records sorted by what the probe found at their two ends.
struct ReplaySort {
    accepted: Vec<UndoMoveItem>,
    collisions: Vec<ReplayCollision>,
    missing: usize,
}

fn sort_replay_records(records: Vec<MoveRecord>, presence: &[Presence]) -> ReplaySort {
    let mut sorted = ReplaySort {
        accepted: Vec::new(),
        collisions: Vec::new(),
        missing: 0,
    };
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

fn replay_conflict_explanation(collision: &ReplayCollision, redo: bool) -> String {
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

/// Skip, and Apply to All beside it, only while an answer can still move something: an item
/// already accepted, or a later conflict that Replace can resolve. Otherwise only Cancel is left.
fn replay_prompt_visibility(remaining: &[ReplayCollision], accepted: usize) -> (bool, bool) {
    let applicable = accepted > 0 || remaining.iter().any(|later| later.kinds.replaceable);
    let (apply_to_all, skip) = ConflictPrompt::visibility(remaining.len(), accepted);
    (apply_to_all && applicable, skip && applicable)
}

fn replay_destination(record: &MoveRecord, redo: bool) -> &Location {
    if redo {
        &record.current
    } else {
        &record.original
    }
}

fn apply_replay_choice(
    current: &ReplayCollision,
    (choice, apply_to_all): (ConflictChoice, bool),
    accepted: &mut Vec<UndoMoveItem>,
    remaining: &mut Vec<ReplayCollision>,
    group: &mut Option<GroupReplay>,
) {
    match choice {
        ConflictChoice::Replace => {
            accepted.push(current.clone().replacing());
            if apply_to_all {
                let (covered, rest) = covered_by_answer(
                    std::mem::take(remaining),
                    TransferConflict::ReplaceExisting,
                    |collision| collision.kinds,
                );
                accepted.extend(covered.into_iter().map(ReplayCollision::replacing));
                *remaining = rest;
            }
        }
        ConflictChoice::Merge | ConflictChoice::KeepBoth => {
            unreachable!("merge and keep-both are not offered for replay conflicts")
        }
        ConflictChoice::Skip => {
            if apply_to_all {
                remaining.clear();
            }
            if let Some(group) = group.as_mut() {
                group.skipped = true;
            }
        }
    }
}

impl ViewState {
    /// Moves the latest completed transfer back, confirming any item that would
    /// overwrite something created since the move.
    pub(in crate::ui::browser) fn undo_move(
        self: &Rc<Self>,
        generation: u64,
        records: Vec<MoveRecord>,
    ) -> bool {
        self.replay_move(ReplayKey::undo(generation), records, false)
    }

    pub(in crate::ui::browser) fn undo_group(
        self: &Rc<Self>,
        generation: u64,
        records: Vec<MoveRecord>,
    ) -> bool {
        self.replay_move(ReplayKey::undo(generation), records, true)
    }

    pub(in crate::ui::browser) fn redo_trash(
        self: &Rc<Self>,
        generation: u64,
        locations: Vec<Location>,
    ) -> bool {
        self.replay_existing_locations(ReplayKey::redo(generation), locations, Browser::redo_trash)
    }

    pub(in crate::ui::browser) fn redo_move(
        self: &Rc<Self>,
        generation: u64,
        records: Vec<MoveRecord>,
    ) -> bool {
        self.replay_move(ReplayKey::redo(generation), records, false)
    }

    pub(in crate::ui::browser) fn undo_copy(
        self: &Rc<Self>,
        generation: u64,
        locations: Vec<Location>,
    ) -> bool {
        self.replay_existing_locations(ReplayKey::undo(generation), locations, Browser::undo_copy)
    }

    pub(in crate::ui::browser) fn undo_merge(
        self: &Rc<Self>,
        generation: u64,
        created: Vec<Location>,
        overwritten: Vec<Location>,
    ) -> bool {
        let Some(first) = created.first() else {
            if overwritten.is_empty() {
                self.browser.discard_pending_replay(false, generation);
                return false;
            }
            return self.browser.undo_merge(generation, created, overwritten);
        };
        let row = folder_row(first, first);
        self.check_replay(
            ReplayKey::undo(generation),
            row,
            created.clone(),
            move |state, presence| {
                let created = found(created, presence);
                // Overwritten entries stay even when the incoming copy is gone: the
                // staged original may still be sitting in Trash waiting to restore.
                if created.is_empty() && overwritten.is_empty() {
                    state.browser.discard_pending_replay(false, generation);
                } else {
                    state.browser.undo_merge(generation, created, overwritten);
                }
            },
        )
    }

    fn check_replay(
        self: &Rc<Self>,
        key: ReplayKey,
        row: (String, (String, String)),
        locations: Vec<Location>,
        resume: impl FnOnce(&Rc<Self>, Vec<Presence>) + 'static,
    ) -> bool {
        let replay = Preparing::Replay {
            redo: key.redo,
            generation: key.generation,
        };
        if self.transfer_in_progress(&replay) {
            return true;
        }
        let cancellable = gio::Cancellable::new();
        let preparation = self.show_transfer_preparation(true, row, &cancellable);
        self.preparing_transfers.borrow_mut().push((
            replay,
            cancellable.clone(),
            Rc::downgrade(&preparation),
        ));
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let presence = probes::presence(&locations, &cancellable).await;
            let Some(state) = weak.upgrade() else {
                preparation.dismiss_file_operation_progress();
                return;
            };
            if !state.finish_preparing(&cancellable, &preparation) {
                return;
            }
            state.transfer_preparation.replace(Some(preparation));
            resume(&state, presence);
            state.dismiss_transfer_preparation();
        });
        true
    }

    fn replay_existing_locations(
        self: &Rc<Self>,
        key: ReplayKey,
        locations: Vec<Location>,
        dispatch: fn(&Rc<Browser>, u64, Vec<Location>) -> bool,
    ) -> bool {
        let Some(first) = locations.first() else {
            key.discard(&self.browser);
            return false;
        };
        let row = preparing_row(first, crate::app::trash_destination_texts());
        self.check_replay(key, row, locations.clone(), move |state, presence| {
            let existing = found(locations, presence);
            if existing.is_empty() {
                key.discard(&state.browser);
            } else {
                dispatch(&state.browser, key.generation, existing);
            }
        })
    }

    fn replay_move(
        self: &Rc<Self>,
        key: ReplayKey,
        records: Vec<MoveRecord>,
        grouped: bool,
    ) -> bool {
        let Some((item, destination)) = records.first().map(|record| key.ends(record)) else {
            key.discard(&self.browser);
            return false;
        };
        let locations = records
            .iter()
            .flat_map(|record| <[Location; 2]>::from(key.ends(record)))
            .collect();
        self.check_replay(
            key,
            folder_row(&item, &destination),
            locations,
            move |state, presence| {
                state.resume_replay_move(key, sort_replay_records(records, &presence), grouped);
            },
        )
    }

    fn resume_replay_move(self: &Rc<Self>, key: ReplayKey, sorted: ReplaySort, grouped: bool) {
        let ReplaySort {
            accepted,
            collisions,
            missing,
        } = sorted;
        if accepted.is_empty() && collisions.is_empty() {
            self.discard_empty_replay(key, grouped && missing > 0);
            return;
        }
        if !collisions.is_empty() {
            self.dismiss_transfer_preparation();
        }
        self.resolve_replay_collisions(
            key,
            collisions,
            accepted,
            grouped.then_some(GroupReplay {
                had_missing: missing > 0,
                skipped: false,
            }),
        );
    }

    fn discard_empty_replay(&self, key: ReplayKey, group_had_missing: bool) {
        if group_had_missing && !key.redo {
            self.browser
                .discard_group_keep_folder(key.redo, key.generation);
        } else {
            key.discard(&self.browser);
        }
    }

    fn dispatch_replay(
        &self,
        key: ReplayKey,
        accepted: Vec<UndoMoveItem>,
        group: Option<GroupReplay>,
    ) {
        let generation = key.generation;
        // Every conflict was skipped, so nothing moves: the entry stays, as after Cancel.
        if accepted.is_empty() {
            return;
        }
        if let Some(state) = group {
            if state.had_missing && !state.skipped {
                self.browser
                    .undo_group_retaining_folder(generation, accepted);
            } else {
                self.browser.undo_group(generation, accepted);
            }
        } else if key.redo {
            self.browser.redo_move(generation, accepted);
        } else {
            self.browser.undo_move(generation, accepted);
        }
    }

    fn resolve_replay_collisions(
        self: &Rc<Self>,
        key: ReplayKey,
        mut collisions: Vec<ReplayCollision>,
        accepted: Vec<UndoMoveItem>,
        group: Option<GroupReplay>,
    ) {
        if collisions.is_empty() {
            self.dispatch_replay(key, accepted, group);
            return;
        }
        let collision = collisions.remove(0);
        let name = replay_destination(&collision.record, key.redo).display_name();
        let explanation = replay_conflict_explanation(&collision, key.redo);
        let (apply_to_all_visible, skip_visible) =
            replay_prompt_visibility(&collisions, accepted.len());
        let state = self.clone();
        self.confirm_replace_conflict(
            ConflictPrompt {
                name: &name,
                explanation: &explanation,
                apply_to_all_visible,
                skip_visible,
                actions: ConflictActions {
                    replace: collision.kinds.replaceable,
                    ..ConflictActions::default()
                },
            },
            Rc::new(move |choice, apply_to_all| {
                let mut accepted = accepted.clone();
                let mut remaining = collisions.clone();
                let mut group = group;
                apply_replay_choice(
                    &collision,
                    (choice, apply_to_all),
                    &mut accepted,
                    &mut remaining,
                    &mut group,
                );
                state.resolve_replay_collisions(key, remaining, accepted, group);
            }),
        );
    }
}
