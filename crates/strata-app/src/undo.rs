// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::Tx;
use strata_core::model::Location;
use strata_desktop::keys::{
    Modifiers,
    bindings::{REDO, UNDO},
};
use strata_fs::{
    adapters::probes::{self, Presence},
    app::Browser,
    services::{MoveRecord, UndoMoveItem},
};

use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Stage},
    conflict_dialog::{self, ConflictActions, ConflictPrompt},
    slots::{Frame, Later, Part, Place, Setup},
};

mod replay;

use replay::{
    GroupReplay, PendingReplay, Replay, Sorted, found, replay_conflict_explanation,
    replay_destination, replay_prompt_visibility,
};

pub(crate) struct Undo {
    later: Later<App>,
    /// The replays whose check has not answered (`transfer_in_progress`).
    checking: Vec<Replay>,
}

/// Nothing of undo is drawn here: its conflict prompt lives in [`crate::conflict_dialog`].
fn nothing(_: &App, _: &mut Frame<'_, App>) -> Option<rune::El> {
    None
}

impl Part for Undo {
    const PLACE: Place<App> = Place::Overlays(nothing);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            checking: Vec::new(),
        }
    }

    const COMMANDS: &'static [Action<App>] = &[
        Action::new("undo", undone).command(UNDO).keys(&[Keys::at(
            Stage::TextInput,
            &[UNDO.ignoring(Modifiers::empty())],
        )
        .when(|_, at| !at.text())]),
        Action::new("redo", redone).command(REDO).keys(&[Keys::at(
            Stage::TextInput,
            &[REDO.ignoring(Modifiers::empty())],
        )
        .when(|_, at| !at.text())]),
    ];
}

/// Taken when something was replayed or is being checked; else the next stage is asked.
fn taken(replayed: bool) -> Outcome {
    if replayed {
        Outcome::Ran
    } else {
        Outcome::Declined
    }
}

fn undone(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    taken(undo_last(app))
}

fn redone(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    taken(redo_last(app))
}

/// `undo_last_operation`: the newest entry that is not the Trash's first, then the Trash's.
fn undo_last(app: &mut App) -> bool {
    let browser = app.browser.clone();
    if let Some((generation, _, _)) = browser.pending_undo_rename() {
        return browser.undo_rename(generation);
    }
    if let Some((generation, records)) = browser.pending_undo_move() {
        return replay_move(app, Replay::undo(generation), records, false);
    }
    if let Some((generation, records)) = browser.pending_undo_group() {
        return replay_move(app, Replay::undo(generation), records, true);
    }
    if let Some((generation, locations)) = browser.pending_undo_copy() {
        return replay_existing(app, Replay::undo(generation), locations, Browser::undo_copy);
    }
    if let Some((generation, created, overwritten)) = browser.pending_undo_merge() {
        return undo_merge(app, generation, created, overwritten);
    }
    browser.undo_last_trash()
}

/// `redo_last_operation`.
fn redo_last(app: &mut App) -> bool {
    let browser = app.browser.clone();
    if let Some((generation, _, _)) = browser.pending_redo_rename() {
        return browser.redo_rename(generation);
    }
    if let Some((generation, records)) = browser.pending_redo_move() {
        return replay_move(app, Replay::redo(generation), records, false);
    }
    if let Some((generation, locations)) = browser.pending_redo_copy() {
        return browser.redo_copy(generation, locations);
    }
    if let Some((generation, locations)) = browser.pending_redo_trash() {
        return replay_existing(
            app,
            Replay::redo(generation),
            locations,
            Browser::redo_trash,
        );
    }
    false
}

fn check(
    app: &mut App,
    replay: Replay,
    locations: Vec<Location>,
    resume: impl FnOnce(&mut App, Vec<Presence>) + 'static,
) -> bool {
    let undo = &mut app.undo;
    if undo.checking.contains(&replay) {
        return true;
    }
    undo.checking.push(replay);
    let later = undo.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let presence = probes::presence(&locations, &gio::Cancellable::new()).await;
        later.then(move |app: &mut App, _| {
            app.undo.checking.retain(|checking| *checking != replay);
            resume(app, presence);
            true
        });
    });
    true
}

/// `replay_existing_locations`: a copy undone or the Trash redone, for what is still there.
fn replay_existing(
    app: &mut App,
    replay: Replay,
    locations: Vec<Location>,
    dispatch: fn(&Rc<Browser>, u64, Vec<Location>) -> bool,
) -> bool {
    if locations.is_empty() {
        replay.discard(&app.browser);
        return false;
    }
    check(app, replay, locations.clone(), move |app, presence| {
        let existing = found(locations, presence);
        if existing.is_empty() {
            replay.discard(&app.browser);
        } else {
            dispatch(&app.browser, replay.generation, existing);
        }
    })
}

fn undo_merge(
    app: &mut App,
    generation: u64,
    created: Vec<Location>,
    overwritten: Vec<Location>,
) -> bool {
    let replay = Replay::undo(generation);
    if created.is_empty() {
        if overwritten.is_empty() {
            replay.discard(&app.browser);
            return false;
        }
        return app.browser.undo_merge(generation, created, overwritten);
    }
    check(app, replay, created.clone(), move |app, presence| {
        let created = found(created, presence);
        if created.is_empty() && overwritten.is_empty() {
            replay.discard(&app.browser);
        } else {
            app.browser.undo_merge(generation, created, overwritten);
        }
    })
}

/// `replay_move`: each record's two ends are checked, and what can move moves.
fn replay_move(app: &mut App, replay: Replay, records: Vec<MoveRecord>, grouped: bool) -> bool {
    if records.is_empty() {
        replay.discard(&app.browser);
        return false;
    }
    let locations = records
        .iter()
        .flat_map(|record| <[Location; 2]>::from(replay.ends(record)))
        .collect();
    check(app, replay, locations, move |app, presence| {
        let sorted = Sorted::of(records, &presence);
        resume_move(app, replay, sorted, grouped);
    })
}

fn resume_move(app: &mut App, replay: Replay, sorted: Sorted, grouped: bool) {
    let Sorted {
        accepted,
        collisions,
        missing,
    } = sorted;
    if accepted.is_empty() && collisions.is_empty() {
        if grouped && missing > 0 && !replay.redo {
            app.browser
                .discard_group_keep_folder(replay.redo, replay.generation);
        } else {
            replay.discard(&app.browser);
        }
        return;
    }
    resolve_replay_collisions(
        app,
        PendingReplay {
            replay,
            collisions,
            accepted,
            group: grouped.then_some(GroupReplay {
                had_missing: missing > 0,
                skipped: false,
            }),
        },
    );
}

fn dispatch_replay(
    browser: &Rc<Browser>,
    replay: Replay,
    accepted: Vec<UndoMoveItem>,
    group: Option<GroupReplay>,
) {
    // Every conflict was skipped, so nothing moves: the entry stays, as after Cancel.
    if accepted.is_empty() {
        return;
    }
    let generation = replay.generation;
    if let Some(state) = group {
        if state.had_missing && !state.skipped {
            browser.undo_group_retaining_folder(generation, accepted);
        } else {
            browser.undo_group(generation, accepted);
        }
    } else if replay.redo {
        browser.redo_move(generation, accepted);
    } else {
        browser.undo_move(generation, accepted);
    }
}

fn resolve_replay_collisions(app: &mut App, mut pending: PendingReplay) {
    if pending.collisions.is_empty() {
        dispatch_replay(
            &app.browser,
            pending.replay,
            pending.accepted,
            pending.group,
        );
        return;
    }
    let collision = pending.collisions.remove(0);
    let redo = pending.replay.redo;
    let name = replay_destination(&collision.record, redo).display_name();
    let explanation = replay_conflict_explanation(&collision, redo);
    let (apply_to_all_visible, skip_visible) =
        replay_prompt_visibility(&pending.collisions, pending.accepted.len());
    conflict_dialog::prompt(
        app,
        ConflictPrompt {
            name,
            explanation,
            apply_to_all_visible,
            skip_visible,
            actions: ConflictActions {
                replace: collision.kinds.replaceable,
                ..ConflictActions::default()
            },
        },
        move |app, choice, apply_to_all| {
            let next = pending
                .clone()
                .apply_choice(&collision, choice, apply_to_all);
            resolve_replay_collisions(app, next);
        },
    );
}

#[cfg(test)]
mod tests;
