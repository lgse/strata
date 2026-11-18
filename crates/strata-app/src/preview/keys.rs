// SPDX-License-Identifier: MIT

//! Keyboard commands and key routing for [`super::Preview`].
//!
//! Mirrors GTK's `src/ui/window/keyboard/commands.rs` and `src/ui/preview/keyboard.rs`:
//! - `preview-panel`: toggles the preview drawer and Columns' reserved slot (Appearance menu).
//! - `preview`: Space (`i` in 10xer) toggles preview on a file, enters a folder without preview
//!   in the listing, or closes an open preview when focus is inside the drawer (except in the
//!   find bar's field, which types Space).
//! - `preview-enter`: plain Right (`l` when type-to-search is off or in 10xer) on a file whose
//!   preview is showing moves focus into the preview's content (`Stage::PreviewNavigation`).
//! - `preview-divider`: `F8` moves focus to the preview divider (`Stage::FocusNavigation`).
//! - `find_keys`: C9's `PreviewDrawer::find_keys` predicate, using `FIND_IN_PREVIEW` (`Ctrl+F`
//!   in default mode, `/` in 10xer mode outside the find field).

use rune::{Key, KeyEvent, Tx};
use strata_components::contract::EntryId;
use strata_core::model::FileEntry;
use strata_desktop::keys::{
    Active, Binding, Key as DesktopKey, KeyPress, Map, Matcher, Modifiers,
    bindings::{ENTER, ESCAPE, ESCAPE_ANY_SHIFT, FOCUS_PREVIOUS, LEFT, PREVIEW, RIGHT, SPACE},
};

use super::{ContentFocus, archive, session};
use crate::{
    App,
    commands::{Action, Call, Keys, On, Outcome, Reads, Situation, Stage},
    filter, shell,
};

const DIVIDER_KEY: Binding = Binding::plain(DesktopKey::F8);
const PREVIEW_IGNORED: Modifiers = Modifiers::SHIFT_MASK.union(Modifiers::SUPER_MASK);
/// What 10xer's Shift+Tab out of the preview ignores (`leave_or_close_preview`).
const RETURN_IGNORED: Modifiers = Modifiers::CONTROL_MASK
    .union(Modifiers::ALT_MASK)
    .union(Modifiers::SUPER_MASK);
/// 10xer's Shift+Tab: the keys go back to the listing, the drawer stays.
pub(super) const TENXER_RETURN: Matcher = FOCUS_PREVIOUS.ignoring(RETURN_IGNORED);

pub(super) const COMMANDS: &[Action<App>] = &[
    super::pdf::RESET,
    super::transport::PLAY_PAUSE,
    super::transport::SEEK_BACK,
    super::transport::SEEK_FORWARD,
    super::transport::VOLUME_UP,
    super::transport::VOLUME_DOWN,
    super::transport::MUTE,
    super::transport::PREVIOUS,
    super::transport::NEXT,
    Action::new("preview-panel", toggle_panel),
    Action::new("preview-close", close_from_preview)
        .command(PREVIEW)
        .keys(&[
            Keys::at(Stage::DirectCommands, &[PREVIEW.in_map(Map::Tenxer)])
                .when(in_tenxer_preview_outside_find),
            // The divider grip, unlike the header's buttons and the body, takes no Space of its
            // own, so GTK's window toggles the preview closed.
            Keys::at(
                Stage::FilterAndLocationCommands,
                &[PREVIEW.in_map(Map::Default)],
            )
            .when(on_preview_grip),
            // GTK's archive list takes no Space either (A13): the window closes the preview.
            Keys::at(
                Stage::FilterAndLocationCommands,
                &[PREVIEW.in_map(Map::Default)],
            )
            .when(on_archive_tree),
        ]),
    Action::new("preview", space_preview)
        .command(PREVIEW)
        .keys(&[
            Keys::at(
                Stage::FilterAndLocationCommands,
                &[PREVIEW.in_map(Map::Default)],
            )
            .when(on_filtered_result),
            Keys::at(Stage::DirectCommands, &[PREVIEW.in_map(Map::Tenxer)])
                .when(in_tenxer_listing_for_preview),
            Keys::at(
                Stage::ItemNavigation,
                &[PREVIEW.in_map(Map::Default).ignoring(PREVIEW_IGNORED)],
            )
            .when(in_listing_for_preview),
        ]),
    // Read at the password field's last stage, so the field hands it over too.
    Action::new("preview-tenxer-return", return_to_files)
        .command(FOCUS_PREVIOUS)
        .keys(&[Keys::at(Stage::PreviewPasswordEscape, &[TENXER_RETURN]).when(in_tenxer_preview)]),
    Action::new("preview-password-escape", close_from_preview)
        .command(ESCAPE)
        .keys(
            &[Keys::at(Stage::PreviewPasswordEscape, &[ESCAPE_ANY_SHIFT])
                .when(password_prompt_focused)],
        ),
    Action::new("preview-open-archive", open_archive_in_columns).keys(&[Keys {
        stage: Stage::PreviewNavigation,
        reads: Reads::Press(opens_archive_key),
        when: opens_archive_in_columns,
    }]),
    Action::new("preview-enter", enter_preview)
        .command(RIGHT)
        .keys(&[Keys::at(
            Stage::PreviewNavigation,
            &[RIGHT.ignoring(Modifiers::empty())],
        )
        .when(can_enter_preview)]),
    Action::new("preview-leave", leave_preview)
        .command(LEFT)
        .keys(&[Keys::at(
            Stage::PreviewNavigation,
            &[LEFT.ignoring(Modifiers::empty())],
        )
        .when(in_preview_body_outside_find)]),
    Action::new("preview-divider", focus_divider).keys(&[Keys {
        stage: Stage::FocusNavigation,
        reads: Reads::Press(|press| press.is(DIVIDER_KEY)),
        when: divider_focusable,
    }]),
];

fn on_filtered_result(app: &App, at: &Situation) -> bool {
    at.in_view() && filter::targets::preview_result(app).is_some()
}

fn password_prompt_focused(app: &App, _: &Situation) -> bool {
    app.preview.enabled && app.preview.viewer.is_prompt_focused()
}

fn opens_archive_key(press: &KeyPress) -> bool {
    press.is(RIGHT) || press.is(ENTER) || press.is(SPACE)
}

/// GTK's 10xer listing claims Right, Enter and Space before this check (marking, activation,
/// `enter_preview`), so only the default map opens an archive from here.
fn opens_archive_in_columns(app: &App, at: &Situation) -> bool {
    at.in_view()
        && !shell::preferences(app).tenxer_mode
        && session::browsing_columns(app)
        && !filter::search_active(app)
        && app
            .browser
            .focused_entry()
            .is_some_and(|entry| archive::is_previewable_archive(&entry))
}

fn in_preview_outside_find(app: &App, at: &Situation) -> bool {
    app.preview.enabled
        && at.within("preview")
        && !app.preview.drawer.is_find_focused()
        && !app.preview.viewer.is_prompt_focused()
}

fn in_tenxer_preview(app: &App, at: &Situation) -> bool {
    let in_drawer = at.within("preview") || app.preview.viewer.is_prompt_focused();
    shell::preferences(app).tenxer_mode && app.preview.enabled && in_drawer
}

/// `return_from_preview`: the listing takes the keys, the drawer stays open.
fn return_to_files(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.preview.drawer.focus_files();
    Outcome::Ran
}

fn in_tenxer_preview_outside_find(app: &App, at: &Situation) -> bool {
    shell::preferences(app).tenxer_mode && in_preview_outside_find(app, at)
}

fn on_preview_grip(app: &App, at: &Situation) -> bool {
    app.preview.enabled && at.within("preview-grip")
}

/// The default map's Space there; 10xer's preview keeps it (`tenxer_preview` swallows what
/// the archive list does not take).
fn on_archive_tree(app: &App, at: &Situation) -> bool {
    app.preview.enabled
        && at.focused() == Some(archive::ARCHIVE_KEY)
        && !shell::preferences(app).tenxer_mode
}

fn in_preview_body_outside_find(app: &App, at: &Situation) -> bool {
    in_preview_outside_find(app, at)
        && !at.within("preview-grip")
        && app.preview.viewer.leaves_on_left()
}

fn in_tenxer_listing_for_preview(app: &App, at: &Situation) -> bool {
    shell::preferences(app).tenxer_mode && at.in_view()
}

fn in_listing_for_preview(_: &App, at: &Situation) -> bool {
    at.in_view()
}

/// Where the preview's media keys reach (`video_controls` in
/// `src/ui/window/keyboard/commands.rs`, with the drawer's `has_video`): an open drawer, not
/// set aside in a narrow window, and the keyboard in no text field, not in the sidebar and not
/// in the header. The route's and the media player's keys read it (`super::transport`).
pub(super) fn media_keys_reach(app: &App, at: &Situation) -> bool {
    let showing = app.preview.drawer.is_open() && !app.preview.drawer.is_suspended();
    showing && !at.text() && !at.within("sidebar") && !at.in_top_bar()
}

fn can_enter_preview(app: &App, at: &Situation) -> bool {
    if !at.in_view()
        || session::browsing_icons(app)
        || !app.preview.drawer.is_open()
        || app.preview.drawer.is_suspended()
    {
        return false;
    }
    if app.preview.viewer.is_empty() && !app.preview.loader.is_active() {
        return false;
    }
    let (_, entry) = session::selected_entry(app);
    entry.is_some_and(|entry| !entry.is_directory())
}

fn divider_focusable(app: &App, _: &Situation) -> bool {
    app.preview.drawer.is_open()
        && !app.preview.drawer.is_suspended()
        && (!app.preview.viewer.is_empty() || app.preview.reserves_empty_preview(app))
}

fn toggle_panel(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    shell::publish(app);
    let (depth, entry) = session::selected_entry(app);
    session::toggle_panel(app, super::preview_target(entry), depth, tx);
    Outcome::Ran
}

fn close_from_preview(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    session::close(app, tx);
    if let Some(view) = shell::view_focus(app) {
        tx.focus(view, true);
    }
    Outcome::Ran
}

fn space_preview(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    if let On::Row((depth, id)) = call.on {
        return preview_context_row(app, depth, id, tx);
    }
    if let Some(press) = call.press
        && is_plain_letter_with_type_to_search(app, press)
    {
        return Outcome::Declined;
    }
    let plain_space = call
        .press
        .is_none_or(|press| press.is(PREVIEW.in_map(Map::Default)));
    shell::publish(app);
    if let Some(target) = filter::targets::preview_result(app) {
        return preview_or_enter_filtered(app, target, plain_space, tx);
    }
    toggle_focused_from_space(app, plain_space, tx);
    Outcome::Ran
}

fn toggle_focused_from_space(app: &mut App, plain_space: bool, tx: &mut Tx) {
    let Some((depth, pos, entry)) = app.browser.focused_item() else {
        if app.preview.enabled {
            session::close(app, tx);
        }
        return;
    };
    if entry.is_directory() {
        if plain_space {
            enter_folder_from_space(app, depth, pos, tx);
        } else if app.preview.enabled {
            session::close(app, tx);
        }
        return;
    }
    toggle_file_from_space(app, (depth, entry), plain_space, tx);
}

fn toggle_file_from_space(
    app: &mut App,
    (depth, entry): (usize, FileEntry),
    plain_space: bool,
    tx: &mut Tx,
) {
    if plain_space && !app.preview.enabled {
        app.preview.loader.focus_archive_on_ready = true;
    }
    session::toggle(app, Some(entry), Some(depth), tx);
}

fn preview_context_row(app: &mut App, depth: usize, id: EntryId, tx: &mut Tx) -> Outcome {
    shell::publish(app);
    if let Some(entry) = filter::targets::at(app, depth, id) {
        app.browser.request_preview(entry);
    } else if let Some(view) = shell::shown_view(app)
        && let Some(position) = (view.rows.position)(app, (depth, id))
    {
        app.browser.preview(depth, position);
    }
    shell::settle(app, tx);
    Outcome::Ran
}

fn is_plain_letter_with_type_to_search(app: &App, press: &KeyPress) -> bool {
    shell::live_keys(app).type_to_search_active()
        && press
            .text()
            .is_some_and(|ch| !ch.is_whitespace() && !ch.is_control())
}

fn preview_or_enter_filtered(
    app: &mut App,
    (depth, entry): (usize, FileEntry),
    plain_space: bool,
    tx: &mut Tx,
) -> Outcome {
    if entry.is_directory() {
        if plain_space {
            app.browser.navigate(entry.location);
            shell::settle(app, tx);
        } else if app.preview.enabled {
            session::close(app, tx);
        }
    } else {
        toggle_file_from_space(app, (depth, entry), plain_space, tx);
    }
    Outcome::Ran
}

fn enter_folder_from_space(app: &mut App, depth: usize, position: usize, tx: &mut Tx) {
    if session::browsing_columns(app) {
        let _ = (depth, position);
        app.browser.activate_focused();
    } else {
        app.browser.activate_focused_in_place();
    }
    shell::settle(app, tx);
}

fn open_archive_in_columns(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    if let Some(press) = call.press
        && is_plain_letter_with_type_to_search(app, press)
    {
        return Outcome::Declined;
    }
    shell::publish(app);
    let Some((depth, _, entry)) = app.browser.focused_item() else {
        return Outcome::Declined;
    };
    enter_entry(app, entry, Some(depth), tx);
    Outcome::Ran
}

pub(super) fn enter_entry(app: &mut App, entry: FileEntry, depth: Option<usize>, tx: &mut Tx) {
    app.preview.loader.focus_archive_on_ready = true;
    session::show(app, entry, depth, tx);
    let _ = focus_loaded_or_pending(app, tx);
}

fn focus_loaded_or_pending(app: &mut App, tx: &mut Tx) -> bool {
    if app.preview.viewer.focus_into(tx) {
        return true;
    }
    if app.preview.viewer.can_focus() {
        app.preview.drawer.focus_content();
        return true;
    }
    if app.preview.viewer.needs_body_focus_owner() {
        app.preview.content_focus = ContentFocus::Pending;
        app.preview.drawer.focus_content_when_shown(tx);
        return true;
    }
    if let Some(request_id) = app.preview.loader.current_request {
        app.preview.loader.focus_archive_request = Some(request_id);
        app.preview.drawer.focus_content_when_shown(tx);
        return true;
    }
    false
}

fn enter_preview(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    if let Some(press) = call.press
        && is_plain_letter_with_type_to_search(app, press)
    {
        return Outcome::Declined;
    }
    if focus_loaded_or_pending(app, tx) {
        Outcome::Ran
    } else {
        Outcome::Declined
    }
}

fn focus_divider(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.preview.drawer.focus_divider();
    Outcome::Ran
}

fn leave_preview(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    if let Some(press) = call.press
        && is_plain_letter_with_type_to_search(app, press)
    {
        return Outcome::Declined;
    }
    app.preview.drawer.focus_files();
    Outcome::Ran
}

/// Key predicate passed to C9's `PreviewDrawer::find_keys`: `Ctrl+F` in the default map, or
/// plain `/` in 10xer mode when the find field is not already focused.
pub(super) fn find_keys_in(tenxer: bool) -> fn(&KeyEvent) -> bool {
    match tenxer {
        false => default_find_key,
        true => tenxer_find_key,
    }
}

fn default_find_key(ev: &KeyEvent) -> bool {
    ev.is_command('f') && !ev.mods.shift && !ev.mods.alt
}

fn tenxer_find_key(ev: &KeyEvent) -> bool {
    matches!(&ev.key, Key::Text(s) if s == "/") && !ev.mods.ctrl && !ev.mods.alt && !ev.mods.cmd
}
