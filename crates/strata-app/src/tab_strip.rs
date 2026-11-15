// SPDX-License-Identifier: MIT

//! The tab strip: C7's `BrowserTabs` across the window, over the shell's tabs
//! ([`shell::tabs`]), with the GTK window's tab commands (`TabWindow` in
//! `src/ui/window/composition/tabs.rs`).
//!
//! - **One strip for the window** ([`Scope::Window`]): its drag, its reveal and the closed
//!   tabs are the window's, as GTK's `TabStrip` and `closed_tabs` are.
//! - **It shows with two tabs or more,** and then holds New tab and the window's Close, which
//!   the header leaves out (`refresh_chrome`).
//! - **A tab is named after its folder** ([`tabs_of::label`]): `~` for home, `/` for the root,
//!   else the folder's name, and "Home" with none. A tab opened at a location is named after it
//!   until its folder opens (`await_location`).
//! - **Opening a folder in a new tab** (`open-in-tab`, `open.rs`): a place (a sidebar row, a
//!   crumb, a view's folder row on middle click), the folders a menu's row acts on, or
//!   Ctrl+Enter on the cursor's folder. The tab opens at the end and is shown, as GTK's
//!   `open_in_new_tab` adds it.
//! - **The number badges** ([`badges_shown`]): 1 to 9 and then 0 on the first ten tabs while
//!   [`TAB_NUMBERS`]'s modifiers are held alone, read from [`shell::held_modifiers`] so a
//!   modal layer does not hide them, and cleared when a key is released or the window loses
//!   focus.
//! - **Showing another tab ends an open rename as Enter does,** as GTK's field commits when
//!   the keyboard leaves it (driven on 2026-11-14). A Columns mirror waiting to open a child
//!   column opens it when the tab is shown again, where GTK's hidden tab opens it at once:
//!   the tab shows the same column either way (driven the same day).
//! - **Closing** records the tab's location with its place, twenty at most, newest last; the
//!   window's last tab closes the window, as its Close button does ([`shell::close_window`]).
//!   A tab whose browser has an operation running is not closed, and GTK's notice says so,
//!   "File operations are still active" (`has_background_operations`, `operations_active`).
//! - **Keys** are read at the GTK window's first stage ([`Stage::TabStrip`](crate::commands::Stage)),
//!   from anywhere but a modal layer: `keys.rs`.
//!
//! Not yet: drops on the strip and the spring (the tabs take no drop), and tab sessions. A4's
//! package file has the plan.

use std::path::Path;

use rune::{El, Tx};
use strata_components::tabs::{
    Accepts, BrowserTabs, BrowserTabsState, ClosedTabs, Tab, TabId, TabsData, TabsIntent,
    TabsLabels,
};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::keys::{
    Keys, Modifiers,
    bindings::{TAB_NEW, TAB_NUMBERS},
};
use strata_kit::icons::IconName;

use crate::{
    App, launch,
    location::labels::with_keys,
    rename,
    shell::{
        self,
        tabs::{self as tabs_of, located, shown_tab, tabs},
    },
    slots::{Frame, Part, Place, Scope, Setup, Strip, TabKey},
};

mod keys;
mod open;

/// The strip's state: C7's and the closed tabs.
pub(crate) struct TabStrip {
    state: BrowserTabsState,
    closed: ClosedTabs<Location>,
}

impl Part for TabStrip {
    const PLACE: Place<App> = Place::Tabs(Strip {
        build,
        shows: |app| tabs(app).len() > 1,
    });

    const SCOPE: Scope = Scope::Window;

    const COMMANDS: &'static [crate::commands::Action<App>] = keys::COMMANDS;

    fn new(_: &Setup<'_>) -> Self {
        Self {
            state: BrowserTabsState::default(),
            closed: ClosedTabs::new(),
        }
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        tabs_of::forget_arrived(app);
        let tabs = named(app);
        let data = TabsData::new(&tabs, Some(id(shown_tab(app))));
        let polled = app.tab_strip.state.tick(clock, &data, tx);
        let mut ran = polled.changed;
        for intent in polled.intents {
            ran |= route(app, intent, tx);
        }
        ran
    }
}

fn id(key: TabKey) -> TabId {
    TabId(key.0)
}

fn key(id: TabId) -> TabKey {
    TabKey(id.0)
}

/// The strip while it shows, with the window's Close after New tab.
fn build(app: &App, close: Option<El>, frame: &mut Frame<'_, App>) -> Option<El> {
    if tabs(app).len() < 2 {
        return None;
    }
    let tabs = named(app);
    let data = TabsData::new(&tabs, Some(id(shown_tab(app))));
    let labels = TabsLabels {
        list: tr("Tabs").into(),
        new_tab: tr("New tab").into(),
        new_tab_help: with_keys(
            "New tab (%{keys})",
            Some("New tab"),
            Keys::One(TAB_NEW.shown()),
        )
        .into(),
    };
    let strip = BrowserTabs::new(
        &data,
        &app.tab_strip.state,
        |app: &mut App| &mut app.tab_strip.state,
        |app| &mut app.tab_strip.state.drag,
    )
    .badges(badges_shown(app))
    .spring_delay(None)
    .labels(labels)
    .on_intent(|app: &mut App, intent, tx: &mut Tx| {
        route(app, intent, tx);
    });
    let band = strip.build_layered(&app.look, frame.cx, frame.layer);
    Some(band.children(close))
}

/// Whether the tab numbers' modifiers (`TAB_NUMBERS.modifiers()`) are held alone: GTK's
/// `handle_key` and `install_keys` in `src/ui/window/composition/tabs.rs`, read before
/// `blocked()` so a modal layer does not hide them.
fn badges_shown(app: &App) -> bool {
    let mods = shell::held_modifiers(app);
    let mut held = Modifiers::empty();
    held.set(Modifiers::SHIFT_MASK, mods.shift);
    held.set(Modifiers::CONTROL_MASK, mods.ctrl || mods.cmd);
    held.set(Modifiers::ALT_MASK, mods.alt);
    held.set(Modifiers::SUPER_MASK, mods.logo);
    held == TAB_NUMBERS.modifiers()
}

/// Each tab, in the strip's order, named as GTK names it. No tab takes a drop yet.
fn named(app: &App) -> Vec<Tab<'static>> {
    let home = Location::local(launch::home_directory());
    tabs(app)
        .iter()
        .map(|&key| {
            let location = located(app, key);
            let name = tabs_of::label(location.as_ref());
            let icon = icon(location.as_ref(), &home);
            Tab::new(id(key), name, icon).accepts(Accepts::NONE)
        })
        .collect()
}

fn icon(location: Option<&Location>, home: &Location) -> IconName {
    match location {
        None => IconName::Home,
        Some(location) if location == home => IconName::Home,
        Some(location) if location.native_path() == Some(Path::new("/")) => IconName::Drive,
        Some(location)
            if location
                .uri_value()
                .is_some_and(|uri| uri.starts_with("trash://")) =>
        {
            IconName::Trash
        }
        Some(location) if location.uri_value().is_some() => IconName::Network,
        Some(_) => IconName::Folder,
    }
}

/// What the strip asked. True when the view has to run.
fn route(app: &mut App, intent: TabsIntent, tx: &mut Tx) -> bool {
    match intent {
        TabsIntent::Select { id, .. } => select(app, key(id), tx),
        TabsIntent::Close { id } => close(app, key(id), tx),
        TabsIntent::New => {
            new_tab(app, tx);
            true
        }
        TabsIntent::Reorder { id, to, .. } => tabs_of::move_to(app, key(id), to),
        // The shell titles the window for the tab shown.
        TabsIntent::Title(_) => false,
        TabsIntent::TearOut { .. }
        | TabsIntent::TearMove { .. }
        | TabsIntent::DropInto { .. }
        | TabsIntent::OpenTabs { .. }
        | TabsIntent::DropHover { .. } => false,
    }
}

/// What the tab shown ends before another is shown in its place, beyond what its parts end
/// themselves: an open rename commits, as GTK's field does when the keyboard leaves it.
fn leaving(app: &mut App, tx: &mut Tx) {
    rename::commit(app, tx);
}

/// Shows tab `key`. False when it is shown already.
pub(crate) fn select(app: &mut App, key: TabKey, tx: &mut Tx) -> bool {
    if key == shown_tab(app) || !tabs(app).contains(&key) {
        return false;
    }
    leaving(app, tx);
    tabs_of::show(app, key, tx)
}

/// Opens a tab at the end at the shown tab's folder, or at home with none (`new_tab`).
pub(crate) fn new_tab(app: &mut App, tx: &mut Tx) {
    let location = app.browser.active_location();
    let location = location.or_else(|| launch::location(None));
    open_at(app, location, tabs(app).len(), tx);
}

/// Opens `location` in a tab at the end, shown (`open_in_new_tab`).
pub(crate) fn open_in_new_tab(app: &mut App, location: Location, tx: &mut Tx) {
    open_at(app, Some(location), tabs(app).len(), tx);
}

fn open_at(app: &mut App, location: Option<Location>, at: usize, tx: &mut Tx) {
    leaving(app, tx);
    tabs_of::open(app, location, at, tx);
}

/// Closes tab `key` (`close`): not while its browser has an operation running, which GTK's
/// notice says; the window's last closes the window; any other is recorded, where it stood,
/// for a reopen.
pub(crate) fn close(app: &mut App, key: TabKey, tx: &mut Tx) -> bool {
    let Some(at) = tabs(app).iter().position(|tab| *tab == key) else {
        return false;
    };
    if tabs_of::is_busy(app, key) {
        tabs_of::operations_active(app);
        return false;
    }
    if tabs(app).len() == 1 {
        shell::close_window(app, tx);
        return false;
    }
    let location = located(app, key);
    if key == shown_tab(app) {
        leaving(app, tx);
    }
    if !tabs_of::close(app, key, tx) {
        return false;
    }
    if let Some(location) = location {
        app.tab_strip.closed.record(location, at);
    }
    true
}

/// Reopens the last closed tab where it stood, or at the end when fewer tabs are open
/// (`reopen_closed_tab`). Nothing with none recorded.
pub(crate) fn reopen(app: &mut App, tx: &mut Tx) {
    let Some(closed) = app.tab_strip.closed.reopen() else {
        return;
    };
    let at = closed.index_in(tabs(app).len());
    open_at(app, Some(closed.item), at, tx);
}

#[cfg(test)]
mod tests;
