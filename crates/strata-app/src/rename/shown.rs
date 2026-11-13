// SPDX-License-Identifier: MIT

//! Showing a renamed row where its new name sorts, once the rename lands
//! (`reveal_completed_rename` and its tick in `src/ui/browser/inline_edit/reveal.rs`).
//!
//! The browser says the rename completed before the listing has the entry under its new
//! name, so the reveal waits for the shown view to list it, for 5 s at most. It gives up
//! when the selection is no longer the renamed item alone (under its old name or its new
//! one) or the keyboard has left its column: something else moved the cursor.
//!
//! Left out: a person's scroll or press during the wait does not cancel it. GTK hears both
//! in the window and bumps its reveal generation (`yield_rename_reveal`, from
//! `install_inline_edit_dismissal` in `src/ui/browser/inline_edit.rs`), so the person's
//! scroll wins; here the reveal scrolls the row back once the listing has it. Parts have no
//! pointer or scroll hook (`A7.md`, "Open").

use std::time::{Duration, Instant};

use rune::Tx;
use strata_core::model::Location;

use super::host;
use crate::{App, shell};

/// How long the reveal waits for the listing (`RenameRevealContext::deadline`).
const PATIENCE: Duration = Duration::from_secs(5);

/// How soon a poll looks again while the reveal waits.
const AGAIN: Duration = Duration::from_millis(16);

/// A completed rename whose row has yet to be shown.
pub(super) struct Pending {
    depth: usize,
    old: Location,
    target: Location,
    until: Instant,
}

impl Pending {
    pub(super) fn new(depth: usize, old: Location, target: Location) -> Self {
        Self {
            depth,
            old,
            target,
            until: Instant::now() + PATIENCE,
        }
    }
}

/// `rename_reveal_is_current` and `rename_selection_is_target`.
fn still_wanted(app: &App, pending: &Pending) -> bool {
    let selected = app.browser.selected_entries();
    let target = matches!(
        selected.as_slice(),
        [only] if only.location == pending.old || only.location == pending.target
    );
    target && Instant::now() < pending.until && app.browser.active_depth() == Some(pending.depth)
}

/// One poll: the row is shown once the view lists it. True when the view has to run.
pub(super) fn tick(app: &mut App, tx: &mut Tx) -> bool {
    let Some(pending) = app.rename.shown.take() else {
        return false;
    };
    let Some(view) = host(app).filter(|_| still_wanted(app, &pending)) else {
        return false;
    };
    let id = shell::id_of(app, &pending.target);
    if (view.rows.position)(app, (pending.depth, id)).is_none() {
        tx.wake_after(AGAIN);
        app.rename.shown = Some(pending);
        return false;
    }
    (view.rows.reveal)(app, (pending.depth, id), tx);
    true
}
