// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::Tx;
use strata_core::i18n::tr;
use strata_desktop::keys::{Command, KeyPress, bindings::*};

use super::prompts::{self, find};
use crate::{
    App,
    clipboard::{self, Yank, paths},
    commands::Outcome,
    conflict_dialog::ConflictFocus,
    open_with, transfer, trash,
};

type Verb = fn(&mut App, &mut Tx);

/// `/`, `?`, `n`, `N`, `z`, `Z`, `f` and `s`.
const PROMPT_KEYS: &[(Command, Verb)] = &[
    (FIND, |app, tx| prompts::open_find(app, false, tx)),
    (FIND_BACKWARD, |app, tx| prompts::open_find(app, true, tx)),
    (JUMP_VISITED, |app, tx| {
        prompts::open_history(app, false, tx)
    }),
    (JUMP_RECENT, |app, tx| prompts::open_history(app, true, tx)),
    (REPEAT_FIND, |app, tx| find::repeat(app, false, tx)),
    (REPEAT_FIND_REVERSED, |app, tx| find::repeat(app, true, tx)),
    (FILTER, prompts::open_filter),
    (SEARCH_SUBFOLDERS, prompts::open_search),
];

/// Rename, Open With, delete, create, move or copy to a folder, yank and paste.
const FILE_VERBS: &[(Command, Verb)] = &[
    (RENAME, prompts::open_rename),
    (OPEN_WITH, open_with_targets),
    (MOVE_TO_TRASH, |app, tx| delete_targets(app, false, tx)),
    (DELETE_PERMANENTLY, |app, tx| delete_targets(app, true, tx)),
    (CREATE, prompts::open_create),
    (MOVE_TO, |app, tx| prompts::open_transfer(app, true, tx)),
    (COPY_TO, |app, tx| prompts::open_transfer(app, false, tx)),
    (YANK, |app, tx| yank(app, false, tx)),
    (YANK_CUT, |app, tx| yank(app, true, tx)),
    (UNYANK, |app, _| clipboard::unyank(app)),
    (PASTE_KEEP_BOTH, |app, tx| {
        paste(app, ConflictFocus::KeepBoth, tx)
    }),
    (PASTE_REPLACE, |app, tx| {
        paste(app, ConflictFocus::Replace, tx)
    }),
    (RESTORE, restore_targets),
];

pub(super) fn prompt_keys(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    run_bound(PROMPT_KEYS, app, press, tx)
}

pub(super) fn file_verbs(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    run_bound(FILE_VERBS, app, press, tx)
}

/// Runs the verb of the first command in `table` that `press` is.
fn run_bound(
    table: &[(Command, Verb)],
    app: &mut App,
    press: &KeyPress,
    tx: &mut Tx,
) -> Option<Outcome> {
    let (_, verb) = table.iter().find(|(command, _)| press.is(*command))?;
    verb(app, tx);
    Some(Outcome::Ran)
}

fn yank(app: &mut App, cut: bool, tx: &mut Tx) {
    let refusal = match clipboard::yank_targets(app, cut) {
        Yank::Nothing if cut => "Nothing to cut",
        Yank::Nothing => "Nothing to yank",
        Yank::Refused => "Can\u{2019}t cut these items",
        Yank::Done => return,
    };
    app.tenxer.flash.show(tr(refusal), tx);
}

fn paste(app: &mut App, focus: ConflictFocus, tx: &mut Tx) {
    transfer::paste_preferring(
        app,
        focus,
        Rc::new(|app: &mut App, tx: &mut Tx| {
            app.tenxer.flash.show(tr("Nothing to paste"), tx);
        }),
        tx,
    );
}

fn restore_targets(app: &mut App, tx: &mut Tx) {
    let entries = super::command_targets(app);
    let refusal = if entries.is_empty() {
        "Nothing to restore"
    } else if !entries
        .iter()
        .all(|entry| paths::is_trash_item(&entry.location))
    {
        "Only items in Trash can be restored"
    } else {
        return;
    };
    app.tenxer.flash.show(tr(refusal), tx);
}

fn delete_targets(app: &mut App, permanent: bool, tx: &mut Tx) {
    if !trash::confirm_tenxer_delete(app, permanent) {
        app.tenxer.flash.show(tr("Nothing to delete"), tx);
    }
}

fn open_with_targets(app: &mut App, tx: &mut Tx) {
    let report: open_with::RefusalReport = Rc::new(|app: &mut App, reason: &str, tx: &mut Tx| {
        app.tenxer.flash.show(tr(reason), tx);
    });
    let _ = open_with::open_targets(app, tx, Some(report));
}
