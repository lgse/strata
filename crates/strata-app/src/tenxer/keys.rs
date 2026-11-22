// SPDX-License-Identifier: MIT

//! 10xer mode's key stage (`Stage::TenxerKeys`), mode toggle (`Ctrl+Shift+M`), Escape ladder,
//! chord prefix triggers, and non-Latin layout cursor/range fallback
//! (`src/ui/window/keyboard/tenxer.rs`, `items/tenxer_items.rs`, `files.rs`, `escape.rs`).

use rune::{Key, KeyEvent, Mods, Tx};
use strata_components::{
    collection::keyboard::{Command as Nav, RangeKind, Target, Vertical},
    contract::Direction,
};
use strata_core::i18n::tr;
use strata_desktop::keys::{self as key_catalog, KeyPress, Map, Modifiers, bindings::*};

use super::{
    ChordKind, arm_default_chord, bookmarks, cancel_chord, chords, end_active_range, end_session,
    preview_keys, prompts::find, run_collection_command, shows, verbs,
};
use crate::{
    App,
    commands::{self, Action, Call, Holds, Keys, On, Outcome, Reads, Situation, Stage},
    filter, preview, shell,
};

pub(super) const COMMANDS: &[Action<App>] = &[
    Action::new("tenxer-entry", bookmarks::on_entry_key).keys(&[Keys {
        stage: Stage::Entry,
        reads: Reads::Press(|press| !press.is_modifier()),
        when: |_, _| true,
    }]),
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
        reads: Reads::Press(|press| !press.is_modifier()),
        when: tenxer_active,
    }]),
    Action::new("tenxer-swallow-typing", swallow_unclaimed_typing).keys(&[Keys {
        stage: Stage::ItemNavigation,
        reads: Reads::Press(claims_file_list_typing),
        when: in_tenxer_listing,
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

fn in_tenxer_listing(app: &App, at: &Situation) -> bool {
    shell::preferences(app).tenxer_mode && at.in_view()
}

fn claims_file_list_typing(press: &KeyPress) -> bool {
    (press.is(SPACE.ignoring(Modifiers::SHIFT_MASK)) || filter::is_typing_key(press))
        && !press.is_any(key_catalog::direct_commands(Map::Tenxer))
}

fn swallow_unclaimed_typing(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    Outcome::Ran
}

fn toggle_mode(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    // The chrome 10xer hides rebuilds the view around the listing; GTK's listing keeps the
    // keyboard through the switch.
    let in_view = Situation::of(app, tx).in_view();
    let mut enabled = false;
    shell::saved(app).change(|settings| {
        settings.tenxer_mode = !settings.tenxer_mode;
        enabled = settings.tenxer_mode;
    });
    shell::live_keys(app).follow(shell::preferences(app));
    if !enabled {
        end_session(app, tx);
    }
    if in_view && let Some(view) = shell::view_focus(app) {
        tx.focus_when_shown(view, true);
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
    window_or_chord(app, press, &at, tx)
        .or_else(|| chrome_or_escape(app, press, &at, tx))
        .or_else(|| listing_or_unbound(app, press, &at, tx))
        .unwrap_or(Outcome::Declined)
}

fn window_or_chord(
    app: &mut App,
    press: &KeyPress,
    at: &Situation,
    tx: &mut Tx,
) -> Option<Outcome> {
    if app.tenxer.armed.is_some() {
        return Some(chords::handle_armed_chord(app, press, tx));
    }
    if let Some(outcome) = crate::chooser::refuse_key(app, press, at, tx) {
        return Some(outcome);
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
    if at.within("sidebar")
        && let Some(outcome) = tenxer_sidebar(app, press, tx)
    {
        return Some(outcome);
    }
    if press.is(ESCAPE_ANY_SHIFT) && at.holds() != Some(Holds::Edit) {
        escape_ladder(app, tx);
        return Some(Outcome::Ran);
    }
    None
}

fn tenxer_sidebar(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    let held = press.modifiers();
    if held.intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
        return None;
    }
    if held.contains(Modifiers::SHIFT_MASK) {
        return Some(Outcome::Ran);
    }
    sidebar_chord_outcome(app, press, tx)
        .or_else(|| sidebar_swallows(press).then_some(Outcome::Ran))
}

fn sidebar_swallows(press: &KeyPress) -> bool {
    press.is(RIGHT) || press.is(MOVE_TO_TRASH.in_map(Map::Default))
}

fn sidebar_chord_outcome(app: &App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    if press.is(LEFT.or_alternates()) || press.is(BACKSPACE) {
        if let Some(view) = shell::view_focus(app) {
            tx.focus(view, true);
        }
        return Some(Outcome::Ran);
    }
    if press.is(ENTER) || press.is(SPACE) {
        return Some(Outcome::Declined);
    }
    if press.is(RIGHT.alternates()) {
        send_plain(tx, Key::Enter);
        return Some(Outcome::Ran);
    }
    sidebar_step_outcome(press, tx)
}

fn sidebar_step_outcome(press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    if press.is(DOWN.or_alternates()) {
        return Some(step_or_yield(press, DOWN, Key::Down, tx));
    }
    if press.is(UP.or_alternates()) {
        return Some(step_or_yield(press, UP, Key::Up, tx));
    }
    None
}

fn step_or_yield(press: &KeyPress, arrow: key_catalog::Command, key: Key, tx: &mut Tx) -> Outcome {
    if !press.is(arrow.alternates()) {
        return Outcome::Declined;
    }
    send_plain(tx, key);
    Outcome::Ran
}

fn send_plain(tx: &mut Tx, key: Key) {
    tx.send_key(KeyEvent {
        key,
        mods: Mods::default(),
    });
}

fn listing_or_unbound(
    app: &mut App,
    press: &KeyPress,
    at: &Situation,
    tx: &mut Tx,
) -> Option<Outcome> {
    if at.within("preview")
        && let Some(outcome) = preview_keys::in_preview(app, press, at, tx)
    {
        return Some(outcome);
    }
    let mut in_view = at.in_view();
    if routes_typing_to_listing(in_view, at, press)
        && let Some(view) = shell::view_focus(app)
    {
        tx.focus(view, true);
        in_view = true;
    }
    if in_view && let Some(outcome) = listing_keys(app, press, tx) {
        return Some(outcome);
    }
    claims_unbound_command(press).then_some(Outcome::Ran)
}

fn routes_typing_to_listing(in_view: bool, at: &Situation, press: &KeyPress) -> bool {
    !in_view && !at.within("preview") && filter::is_typing_key(press)
}

/// 10xer's Escape ladder (`src/ui/window/keyboard/escape.rs`): dismisses one interaction per
/// press, and never closes a Miller column.
fn escape_ladder(app: &mut App, tx: &mut Tx) {
    if crate::peek::close(app) {
        return;
    }
    let dismissed = if filter::search_active(app) {
        escape_in_search(app, tx)
    } else {
        escape_in_listing(app, tx)
    };
    if !dismissed {
        crate::chooser::tenxer::cancel_on_escape(app, tx);
    }
}

fn escape_in_search(app: &mut App, tx: &mut Tx) -> bool {
    end_active_range(app)
        || find::dismiss_highlight(app)
        || preview::dismiss_escape(app, tx)
        || filter::dismiss_search_only(app, tx)
}

/// A range over `f` results lives on those results, so it ends before the filter that shows
/// them; a range over the listing ends after the find's highlight.
fn escape_in_listing(app: &mut App, tx: &mut Tx) -> bool {
    end_results_range(app)
        || filter::dismiss_filter_only(app, tx)
        || find::dismiss_highlight(app)
        || end_active_range(app)
        || preview::dismiss_escape(app, tx)
        || clear_selection(app)
}

/// A chooser's first row, selected as it loads, is not a selection to clear before Escape
/// cancels the request (`clear_selection` in `src/ui/window/keyboard/escape.rs`).
fn clear_selection(app: &App) -> bool {
    !crate::chooser::tenxer::keeps_load_cursor(app) && app.browser.clear_active_selection()
}

fn end_results_range(app: &App) -> bool {
    filter::results_shown(app) && end_active_range(app)
}

fn listing_keys(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    // `i` on a folder peeks it in Icons and List; on a file it is the preview's (A15's).
    if press.is(PREVIEW) && crate::peek::toggle_at_cursor(app) {
        return Some(Outcome::Ran);
    }
    let outcome = preview_keys::scroll(app, press, tx)
        .or_else(|| verbs::prompt_keys(app, press, tx))
        .or_else(|| file_keys(app, press, tx))
        .or_else(|| verbs::file_verbs(app, press, tx))
        .or_else(|| alt_navigation(app, press, tx));
    if outcome.is_some() {
        return outcome;
    }
    if shows(app, "icons") {
        icons_keys(app, press, tx)
    } else {
        rows_keys(app, press, tx)
    }
}

/// `tenxer_file_keys` (`src/ui/window/keyboard/files.rs`): file verbs and chord triggers on
/// the focused listing.
fn file_keys(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    if press.is(HIDDEN_FILES) {
        return Some(commands::run(app, "hidden", On::Focus, tx));
    }
    if press.is(BOOKMARK_SET) {
        arm_default_chord(app, ChordKind::BookmarkSet);
    } else if press.is(BOOKMARK_GO) {
        bookmarks::arm_go(app);
    } else if press.is(SORT_BY) {
        arm_default_chord(app, ChordKind::Sort);
    } else if press.is(ACTIONS) {
        chords::arm_actions(app, false);
    } else if press.is(COPY_AS) {
        arm_default_chord(app, ChordKind::Copy);
    } else if arms_tab_chord(app, press) {
        arm_default_chord(app, ChordKind::Tabs);
    } else {
        return None;
    }
    Some(Outcome::Ran)
}

/// `t` arms the tab chord outside a chooser, which has no tabs (`tenxer_file_keys`).
fn arms_tab_chord(app: &App, press: &KeyPress) -> bool {
    press.is(TAB_CHORD) && !app.chooser.is_active()
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
        arm_default_chord(app, ChordKind::Go);
        return Some(Outcome::Ran);
    }
    run_collection(app, press, true, tx)
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
        arm_default_chord(app, ChordKind::Go);
        return Some(Outcome::Ran);
    }
    run_collection(app, press, false, tx)
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
        Some(target) => preview::show_entry(app, target, app.browser.active_depth(), tx),
        None => app.tenxer.flash.show(tr("Nothing to preview"), tx),
    }
}

fn run_collection(app: &mut App, press: &KeyPress, icons: bool, tx: &mut Tx) -> Option<Outcome> {
    if swallowed_on_hits(app, press) {
        return Some(Outcome::Ran);
    }
    let command = non_native_step(press, icons)
        .map(|dir| Nav::Cursor(Target::Step(dir)))
        .or_else(|| mapped_collection_command(app, press))?;
    run_collection_command(app, command, tx).then_some(Outcome::Ran)
}

fn non_native_step(press: &KeyPress, icons: bool) -> Option<Direction> {
    if !press.used_fallback() && !press.aliased() {
        return None;
    }
    step_direction(press, icons)
}

fn swallowed_on_hits(app: &App, press: &KeyPress) -> bool {
    filter::search_active(app)
        && (press.is(FIRST_ITEM)
            || press.is(END_KEY)
            || press.is(PAGE_UP_KEY)
            || press.is(PAGE_DOWN_KEY))
}

fn mapped_collection_command(app: &App, press: &KeyPress) -> Option<Nav> {
    if press.is(SELECT_ALL) {
        return filter::targets::selected(app)
            .is_none()
            .then_some(Nav::SelectAll);
    }
    const MAPPED: [(key_catalog::Command, Nav); 12] = [
        (VISUAL_SELECT, Nav::Range(RangeKind::Select)),
        (VISUAL_UNSET, Nav::Range(RangeKind::Unset)),
        (FIRST_ITEM, Nav::Cursor(Target::First)),
        (FIRST_ITEM_UP, Nav::Cursor(Target::First)),
        (LAST_ITEM, Nav::Cursor(Target::Last)),
        (LAST_ITEM_DOWN, Nav::Cursor(Target::Last)),
        (PAGE_UP, Nav::Cursor(Target::Page(Vertical::Up))),
        (PAGE_DOWN, Nav::Cursor(Target::Page(Vertical::Down))),
        (HALF_PAGE_UP, Nav::Cursor(Target::HalfPage(Vertical::Up))),
        (
            HALF_PAGE_DOWN,
            Nav::Cursor(Target::HalfPage(Vertical::Down)),
        ),
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
    press.is(TAB_NEW)
        || [
            DUPLICATE,
            FILTER,
            SIDEBAR,
            RENAME,
            ARROW_SCOPE,
            JUMP_TO_FOLDER,
        ]
        .into_iter()
        .any(|command| {
            press.is(command.in_map(Map::Default)) && !press.is(command.in_map(Map::Tenxer))
        })
}
