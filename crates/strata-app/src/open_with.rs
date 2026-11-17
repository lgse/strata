// SPDX-License-Identifier: MIT

//! Opening files with desktop applications and the Open With chooser (`src/ui/open_with/` and
//! `src/ui/browser/desktop.rs`), built on C15's [`StrataDialog`](strata_components::dialog::StrataDialog).
//!
//! - **`BrowserEvent::OpenRequested`** ([`launch::open_location_at`]): refuses Trash and
//!   administrator locations with `"Unable to open item"`, launches `.desktop` web links via
//!   `gio::AppInfo::launch_default_for_uri_future`, launches registered default handlers via
//!   [`launch::launch_at`], hands local executable files with no registered default to
//!   [`crate::run::confirm`], and falls back to the Open With chooser otherwise.
//! - **Context menus** ([`menu::prepare_menu`], [`menu::open_selection_default`]): starts
//!   `"Open With…"` insensitive with `"Looking for compatible applications…"`, fills common
//!   handlers off the UI thread, reveals `"Open"` on multi-selections when a shared default
//!   exists, and loads folder recent-app history on background menus.
//! - **The chooser** ([`view`]): search field with live filtering, `Up`/`Down` selection
//!   stepping from search without moving the caret, `Escape` clearing a non-empty search before
//!   dismissing, Tab order (`Search` -> selected row -> `Always use…` -> `Cancel` -> `Open` ->
//!   `Close dialog`), `"Recently Used"` / `"Recommended Applications"` / `"Other Applications"`
//!   sections, default association saving (`gio::AppInfo::set_as_default_for_type`), and error
//!   reporting (`"Unable to open file"`).

mod apps;
pub(crate) mod launch;
mod menu;
mod recent;
mod view;

#[cfg(test)]
mod tests;

use std::{cell::Cell, rc::Rc};

use gio::prelude::*;
use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::i18n::tr;
use strata_desktop::keys::{Map, bindings::OPEN_WITH};
use strata_fs::app::BrowserEvent;

use crate::{
    App,
    commands::{Action, Holds, Keys, Stage},
    error_dialog::{self, Error},
    shell,
    slots::{Later, Listener, Part, Place, Setup},
};

#[cfg(test)]
pub(crate) use launch::open_location_at;
pub(crate) use menu::{
    RefusalReport, cancel_target_lookup, open_selection_default, open_targets, prepare_menu,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenWithContext {
    Explicit,
    ActivationFallback,
}

impl OpenWithContext {
    pub(super) fn empty_message(self) -> &'static str {
        match self {
            Self::Explicit => "No compatible applications were found.",
            Self::ActivationFallback => "No application is registered for this file",
        }
    }
}

pub(super) struct ChooserSpec {
    pub(super) files: Vec<gio::File>,
    pub(super) content_types: Vec<String>,
    pub(super) choices: apps::Choices,
    pub(super) context: OpenWithContext,
}

pub(crate) struct OpenWith {
    later: Later<App>,
    form: Option<view::Form>,
    open: bool,
    opened: u64,
    menu_generation: Rc<Cell<u64>>,
    menu_selection: Option<menu::MenuSelection>,
    folder_selection: Option<menu::FolderSelection>,
    target_generation: Rc<Cell<u64>>,
}

impl OpenWith {
    fn bump_menu_generation(&self) -> u64 {
        let next = self.menu_generation.get().wrapping_add(1);
        self.menu_generation.set(next);
        next
    }

    fn bump_target_generation(&self) -> u64 {
        let next = self.target_generation.get().wrapping_add(1);
        self.target_generation.set(next);
        next
    }

    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }
}

impl Part for OpenWith {
    const PLACE: Place<App> = Place::Dialogs(view::build);

    fn new(setup: &Setup<'_>) -> Self {
        {
            let _startup = strata_core::util::ui_thread::allow(
                strata_core::util::ui_thread::Exception::Startup,
            );
            let _ = apps::uses_portal();
        }
        Self {
            later: setup.later.clone(),
            form: None,
            open: false,
            opened: 0,
            menu_generation: Rc::new(Cell::new(0)),
            menu_selection: None,
            folder_selection: None,
            target_generation: Rc::new(Cell::new(0)),
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, browser, heard| {
            if browser.is_chooser_mode() {
                return;
            }
            if let BrowserEvent::OpenRequested { location } = event {
                let location = location.clone();
                heard.then(move |app: &mut App, tx: &mut Tx| {
                    launch::open_location_at(app, &location, None);
                    shell::settle(app, tx);
                    true
                });
            }
        }))
    }

    fn keyboard(app: &App) -> Option<Holds> {
        let open_with = &app.open_with;
        let form = open_with.form.as_ref().filter(|_| open_with.open)?;
        Some(Holds::Modal {
            field: form.field_focused(),
        })
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("open-with", menu::command)
        .command(OPEN_WITH)
        .keys(&[
            Keys::at(Stage::DirectCommands, &[OPEN_WITH.in_map(Map::Default)])
                .when(|app, at| !shell::preferences(app).tenxer_mode && !at.text() && at.in_view()),
            Keys::at(Stage::FileCommands, &[OPEN_WITH.in_map(Map::Tenxer)])
                .when(|app, at| shell::preferences(app).tenxer_mode && !at.text() && at.in_view()),
        ])];
}

pub(super) fn foreign_modal_open(app: &App) -> bool {
    shell::parts(app).active().any(|part| {
        part.name != "open_with" && matches!((part.keyboard)(app), Some(Holds::Modal { .. }))
    })
}

pub(super) fn show_chooser(app: &mut App, spec: ChooserSpec) {
    if apps::uses_portal() {
        launch::launch_with_portal(spec.files, app.open_with.later.clone());
        return;
    }
    let open_with = &mut app.open_with;
    open_with.opened += 1;
    let generation = open_with.opened;
    open_with.form = Some(view::Form::new(generation, spec));
    open_with.open = true;
}

fn cancel(app: &mut App) {
    app.open_with.open = false;
}

fn dismiss_by_escape_not_close(tx: &Tx, close_key: &str) -> bool {
    tx.by_keys() && tx.focused() != Some(close_key)
}

fn clear_search_and_focus(form: &mut view::Form, tx: &mut Tx) {
    let generation = form.generation;
    form.search.replace_text(String::new());
    form.search.focused = true;
    form.sync_selection();
    tx.focus(view::search_key(generation), true);
}

fn non_empty_search_form(open_with: &mut OpenWith) -> Option<&mut view::Form> {
    open_with
        .form
        .as_mut()
        .filter(|form| !form.search.text.is_empty())
}

fn dismiss_or_clear_search(app: &mut App, tx: &mut Tx, close_key: &str) {
    if dismiss_by_escape_not_close(tx, close_key)
        && let Some(form) = non_empty_search_form(&mut app.open_with)
    {
        clear_search_and_focus(form, tx);
        return;
    }
    app.open_with.open = false;
}

fn toggle_always_use(app: &mut App) {
    if let Some(form) = app.open_with.form.as_mut() {
        form.always_use = !form.always_use;
    }
}

fn search_changed(app: &mut App) {
    if let Some(form) = app.open_with.form.as_mut() {
        form.sync_selection();
    }
}

fn search_key_pressed(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(form) = app.open_with.form.as_mut() else {
        return false;
    };
    if form.search.is_composing() {
        return false;
    }
    if ev.key == Key::Escape {
        return handle_search_escape(form, tx);
    }
    if ev.mods == Mods::default() && matches!(ev.key, Key::Down | Key::Up) {
        return step_from_search(form, ev.key == Key::Down, tx);
    }
    false
}

fn handle_search_escape(form: &mut view::Form, tx: &Tx) -> bool {
    if tx.key_repeat() {
        return true;
    }
    if form.search.text.is_empty() {
        return false;
    }
    form.search.replace_text(String::new());
    form.sync_selection();
    true
}

fn step_from_search(form: &mut view::Form, down: bool, tx: &mut Tx) -> bool {
    let generation = form.generation;
    if let Some(selected) = form.step_selection(down) {
        tx.scroll_into_view(view::row_key(generation, selected));
    }
    true
}

fn select_row(app: &mut App, index: usize, tx: &mut Tx) {
    if let Some(form) = app.open_with.form.as_mut() {
        let generation = form.generation;
        form.selected = Some(index);
        form.search.focused = false;
        tx.focus_when_shown(view::row_key(generation, index), false);
    }
}

fn activate_row(app: &mut App, index: usize) {
    if let Some(form) = app.open_with.form.as_mut() {
        form.selected = Some(index);
    }
    confirm_selected(app);
}

fn list_key_pressed(app: &mut App, index: usize, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let plain = ev.mods == Mods::default();
    if ev.key == Key::Enter && plain {
        if !tx.key_repeat() {
            submit(app, tx);
        }
        return true;
    }
    if ev.key == Key::Escape {
        return handle_list_escape(app, tx);
    }
    if plain && matches!(ev.key, Key::Up | Key::Down) {
        return step_focused_row(app, index, ev.key == Key::Down, tx);
    }
    redirect_list_typing(app, ev, tx)
}

fn handle_list_escape(app: &mut App, tx: &mut Tx) -> bool {
    if tx.key_repeat() {
        return true;
    }
    let Some(form) = app.open_with.form.as_mut() else {
        return false;
    };
    if form.search.text.is_empty() {
        return false;
    }
    clear_search_and_focus(form, tx);
    true
}

fn step_focused_row(app: &mut App, index: usize, down: bool, tx: &mut Tx) -> bool {
    let Some(form) = app.open_with.form.as_mut() else {
        return false;
    };
    let generation = form.generation;
    let visible = form.filtered_indices();
    let Some(pos) = visible.iter().position(|&idx| idx == index) else {
        return false;
    };
    if !down && pos == 0 {
        form.search.focused = true;
        tx.focus(view::search_key(generation), true);
        return true;
    }
    if let Some(next) = form.step_selection(down)
        && next != index
    {
        let key = view::row_key(generation, next);
        tx.focus_when_shown(key.clone(), true);
        tx.scroll_into_view(key);
    }
    true
}

fn has_command_modifier(mods: Mods) -> bool {
    [mods.ctrl, mods.cmd, mods.alt, mods.logo]
        .into_iter()
        .any(|held| held)
}

fn redirect_list_typing(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if has_command_modifier(ev.mods) {
        return false;
    }
    let Some(form) = app.open_with.form.as_mut() else {
        return false;
    };
    let generation = form.generation;
    match &ev.key {
        Key::Backspace => {
            let mut next = form.search.text.clone();
            next.pop();
            form.search.replace_text(next);
        }
        Key::Text(ch) if !ch.chars().any(char::is_control) => {
            let mut next = form.search.text.clone();
            next.push_str(ch);
            form.search.replace_text(next);
        }
        _ => return false,
    }
    form.search.focused = true;
    form.sync_selection();
    tx.focus(view::search_key(generation), true);
    true
}

fn submit(app: &mut App, _: &mut Tx) {
    confirm_selected(app);
}

fn confirm_selected(app: &mut App) {
    if !app.open_with.open {
        return;
    }
    let Some(form) = app.open_with.form.as_ref() else {
        return;
    };
    let Some(selected) = form.selected.and_then(|idx| form.entries.get(idx)) else {
        return;
    };
    let chosen = selected.app.clone();
    let files = form.files.clone();
    let content_types = form.content_types.clone();
    let always_use = form.always_use;
    if always_use {
        set_as_default(&chosen, &content_types);
    }
    let context = launch::launch_context();
    let launched = launch::launch(
        &chosen,
        launch::LaunchBatch {
            files: &files,
            content_types: &content_types,
        },
        Some(&context),
    );
    app.open_with.open = false;
    if let Err(error) = launched {
        error_dialog::show(
            app,
            Error::new(tr("Unable to open file"), error.message().to_owned()),
        );
    }
}

fn set_as_default(app: &gio::AppInfo, content_types: &[String]) {
    for content_type in content_types {
        if let Err(error) = app.set_as_default_for_type(content_type) {
            tracing::warn!(%content_type, %error, "unable to set default application");
        }
    }
}
