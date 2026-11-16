// SPDX-License-Identifier: MIT

//! Armed 10xer chord completion for sort (`,`), copy (`c`), tabs (`t`), and actions (`;` / `; .`),
//! ported from `src/ui/window/keyboard/chords.rs` and `src/ui/window/keyboard/files.rs`.

use rune::Tx;
use strata_core::{
    i18n::tr,
    model::{ArchiveFormat, FileEntry, SortDirection, SortKey},
};
use strata_desktop::keys::{
    self as key_catalog, Binding, Command, KeyPress, Keys, Modifiers, bindings::*,
};

use super::{
    ChordKind, arm_chord, bookmarks, cancel_chord, command_targets, focused_listing_depth, go,
};
use crate::{
    App, archive, clipboard,
    commands::{self, On, Outcome},
    compress::{self, CompressAttempt},
    shell::tabs::{shown_tab, tabs},
    tab_strip, terminal,
};

pub(super) fn handle_armed_chord(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Outcome {
    let Some(kind) = app.tenxer.armed else {
        return Outcome::Declined;
    };
    cancel_chord(app);
    // Settings opens over a canceled chord (`keyboard/chords.rs:134`).
    if press.is(SETTINGS) {
        return Outcome::Declined;
    }
    if press.is(ESCAPE) {
        return Outcome::Ran;
    }
    let completed = match kind {
        ChordKind::Sort => complete_sort(app, press),
        ChordKind::Go => go::complete(app, press, tx),
        ChordKind::Copy => complete_copy(app, press, tx),
        ChordKind::Action => complete_action(app, press, tx),
        ChordKind::FolderAction => complete_folder_action(app, press, tx),
        ChordKind::Tabs => complete_tab(app, press, tx),
        ChordKind::BookmarkSet | ChordKind::BookmarkGo => bookmarks::complete(app, kind, press, tx),
    };
    if !completed {
        app.tenxer.flash.show(tr("Unknown chord"), tx);
    }
    Outcome::Ran
}

/// Shift with a step reverses it, on whatever key the step is bound to (`sort_choice` in
/// `src/ui/window/keyboard/files.rs`).
fn sort_choice(press: &KeyPress) -> Option<(SortKey, SortDirection)> {
    const STEPS: [(Command, SortKey); 7] = [
        (SORT_BY_NAME, SortKey::Name),
        (SORT_BY_MODIFIED, SortKey::Modified),
        (SORT_BY_SIZE, SortKey::Size),
        (SORT_BY_TYPE, SortKey::Type),
        (SORT_BY_EXTENSION, SortKey::Extension),
        (SORT_BY_CREATED, SortKey::Created),
        (SORT_BY_UNSORTED, SortKey::Unsorted),
    ];
    let reversed = |step: Command| {
        step.bindings().into_iter().any(|binding| {
            press.is(Binding::new(
                binding.key(),
                binding.modifiers() | Modifiers::SHIFT_MASK,
            ))
        })
    };
    STEPS
        .into_iter()
        .find(|(step, _)| press.is(*step))
        .map(|(_, sort_key)| (sort_key, SortDirection::Ascending))
        .or_else(|| {
            STEPS
                .into_iter()
                .find(|(step, _)| reversed(*step))
                .map(|(_, sort_key)| (sort_key, SortDirection::Descending))
        })
}

fn complete_sort(app: &mut App, press: &KeyPress) -> bool {
    if press.is(SORT_BY_FOLDERS_FIRST) {
        toggle_folders_first(app);
        return true;
    }
    if is_name_comparison_step(press) {
        change_name_comparison(app, press);
        return true;
    }
    let Some((sort_key, direction)) = sort_choice(press) else {
        return false;
    };
    if let Some(depth) = focused_listing_depth(app) {
        app.browser.clone().set_sort(depth, sort_key, direction);
    }
    true
}

fn toggle_folders_first(app: &App) {
    if let Some(depth) = focused_listing_depth(app) {
        app.browser.clone().toggle_folders_first(depth);
    }
}

fn is_name_comparison_step(press: &KeyPress) -> bool {
    press.is_any(&[
        SORT_BY_NATURAL,
        SORT_BY_ALPHABETICAL,
        SORT_BY_CASE_SENSITIVE,
    ])
}

fn change_name_comparison(app: &App, press: &KeyPress) {
    let Some(depth) = focused_listing_depth(app) else {
        return;
    };
    let browser = app.browser.clone();
    let mut comparison = browser
        .requested_column_preferences(depth)
        .unwrap_or_default()
        .name_comparison;
    if press.is(SORT_BY_CASE_SENSITIVE) {
        comparison.case_sensitive = !comparison.case_sensitive;
    } else {
        comparison.natural = press.is(SORT_BY_NATURAL);
    }
    browser.set_name_comparison(depth, comparison);
}

fn complete_copy(app: &mut App, press: &KeyPress, tx: &mut Tx) -> bool {
    let names = if press.is(COPY_AS_PATH) {
        false
    } else if press.is(COPY_AS_NAME) {
        true
    } else {
        return false;
    };
    let entries = command_targets(app);
    if !clipboard::copy_entries_text(app, &entries, names) {
        app.tenxer.flash.show(tr("Nothing to copy"), tx);
    }
    true
}

fn complete_tab(app: &mut App, press: &KeyPress, tx: &mut Tx) -> bool {
    if press.is(TAB_CHORD_NEW) {
        tab_strip::new_tab(app, tx);
    } else if press.is(TAB_CHORD_CLOSE) {
        tab_strip::close(app, shown_tab(app), tx);
    } else if press.is(TAB_CHORD_PREVIOUS) {
        commands::run(app, "tab-previous", On::Focus, tx);
    } else if let Some(digit) = press.digit_in(TAB_CHORD_NUMBERS) {
        let index = (usize::from(digit) + 9) % 10;
        if let Some(&key) = tabs(app).get(index) {
            tab_strip::select(app, key, tx);
        }
    } else {
        return false;
    }
    true
}

pub(super) fn arm_actions(app: &mut App, folder_only: bool) {
    let mut rows = vec![(
        key_catalog::label(&Keys::Digits(ACTIONS_NUMBERS.step())),
        tr("No matching actions"),
    )];
    let kind = if folder_only {
        ChordKind::FolderAction
    } else {
        rows.extend(
            [
                (ACTIONS_FOLDER, "Actions for this folder\u{2026}"),
                (ACTIONS_TERMINAL, "Open terminal here"),
                (ACTIONS_COMPRESS, "Compress\u{2026}"),
                (ACTIONS_EXTRACT, "Extract here"),
                (ACTIONS_EXTRACT_TO, "Extract to\u{2026}"),
            ]
            .map(|(step, action)| (key_catalog::label(&Keys::One(step.shown())), tr(action))),
        );
        ChordKind::Action
    };
    arm_chord(app, kind, rows);
}

fn complete_action(app: &mut App, press: &KeyPress, tx: &mut Tx) -> bool {
    if complete_lettered_action(app, press, tx) {
        return true;
    }
    if press.is(ACTIONS_FOLDER) {
        arm_actions(app, true);
        return true;
    }
    complete_numbered_slot(app, press, ACTIONS_NUMBERS, tx)
}

fn complete_lettered_action(app: &mut App, press: &KeyPress, tx: &mut Tx) -> bool {
    if press.is(ACTIONS_TERMINAL) {
        open_action_terminal(app, tx);
    } else if press.is(ACTIONS_COMPRESS) {
        open_action_compress(app, tx);
    } else if press.is(ACTIONS_EXTRACT) {
        run_action_extract(app, tx);
    } else if press.is(ACTIONS_EXTRACT_TO) {
        run_action_extract_to(app, tx);
    } else {
        return false;
    }
    true
}

fn open_action_terminal(app: &mut App, tx: &mut Tx) {
    if !terminal::open_focused_folder(app) {
        app.tenxer.flash.show(tr("Can’t open a terminal here"), tx);
    }
}

fn open_action_compress(app: &mut App, tx: &mut Tx) {
    match compress::open_for_focus(app) {
        CompressAttempt::Nothing => app.tenxer.flash.show(tr("Nothing to compress"), tx),
        CompressAttempt::Refused => app.tenxer.flash.show(tr("Can’t compress these items"), tx),
        CompressAttempt::Started => {}
    }
}

fn run_action_extract(app: &mut App, tx: &mut Tx) {
    match extract_target(app) {
        Ok(entry) => archive::extract_entry(app, entry, tx),
        Err(reason) => app.tenxer.flash.show(tr(reason), tx),
    }
}

fn run_action_extract_to(app: &mut App, tx: &mut Tx) {
    if let Err(reason) = extract_target(app) {
        app.tenxer.flash.show(tr(reason), tx);
    }
}

fn complete_folder_action(app: &mut App, press: &KeyPress, tx: &mut Tx) -> bool {
    complete_numbered_slot(app, press, ACTIONS_FOLDER_NUMBERS, tx)
}

fn complete_numbered_slot(
    app: &mut App,
    press: &KeyPress,
    digits: strata_desktop::keys::Digits,
    tx: &mut Tx,
) -> bool {
    let Some(digit) = press.digit_in(digits) else {
        return false;
    };
    app.tenxer
        .flash
        .show(rust_i18n::t!("No action %{digit}", digit = digit), tx);
    true
}

fn extract_target(app: &mut App) -> Result<FileEntry, &'static str> {
    let mut entries = command_targets(app);
    let entry = match entries.len() {
        0 => return Err("Nothing to extract"),
        1 => entries.remove(0),
        _ => return Err("Extract one archive at a time"),
    };
    if ArchiveFormat::for_entry(&entry).is_none() {
        return Err("Not an archive");
    }
    Ok(entry)
}
