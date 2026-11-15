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
use strata_components::{contract::EntryId, preview_drawer::HostedPreview};
use strata_core::model::FileEntry;
use strata_desktop::keys::{
    Active, Binding, Key as DesktopKey, KeyPress, Map, Modifiers,
    bindings::{PREVIEW, RIGHT},
};

use super::session;
use crate::{
    App,
    commands::{Action, Call, Keys, On, Outcome, Reads, Situation, Stage},
    filter, shell,
};

const DIVIDER_KEY: Binding = Binding::plain(DesktopKey::F8);
const PREVIEW_IGNORED: Modifiers = Modifiers::SHIFT_MASK.union(Modifiers::SUPER_MASK);

pub(super) const COMMANDS: &[Action<App>] = &[
    Action::new("preview-panel", toggle_panel),
    Action::new("preview-close", close_from_preview).keys(&[Keys::at(
        Stage::FilterAndLocationCommands,
        &[PREVIEW.ignoring(Modifiers::empty())],
    )
    .when(in_preview_outside_find)]),
    Action::new("preview", space_preview)
        .command(PREVIEW)
        .keys(&[Keys::at(
            Stage::ItemNavigation,
            &[
                PREVIEW.in_map(Map::Default).ignoring(PREVIEW_IGNORED),
                PREVIEW.in_map(Map::Tenxer),
            ],
        )
        .when(in_listing_for_preview)]),
    Action::new("preview-enter", enter_preview)
        .command(RIGHT)
        .keys(&[Keys::at(
            Stage::PreviewNavigation,
            &[RIGHT.ignoring(Modifiers::empty())],
        )
        .when(can_enter_preview)]),
    Action::new("preview-divider", focus_divider).keys(&[Keys {
        stage: Stage::FocusNavigation,
        reads: Reads::Press(|press| press.is(DIVIDER_KEY)),
        when: divider_focusable,
    }]),
];

fn in_preview_outside_find(app: &App, at: &Situation) -> bool {
    app.preview.enabled && at.within("preview") && !app.preview.drawer.is_find_focused()
}

fn in_listing_for_preview(app: &App, at: &Situation) -> bool {
    if !at.in_view() {
        return false;
    }
    // In default mode with type-to-search on, `PREVIEW` is Space (not a letter); in 10xer mode
    // `PREVIEW` is `i` and type-to-search is inactive. Guard against plain-letter bindings when
    // type-to-search is active, just like `navigation_keys_active`.
    let _ = app;
    true
}

fn can_enter_preview(app: &App, at: &Situation) -> bool {
    if !at.in_view()
        || session::browsing_icons(app)
        || !app.preview.drawer.is_open()
        || app.preview.drawer.is_suspended()
    {
        return false;
    }
    if app.preview.viewer.is_empty() {
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
    let Some((depth, pos, entry)) = app.browser.focused_item() else {
        if app.preview.enabled {
            session::close(app, tx);
        }
        return Outcome::Ran;
    };
    if entry.is_directory() {
        if plain_space {
            enter_folder_from_space(app, depth, pos, tx);
        } else if app.preview.enabled {
            session::close(app, tx);
        }
        return Outcome::Ran;
    }
    session::toggle(app, Some(entry), Some(depth), tx);
    Outcome::Ran
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
        session::toggle(app, Some(entry), Some(depth), tx);
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

fn enter_preview(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    if let Some(press) = call.press
        && is_plain_letter_with_type_to_search(app, press)
    {
        return Outcome::Declined;
    }
    if !app.preview.viewer.searchable() && app.preview.viewer.is_empty() {
        return Outcome::Declined;
    }
    app.preview.drawer.focus_content();
    Outcome::Ran
}

fn focus_divider(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.preview.drawer.focus_divider();
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
