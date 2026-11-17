// SPDX-License-Identifier: MIT

//! 10xer letter bookmarks (`m` save, `'` jump) backed by
//! `strata_fs::services::letter_bookmarks`, ported from `src/ui/window/keyboard/chords.rs`.

use std::{io, path::PathBuf};

use rune::Tx;
use strata_core::{i18n::tr, model::Location};
use strata_desktop::keys::KeyPress;
use strata_fs::{
    adapters::{mount_lanes, probes},
    app::{Browser, BrowserEvent},
    services::{io_error_detail, letter_bookmarks},
};

use super::{ChordKind, arm_chord, cancel_chord, focused_listing_depth};
use crate::{
    App,
    commands::{Call, Outcome, Situation},
    shell,
};

/// `cancels_pending_bookmark` (`src/ui/window/keyboard/chords.rs`).
pub(super) fn cancels_pending_bookmark(browser: &Browser, event: &BrowserEvent) -> bool {
    match event {
        BrowserEvent::NavigationStarting { .. } => true,
        BrowserEvent::FocusChanged { .. }
        | BrowserEvent::SelectionSetChanged { .. }
        | BrowserEvent::SelectionSynced { .. } => !browser.focus_follows_external_change(),
        _ => false,
    }
}

/// `Stage::Entry`: cancels a pending bookmark jump on any non-modifier key (`keyboard.rs:136`)
/// and advances `bookmark_revision` (`keyboard.rs:424`).
pub(super) fn on_entry_key(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    if call.press.is_some_and(|press| !press.is_modifier()) {
        crate::open_with::cancel_target_lookup(app);
        if app.tenxer.bookmark_pending {
            cancel_chord(app);
            return Outcome::Ran;
        }
        app.tenxer.bookmark_revision = app.tenxer.bookmark_revision.wrapping_add(1);
    }
    Outcome::Declined
}

fn bookmark_folder(app: &App) -> Option<PathBuf> {
    focused_listing_depth(app)
        .and_then(|depth| app.browser.location_at(depth))
        .and_then(|location| location.native_path().map(PathBuf::from))
}

pub(super) fn arm_go(app: &mut App) {
    arm_chord(app, ChordKind::BookmarkGo, Vec::new());
    let expected = app.tenxer.bookmark_revision;
    let generation = app.browser.navigation_generation();
    let later = app.tenxer.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let result = letter_bookmarks::list().await;
        later.then(move |app, tx| {
            let valid = app.tenxer.bookmark_revision == expected
                && app.tenxer.armed == Some(ChordKind::BookmarkGo)
                && shell::preferences(app).tenxer_mode
                && Situation::of(app, tx).in_view()
                && app.browser.navigation_generation() == generation;
            if !valid {
                return false;
            }
            match result {
                Ok(entries) => {
                    let rows = entries
                        .into_iter()
                        .map(|(letter, path)| (letter.to_string(), path.display().to_string()))
                        .collect();
                    arm_chord(app, ChordKind::BookmarkGo, rows);
                }
                Err(error) => {
                    cancel_chord(app);
                    app.tenxer.flash.show(
                        rust_i18n::t!(
                            "Could not access bookmarks: %{error}",
                            error = io_error_detail(&error)
                        ),
                        tx,
                    );
                }
            }
            true
        });
    });
}

pub(super) fn complete(app: &mut App, kind: ChordKind, press: &KeyPress, tx: &mut Tx) -> bool {
    let Some(letter) = press.letter() else {
        return false;
    };
    if kind == ChordKind::BookmarkSet {
        save(app, letter, tx);
    } else {
        jump(app, letter);
    }
    true
}

fn save(app: &mut App, letter: char, tx: &mut Tx) {
    let Some(directory) = bookmark_folder(app) else {
        app.tenxer
            .flash
            .show(tr("Only local folders can be bookmarked"), tx);
        return;
    };
    let later = app.tenxer.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let result = letter_bookmarks::set(letter, directory).await;
        later.then(move |app, tx| {
            let key = letter.to_string();
            let message = match result {
                Ok(()) => rust_i18n::t!("Saved bookmark %{key}", key = key).into_owned(),
                Err(error) => rust_i18n::t!(
                    "Could not access bookmarks: %{error}",
                    error = io_error_detail(&error)
                )
                .into_owned(),
            };
            app.tenxer.flash.show(message, tx);
            true
        });
    });
}

struct PendingJump {
    letter: char,
    expected: u64,
    generation: u64,
    origin: Option<PathBuf>,
}

enum JumpOutcome {
    Navigate(PathBuf),
    Unavailable,
    Missing,
    Failed(io::Error),
}

async fn resolve_jump(letter: char) -> JumpOutcome {
    match letter_bookmarks::get(letter).await {
        Ok(Some(path)) => {
            let valid = matches!(
                glib::future_with_timeout(mount_lanes::STALLED_AFTER, probes::folder(&path)).await,
                Ok(Ok(()))
            );
            if valid {
                JumpOutcome::Navigate(path)
            } else {
                JumpOutcome::Unavailable
            }
        }
        Ok(None) => JumpOutcome::Missing,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => JumpOutcome::Unavailable,
        Err(error) => JumpOutcome::Failed(error),
    }
}

fn jump(app: &mut App, letter: char) {
    arm_chord(app, ChordKind::BookmarkGo, Vec::new());
    app.tenxer.bookmark_pending = true;
    let pending = PendingJump {
        letter,
        expected: app.tenxer.bookmark_revision,
        generation: app.browser.navigation_generation(),
        origin: bookmark_folder(app),
    };
    let later = app.tenxer.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let outcome = resolve_jump(letter).await;
        later.then(move |app, tx| finish_jump(app, &pending, outcome, tx));
    });
}

fn finish_jump(app: &mut App, pending: &PendingJump, outcome: JumpOutcome, tx: &mut Tx) -> bool {
    let was_pending = std::mem::replace(&mut app.tenxer.bookmark_pending, false);
    if app.tenxer.bookmark_revision != pending.expected || !was_pending {
        return false;
    }
    cancel_chord(app);
    if !jump_target_current(app, pending, tx) {
        return true;
    }
    apply_jump_outcome(app, pending.letter, outcome, tx);
    true
}

fn jump_target_current(app: &App, pending: &PendingJump, tx: &Tx) -> bool {
    app.browser.navigation_generation() == pending.generation
        && shell::preferences(app).tenxer_mode
        && Situation::of(app, tx).in_view()
        && bookmark_folder(app) == pending.origin
}

fn apply_jump_outcome(app: &mut App, letter: char, outcome: JumpOutcome, tx: &mut Tx) {
    let key = letter.to_string();
    match outcome {
        JumpOutcome::Navigate(path) => {
            shell::publish(app);
            app.browser
                .clone()
                .navigate_location(Location::local(path), true);
            shell::settle(app, tx);
        }
        JumpOutcome::Unavailable => {
            app.tenxer.flash.show(
                rust_i18n::t!("Bookmark %{key} is unavailable", key = key),
                tx,
            );
        }
        JumpOutcome::Missing => {
            app.tenxer
                .flash
                .show(rust_i18n::t!("No bookmark %{key}", key = key), tx);
        }
        JumpOutcome::Failed(error) => {
            app.tenxer.flash.show(
                rust_i18n::t!(
                    "Could not access bookmarks: %{error}",
                    error = io_error_detail(&error)
                ),
                tx,
            );
        }
    }
}
