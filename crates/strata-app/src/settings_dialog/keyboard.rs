// SPDX-License-Identifier: MIT

//! Settings → Keyboard: each map's table of commands and their keys, GTK's
//! `src/ui/settings/keyboard.rs`. The page only edits the saved keymap overrides and
//! type-to-search, through `shell::saved(app).change`; dispatch and labels follow the saved
//! settings at the next press (`shell::live_keys`). See `docs/keyboard-shortcuts.md`.

use std::rc::Rc;

use rune::{El, TextEdit, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::controls::settings_search::SettingsSearch;
use strata_core::i18n::tr;
use strata_desktop::keys::{Active, Command, Keymap, Map, conflicts::Policy};
use strata_fs::services::ActionCatalog;

use super::{
    controls::{SearchField, group_card, search_field, segmented_toggles},
    search_index::KEYBOARD_SHORTCUTS,
};
use crate::{App, buttons, shell};

mod capture;
mod messages;
mod names;
pub(super) mod reset_all;
mod rows;
mod sections;
#[cfg(test)]
mod tests;

pub(super) use reset_all::overlay as reset_all_overlay;

const SEARCH_KEY: &str = "settings-keymap-search";
const RESET_ALL_KEY: &str = "settings-keymap-reset-all";
const MAPS_KEY: &str = "settings-keymap-maps";

/// The page's own state: the table shown, the keybind search, the armed capture and the
/// open Reset all confirmation.
pub(crate) struct KeyboardState {
    shown: Map,
    pub(super) query: TextEdit,
    capture: Option<capture::Capturing>,
    confirm: Option<Map>,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self {
            shown: Map::Default,
            query: TextEdit::default(),
            capture: None,
            confirm: None,
        }
    }
}

impl KeyboardState {
    /// The state Settings opens with: the table of the mode in force.
    pub(super) fn opened(tenxer_mode: bool) -> Self {
        Self {
            shown: if tenxer_mode {
                Map::Tenxer
            } else {
                Map::Default
            },
            ..Self::default()
        }
    }

    pub(super) fn confirm_open(&self) -> bool {
        self.confirm.is_some()
    }

    /// The capture ends when its row goes, as GTK ends it when the button unmaps.
    pub(super) fn end_capture(&mut self) {
        self.capture = None;
    }
}

type Section = (&'static str, Vec<Command>);

/// What every row of the shown table reads.
pub(super) struct Page<'a> {
    pub(super) app: &'a App,
    pub(super) keymap: Rc<Keymap>,
    pub(super) policy: Policy,
    pub(super) catalog: Option<Rc<ActionCatalog>>,
    query: String,
}

impl<'a> Page<'a> {
    pub(super) fn of(app: &'a App) -> Self {
        Self {
            app,
            keymap: shell::live_keys(app).keymap(),
            policy: policy(app),
            catalog: app.custom_actions.catalog(),
            query: app
                .settings_dialog
                .keyboard
                .query
                .text
                .trim()
                .to_lowercase(),
        }
    }

    fn matches(&self, command: Command) -> bool {
        names::search_names(command, self.catalog.as_deref()).contains(&self.query)
    }

    /// The sections and the rows the keybind search leaves under each.
    fn filtered(&self, sections: &[Section]) -> Vec<Section> {
        sections
            .iter()
            .map(|(title, commands)| {
                let shown = commands.iter().copied().filter(|c| self.matches(*c));
                (*title, shown.collect::<Vec<_>>())
            })
            .filter(|(_, commands)| !commands.is_empty())
            .collect()
    }
}

pub(super) fn policy(app: &App) -> Policy {
    Policy {
        type_to_search: shell::preferences(app).type_to_search,
    }
}

pub(super) fn build(app: &App, search: &SettingsSearch<App>) -> Vec<El> {
    if !search.shows_row(app, KEYBOARD_SHORTCUTS) {
        return Vec::new();
    }
    let page = Page::of(app);
    let shown = app.settings_dialog.keyboard.shown;
    let all = sections::sections(shown);
    let tables = page.filtered(&all);
    let overridden = all
        .iter()
        .flat_map(|(_, commands)| commands)
        .any(|command| page.keymap.overrides(shown, *command));
    let mut content = vec![controls(&page, tables.is_empty(), overridden)];
    let findings = rows::audit(&page, shown);
    for (title, commands) in tables {
        content.push(heading(app, title));
        let rows = commands
            .into_iter()
            .map(|command| {
                let found = findings.get(&command).map_or(&[][..], Vec::as_slice);
                Some(rows::command_row(&page, shown, command, found))
            })
            .collect();
        content.extend(group_card(&app.look, rows));
    }
    content
}

fn heading(app: &App, title: &str) -> El {
    let look = &app.look;
    text(tr(title))
        .font(look.ty.heading)
        .color(look.kit.c.text_muted)
        .pt(look.space.md)
        .pb(look.space.xs)
}

/// The map switcher and Reset all, then the keybind search, and the note when it matches
/// nothing in the shown table.
fn controls(page: &Page<'_>, no_match: bool, overridden: bool) -> El {
    let app = page.app;
    let look = &app.look;
    let toolbar = row()
        .w_full()
        .gap(look.space.sm)
        .items_center()
        .child(map_switcher(app))
        .child(col().grow(1.0))
        .child(reset_all_button(app, overridden));
    let mut controls = col()
        .w_full()
        .gap(look.space.md)
        .pb(look.space.lg)
        .child(toolbar)
        .child(keybind_search(app));
    if no_match {
        controls = controls.child(
            text(tr("No matching shortcuts"))
                .font(look.kit.ty.small)
                .color(look.kit.c.text_muted),
        );
    }
    controls
}

fn map_switcher(app: &App) -> El {
    let shown = app.settings_dialog.keyboard.shown;
    let choice = |label: &str, map: Map| -> super::controls::SegmentChoice {
        (
            tr(label),
            shown == map,
            Rc::new(move |app: &mut App, _: &mut Tx| show(app, map)),
        )
    };
    segmented_toggles(
        &app.look,
        MAPS_KEY,
        None,
        vec![
            choice("Default", Map::Default),
            choice("10xer mode", Map::Tenxer),
        ],
    )
}

fn reset_all_button(app: &App, overridden: bool) -> El {
    let button = Button::new(tr("Reset all"))
        .variant(ButtonVariant::Neutral)
        .disabled(!overridden)
        .on_press_tx(|app: &mut App, tx: &mut Tx| reset_all::ask(app, tx))
        .build(&app.look.kit)
        .description(tr(
            "Restore the shipped keys of every action in the shown mode.",
        ));
    buttons::on_return(RESET_ALL_KEY, button)
}

fn keybind_search(app: &App) -> El {
    search_field(
        app,
        SearchField {
            key: SEARCH_KEY,
            clear_key: "settings-keymap-search-clear",
            label: tr("Search keybinds"),
            edit: |app| &app.settings_dialog.keyboard.query,
            edit_mut: |app| &mut app.settings_dialog.keyboard.query,
            width: None,
            on_change: follow_search,
        },
    )
}

/// Shows `map`'s table, ending any capture.
fn show(app: &mut App, map: Map) {
    let state = &mut app.settings_dialog.keyboard;
    state.end_capture();
    state.shown = map;
}

/// Shows the other table when the search leaves no row in the shown one but matches some
/// there, at each change of the query: GTK's `follow_search`.
fn follow_search(app: &mut App) {
    let shown = app.settings_dialog.keyboard.shown;
    let other = match shown {
        Map::Default => Map::Tenxer,
        Map::Tenxer => Map::Default,
    };
    let follow = {
        let page = Page::of(app);
        let rows_in = |map| !page.filtered(&sections::sections(map)).is_empty();
        !rows_in(shown) && rows_in(other)
    };
    if follow {
        show(app, other);
    }
}

/// The page's turn: a capture whose button lost the keyboard ends.
pub(super) fn poll(app: &mut App, tx: &mut Tx) -> bool {
    capture::follow_focus(app, tx)
}
