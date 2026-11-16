// SPDX-License-Identifier: MIT

//! 10xer mode's key stage (`Stage::TenxerKeys`), mode toggle (`Ctrl+Shift+M`), Escape ladder
//! and non-Latin layout cursor/range fallback (`src/ui/window/keyboard/tenxer.rs`,
//! `items/tenxer_items.rs`, `escape.rs`).

use rune::Tx;
use strata_components::{
    collection::keyboard::{Command as Nav, RangeKind, Target, Vertical},
    contract::Direction,
};
use strata_core::i18n::tr;
use strata_desktop::keys::{
    self as key_catalog, KeyPress, Keys as KeyLabels, Map, Modifiers, bindings::*,
};

use super::{
    ChordKind, arm_chord, cancel_chord, end_active_range, end_session, run_collection_command,
};
use crate::{
    App,
    commands::{self, Action, Call, Holds, Keys, On, Outcome, Reads, Situation, Stage},
    filter, preview, shell,
};

pub(super) const COMMANDS: &[Action<App>] = &[
    Action::new("tenxer-mode", toggle_mode)
        .command(TENXER_MODE)
        .keys(&[
            Keys::at(
                Stage::TenxerKeys,
                &[TENXER_MODE.ignoring(Modifiers::empty())],
            )
            .when(outside_modal),
            Keys::at(
                Stage::DirectCommands,
                &[TENXER_MODE.ignoring(Modifiers::empty())],
            )
            .when(outside_modal),
            Keys::at(
                Stage::AfterYield,
                &[TENXER_MODE.ignoring(Modifiers::empty())],
            ),
        ]),
    Action::new("tenxer-keys", tenxer_stage).keys(&[Keys {
        stage: Stage::TenxerKeys,
        reads: Reads::Press(|press| press.modifier_key().is_empty()),
        when: tenxer_active,
    }]),
];

fn outside_modal(_: &App, at: &Situation) -> bool {
    !matches!(at.holds(), Some(Holds::Modal { .. }))
}

fn tenxer_active(app: &App, at: &Situation) -> bool {
    shell::preferences(app).tenxer_mode
        && !matches!(
            at.holds(),
            Some(Holds::Modal { .. } | Holds::Menu | Holds::Popover)
        )
}

fn toggle_mode(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    let mut enabled = false;
    shell::saved(app).change(|settings| {
        settings.tenxer_mode = !settings.tenxer_mode;
        enabled = settings.tenxer_mode;
    });
    shell::live_keys(app).follow(shell::preferences(app));
    if !enabled {
        end_session(app);
    }
    Outcome::Ran
}

fn tenxer_stage(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    let Some(press) = call.press else {
        return Outcome::Declined;
    };
    let at = Situation::of(app, tx);
    if at.text() {
        cancel_chord(app);
        return Outcome::Declined;
    }
    window_or_chord(app, press, tx)
        .or_else(|| chrome_or_escape(app, press, &at, tx))
        .or_else(|| listing_or_unbound(app, press, &at, tx))
        .unwrap_or(Outcome::Declined)
}

fn window_or_chord(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    if app.tenxer.armed.is_some() {
        return Some(handle_armed_chord(app, press, tx));
    }
    if press.is(CLOSE_WINDOW) {
        shell::close_window(app, tx);
        return Some(Outcome::Ran);
    }
    None
}

fn chrome_or_escape(
    app: &mut App,
    press: &KeyPress,
    at: &Situation,
    tx: &mut Tx,
) -> Option<Outcome> {
    if at.within("sidebar") {
        return Some(Outcome::Declined);
    }
    if press.is(ESCAPE_ANY_SHIFT) && at.holds() != Some(Holds::Edit) {
        escape_ladder(app, tx);
        return Some(Outcome::Ran);
    }
    None
}

fn listing_or_unbound(
    app: &mut App,
    press: &KeyPress,
    at: &Situation,
    tx: &mut Tx,
) -> Option<Outcome> {
    if at.in_view()
        && let Some(outcome) = listing_keys(app, press, tx)
    {
        return Some(outcome);
    }
    claims_unbound_command(press).then_some(Outcome::Ran)
}

fn handle_armed_chord(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Outcome {
    let Some(kind) = app.tenxer.armed else {
        return Outcome::Declined;
    };
    cancel_chord(app);
    if press.is(ESCAPE_ANY) {
        return Outcome::Ran;
    }
    match kind {
        ChordKind::Go if press.is(GO_FIRST) => {
            run_collection_command(app, Nav::Cursor(Target::First), tx);
            Outcome::Ran
        }
        ChordKind::Go => {
            app.tenxer.flash.show(tr("Unknown chord"), tx);
            Outcome::Ran
        }
    }
}

fn arm_go(app: &mut App) {
    let cap = key_catalog::label(&KeyLabels::One(GO_FIRST.shown()));
    arm_chord(app, ChordKind::Go, vec![(cap, tr("First item"))]);
}

/// 10xer's Escape ladder (`src/ui/window/keyboard/escape.rs`): dismisses one interaction per
/// press, and never closes a Miller column.
fn escape_ladder(app: &mut App, tx: &mut Tx) {
    if app.browser.close_peek() {
        return;
    }
    if filter::search_active(app) {
        let _ = end_active_range(app)
            || preview::dismiss_escape(app, tx)
            || filter::dismiss_search_only(app, tx);
        return;
    }
    let _ = end_active_range(app)
        || filter::dismiss_filter_only(app, tx)
        || preview::dismiss_escape(app, tx)
        || app.browser.clear_active_selection();
}

fn shows(app: &App, mode: &str) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)(mode))
}

fn listing_keys(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    if let Some(outcome) = alt_navigation(app, press, tx) {
        return Some(outcome);
    }
    let icons = shows(app, "icons");
    if icons {
        icons_keys(app, press, tx)
    } else {
        rows_keys(app, press, tx)
    }
}

fn alt_navigation(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    if press.is(HISTORY_BACK) {
        Some(commands::run(app, "history-back", On::Focus, tx))
    } else if press.is(HISTORY_FORWARD) {
        Some(commands::run(app, "history-forward", On::Focus, tx))
    } else if press.is(PARENT_FOLDER) {
        Some(commands::run(app, "parent-folder", On::Focus, tx))
    } else {
        None
    }
}

fn icons_keys(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    if press.is(THUMBNAILS_LARGER) {
        return Some(commands::run(app, "thumbnails-larger", On::Focus, tx));
    }
    if press.is(THUMBNAILS_SMALLER) {
        return Some(commands::run(app, "thumbnails-smaller", On::Focus, tx));
    }
    if press.is(BACKSPACE) {
        return Some(commands::run(app, "navigate-up", On::Focus, tx));
    }
    if press.is(GO) {
        arm_go(app);
        return Some(Outcome::Ran);
    }
    run_non_ascii_collection(app, press, true, tx)
}

fn rows_keys(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    let left = press.is(LEFT.or_alternates());
    if left && filter::dismiss_search_only(app, tx) {
        return Some(Outcome::Ran);
    }
    if left || press.is(BACKSPACE) {
        return Some(commands::run(app, "navigate-up", On::Focus, tx));
    }
    if press.is(RIGHT.or_alternates()) {
        enter_or_open_folder(app, tx);
        return Some(Outcome::Ran);
    }
    if press.is(GO) {
        arm_go(app);
        return Some(Outcome::Ran);
    }
    run_non_ascii_collection(app, press, false, tx)
}

fn enter_or_open_folder(app: &mut App, tx: &mut Tx) {
    let search = filter::targets::selected(app).is_some();
    let entry = if search {
        filter::targets::under_cursor(app).map(|(_, entry)| entry)
    } else {
        app.browser.focused_entry()
    };
    if let Some(dir) = entry.as_ref().filter(|entry| entry.is_directory()) {
        if search {
            app.browser.navigate(dir.location.clone());
        } else if shows(app, "columns") {
            app.browser.activate_focused();
        } else {
            app.browser.activate_focused_in_place();
        }
        return;
    }
    match preview::preview_target(entry) {
        Some(target) => preview::toggle_entry(app, Some(target), app.browser.active_depth(), tx),
        None => app.tenxer.flash.show(tr("Nothing to preview"), tx),
    }
}

fn run_non_ascii_collection(
    app: &mut App,
    press: &KeyPress,
    icons: bool,
    tx: &mut Tx,
) -> Option<Outcome> {
    if !press.used_fallback() && !press.aliased() {
        return None;
    }
    let command = collection_command(press, icons)?;
    run_collection_command(app, command, tx).then_some(Outcome::Ran)
}

fn collection_command(press: &KeyPress, icons: bool) -> Option<Nav> {
    if let Some(dir) = step_direction(press, icons) {
        return Some(Nav::Cursor(Target::Step(dir)));
    }
    const MAPPED: [(key_catalog::Command, Nav); 8] = [
        (VISUAL_SELECT, Nav::Range(RangeKind::Select)),
        (VISUAL_UNSET, Nav::Range(RangeKind::Unset)),
        (LAST_ITEM, Nav::Cursor(Target::Last)),
        (HALF_PAGE_UP, Nav::Cursor(Target::HalfPage(Vertical::Up))),
        (
            HALF_PAGE_DOWN,
            Nav::Cursor(Target::HalfPage(Vertical::Down)),
        ),
        (SELECT_ALL, Nav::SelectAll),
        (INVERT_SELECTION, Nav::Invert),
        (OPEN, Nav::Activate),
    ];
    MAPPED
        .into_iter()
        .find(|(command, _)| press.is(*command))
        .map(|(_, nav)| nav)
}

fn step_direction(press: &KeyPress, icons: bool) -> Option<Direction> {
    if press.is(DOWN.or_alternates()) {
        Some(Direction::Down)
    } else if press.is(UP.or_alternates()) {
        Some(Direction::Up)
    } else if icons && press.is(LEFT.or_alternates()) {
        Some(Direction::Left)
    } else if icons && press.is(RIGHT.or_alternates()) {
        Some(Direction::Right)
    } else {
        None
    }
}

/// Default-map keys of window commands that 10xer mode leaves unbound or binds to other keys
/// (`claims_unbound_command` in `src/ui/window/keyboard/tenxer.rs`).
fn claims_unbound_command(press: &KeyPress) -> bool {
    [
        DUPLICATE,
        FILTER,
        SIDEBAR,
        RENAME,
        ARROW_SCOPE,
        JUMP_TO_FOLDER,
    ]
    .into_iter()
    .any(|command| press.is(command.in_map(Map::Default)) && !press.is(command.in_map(Map::Tenxer)))
}
