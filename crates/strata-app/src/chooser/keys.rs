// SPDX-License-Identifier: MIT

//! Chooser keyboard bindings: Escape ladder, `Ctrl+Return` folder selection, 10xer Save/Edit-name,
//! and chooser command refusals (`src/ui/chooser/shortcuts/editing.rs` and
//! `src/ui/window/keyboard/chooser.rs`).

use rune::Tx;
use strata_core::i18n::tr;
use strata_desktop::keys::{
    Command, KeyPress, Map, Modifiers,
    bindings::{
        ACTIONS, ACTIONS_COMPRESS, ACTIONS_TERMINAL, BOOKMARK_GO, BOOKMARK_SET, CHOOSE_FOLDER,
        CLOSE_WINDOW, COPY, COPY_PATH, COPY_TO, CUSTOM_ACTION_1, CUSTOM_ACTION_2, CUSTOM_ACTION_3,
        CUSTOM_ACTION_4, CUSTOM_ACTION_5, CUSTOM_ACTION_6, CUSTOM_ACTION_7, CUSTOM_ACTION_8,
        CUSTOM_ACTION_9, CUSTOM_ACTION_10, CUT, DUPLICATE, ENTER, ESCAPE, ESCAPE_ANY,
        ESCAPE_ANY_SHIFT, INVERT_SELECTION, JUMP_TO_FOLDER, MOVE_TO, NEW_FOLDER_WITH_SELECTION,
        OPEN_IN_NEW_TAB, OPEN_WITH, PASTE, PASTE_KEEP_BOTH, PASTE_REPLACE, PREVIEW, REDO, RENAME,
        RESTORE, SEARCH, SELECT_ALL, SETTINGS, SPACE, TAB_CLOSE, TAB_MOVE_LEFT, TAB_MOVE_RIGHT,
        TAB_NEW, TAB_NEXT, TAB_PREVIOUS, TAB_REOPEN, TERMINAL, UNDO, UNYANK, VISUAL_SELECT,
        VISUAL_UNSET, YANK, YANK_CUT,
    },
};

use super::{NAME_KEY, accept};
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Reads, Situation, Stage},
    portal::ChooserKind,
    shell,
};

const UNAVAILABLE: &str = "Not available in the file chooser";

/// The window's commands GTK's chooser has no stage for (`PRESSED_STAGES` and
/// `NAVIGATION_STAGES` in `src/ui/chooser/shortcuts.rs`): outside 10xer mode its Ctrl keys go
/// to the widgets (`control_keeps_key`) and the rest reach nothing, so these keys do nothing in
/// a chooser. 10xer mode reads the window's keys there, behind the chooser's refusals.
const NOT_IN_CHOOSER: &[Command] = &[
    COPY,
    CUT,
    PASTE,
    DUPLICATE,
    UNDO,
    REDO,
    COPY_PATH,
    OPEN_WITH,
    OPEN_IN_NEW_TAB,
    NEW_FOLDER_WITH_SELECTION,
    JUMP_TO_FOLDER,
    TERMINAL,
    ACTIONS_TERMINAL,
    ACTIONS_COMPRESS,
    CUSTOM_ACTION_1,
    CUSTOM_ACTION_2,
    CUSTOM_ACTION_3,
    CUSTOM_ACTION_4,
    CUSTOM_ACTION_5,
    CUSTOM_ACTION_6,
    CUSTOM_ACTION_7,
    CUSTOM_ACTION_8,
    CUSTOM_ACTION_9,
    CUSTOM_ACTION_10,
    TAB_NEW,
    TAB_CLOSE,
    TAB_REOPEN,
    TAB_NEXT,
    TAB_PREVIOUS,
    TAB_MOVE_LEFT,
    TAB_MOVE_RIGHT,
    SETTINGS,
    SEARCH,
];

/// Whether a key may not run `action` here: a chooser outside 10xer mode, and a command its
/// key table lacks ([`NOT_IN_CHOOSER`]).
pub(crate) fn declines_key(app: &App, action: &Action<App>) -> bool {
    app.chooser.is_active()
        && !shell::preferences(app).tenxer_mode
        && action
            .command
            .is_some_and(|command| NOT_IN_CHOOSER.contains(&command))
}
const SINGLE_ITEM: &str = "Only one item can be chosen";

fn escape_outside_text(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if app
        .chooser
        .active
        .as_mut()
        .is_some_and(super::state::ActiveChooser::dismiss_dropdown)
    {
        return Outcome::Ran;
    }
    if crate::preview::dismiss_escape(app, tx) {
        return Outcome::Ran;
    }
    if app
        .chooser
        .active
        .as_mut()
        .is_some_and(super::state::ActiveChooser::cancel_download)
    {
        return Outcome::Ran;
    }
    accept::cancel(app, tx);
    Outcome::Ran
}

fn escape_in_name(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if shell::preferences(app).tenxer_mode {
        if let Some(active) = app.chooser.active.as_mut() {
            active.name.focused = false;
        }
        if let Some(view) = shell::view_focus(app) {
            tx.focus(view, true);
        }
        return Outcome::Ran;
    }
    accept::cancel(app, tx);
    Outcome::Ran
}

fn choose_folder(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    accept::accept(app, tx);
    Outcome::Ran
}

fn save_enter(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    accept::accept(app, tx);
    Outcome::Ran
}

fn edit_name(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    let Some(active) = app.chooser.active.as_mut().filter(|a| a.has_name) else {
        return Outcome::Declined;
    };
    active.name.focused = true;
    active.select_name_stem();
    tx.focus(NAME_KEY, true);
    Outcome::Ran
}

fn refusal_reason(press: &KeyPress, multiple: bool, in_view: bool) -> Option<&'static str> {
    if press.is(CLOSE_WINDOW) {
        return Some(UNAVAILABLE);
    }
    if !in_view {
        return None;
    }
    if press.is(PREVIEW.in_map(Map::Tenxer)) {
        return Some(UNAVAILABLE);
    }
    if press.is_any(&[
        YANK,
        PASTE_KEEP_BOTH,
        YANK_CUT,
        CUT,
        COPY,
        PASTE,
        BOOKMARK_SET,
        BOOKMARK_GO,
        ACTIONS,
        UNYANK,
        PASTE_REPLACE,
        OPEN_WITH,
        MOVE_TO,
        COPY_TO,
        RESTORE,
    ]) {
        return Some(UNAVAILABLE);
    }
    let fills = press.is(SPACE)
        || press.is_any(&[VISUAL_SELECT, VISUAL_UNSET, SELECT_ALL, INVERT_SELECTION]);
    (fills && !multiple).then_some(SINGLE_ITEM)
}

fn reads_refusal(press: &KeyPress) -> bool {
    refusal_reason(press, false, true).is_some()
}

fn refuse_command(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    let Some(press) = call.press else {
        return Outcome::Declined;
    };
    let multiple = app.chooser.allows_multiple();
    let in_view = Situation::of(app, tx).in_view();
    let Some(reason) = refusal_reason(press, multiple, in_view) else {
        return Outcome::Declined;
    };
    crate::tenxer::show_flash(app, tr(reason), tx);
    Outcome::Ran
}

/// GTK refuses these only among its 10xer keys (`tenxer_window_keys`), which read nothing
/// outside 10xer mode: there Space previews, as in the window.
fn refuses_commands(app: &App, _: &Situation) -> bool {
    app.chooser.is_active() && shell::preferences(app).tenxer_mode
}

fn is_chooser_outside_text(app: &App, at: &Situation) -> bool {
    app.chooser.is_active() && !at.text()
}

fn is_chooser_name_focused(app: &App, _at: &Situation) -> bool {
    app.chooser.name_focused()
}

fn can_choose_folder(app: &App, at: &Situation) -> bool {
    app.chooser.chooses_directory() && at.in_view()
}

fn is_tenxer_save_view(app: &App, at: &Situation) -> bool {
    shell::preferences(app).tenxer_mode
        && at.in_view()
        && app.chooser.active.as_ref().is_some_and(|active| {
            matches!(
                active.request.kind,
                ChooserKind::SaveFile { .. } | ChooserKind::SaveFiles { .. }
            )
        })
}

fn is_tenxer_save_file_view(app: &App, at: &Situation) -> bool {
    shell::preferences(app).tenxer_mode
        && at.in_view()
        && app.chooser.active.as_ref().is_some_and(|active| {
            active.has_name && matches!(active.request.kind, ChooserKind::SaveFile { .. })
        })
}

pub(crate) const COMMANDS: &[Action<App>] = &[
    Action::new("chooser-refusal", refuse_command).keys(&[Keys {
        stage: Stage::TenxerKeys,
        reads: Reads::Press(reads_refusal),
        when: refuses_commands,
    }]),
    Action::new("chooser-escape-name", escape_in_name)
        .command(ESCAPE)
        .keys(&[Keys::at(Stage::InlineEditing, &[ESCAPE_ANY]).when(is_chooser_name_focused)]),
    Action::new("chooser-escape", escape_outside_text)
        .command(ESCAPE)
        .keys(&[Keys::at(Stage::Dismissal, &[ESCAPE_ANY_SHIFT]).when(is_chooser_outside_text)]),
    Action::new("choose-folder", choose_folder)
        .command(CHOOSE_FOLDER)
        .keys(&[Keys::at(
            Stage::FileCommands,
            &[CHOOSE_FOLDER.ignoring(Modifiers::SUPER_MASK)],
        )
        .when(can_choose_folder)]),
    Action::new("chooser-save-enter", save_enter)
        .command(ENTER)
        .keys(&[
            Keys::at(Stage::TenxerKeys, &[ENTER.ignoring(Modifiers::empty())])
                .when(is_tenxer_save_view),
        ]),
    Action::new("chooser-edit-name", edit_name)
        .command(RENAME)
        .keys(&[Keys::at(Stage::TenxerKeys, &[RENAME.in_map(Map::Tenxer)])
            .when(is_tenxer_save_file_view)]),
];
