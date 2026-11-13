// SPDX-License-Identifier: MIT

//! Undo and redo: Ctrl+Z, and Ctrl+Shift+Z or Ctrl+Y, outside a text field. `Browser` keeps
//! the history and replays it; this part reads the keys where the GTK window does and asks
//! for the replay as the GTK view does (`undo_last_operation` and `redo_last_operation` in
//! `src/ui/browser/view_edit.rs`).
//!
//! - **The keys.** The dispatcher's `text_input` stage reads both outside a text field, after
//!   the inline edit, the filter and the location bar (`src/ui/window/keyboard/commands.rs`).
//!   A key with nothing to replay is not taken. In a field Ctrl+Z is the field's own.
//! - **What is replayed, in GTK's order.** Undo: a rename at once; a move, a typed path's
//!   move into a folder it made (a group), a copy (what was created, too) and a merge after a
//!   check of what is on disk (`replay`); then the Trash, at once. Redo: a rename and a copy at
//!   once, a move and the Trash after the check.
//! - **The check** reads the replay's locations off the interface thread
//!   (`strata_fs::adapters::probes::presence`) and answers through `Setup::later`. A second
//!   press for the same replay while it runs is taken and starts nothing.
//!
//! Left out, with what brings each back:
//!
//! - **The prompt for an item in the way** (`resolve_replay_collisions` in
//!   `src/ui/browser/transfer/replay.rs`): the conflict prompt is the clipboard and conflicts
//!   package's, A12 (`docs/rune/parity/packages/A12.md`). Until it lands, a replay with an
//!   item in the way stops as the prompt's Cancel does, and its entry stays.
//! - **The card that shows the check running** (`show_transfer_preparation`): the progress
//!   card is not on this interface yet.
//! - **The Trash undo's fly-out** (`prepare_fly_from_trash`): an animation of the trash
//!   button, which this interface has not.

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
    services::MoveRecord,
};

use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Stage},
    slots::{Frame, Later, Part, Place, Setup},
};

mod replay;

use replay::{Replay, Sorted, found};

pub(crate) struct Undo {
    later: Later<App>,
    /// The replays whose check has not answered (`transfer_in_progress`).
    checking: Vec<Replay>,
}

/// Nothing of undo is drawn: the check's card and the conflict prompt are left out.
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

    // `text_input` in `src/ui/window/keyboard/commands.rs`: `press.is(UNDO)`, the active
    // map's bindings as they are, and `!event.text_has_focus()`.
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

/// Reads what is at `locations` and hands it to `resume` in a later update. True, so the key
/// is taken, also when this replay is being checked already (`check_replay`).
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

/// The view's `undo_merge`: what the merge created is checked; what it overwrote is in the
/// Trash, and is restored whether or not the copy is still there.
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
        resume_move(&app.browser, replay, sorted, grouped);
    })
}

/// `resume_replay_move` and `dispatch_replay`, with no prompt for an item in the way.
fn resume_move(browser: &Rc<Browser>, replay: Replay, sorted: Sorted, grouped: bool) {
    let Sorted {
        accepted,
        collided,
        missing,
    } = sorted;
    if accepted.is_empty() && collided == 0 {
        if grouped && missing > 0 && !replay.redo {
            browser.discard_group_keep_folder(replay.redo, replay.generation);
        } else {
            replay.discard(browser);
        }
        return;
    }
    // The prompt's Cancel: nothing moves, and the entry stays.
    if collided > 0 {
        tracing::debug!(
            collided,
            "an item is in the way of the replay, and no prompt asks"
        );
        return;
    }
    let generation = replay.generation;
    if grouped {
        if missing > 0 {
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

#[cfg(test)]
mod tests;
