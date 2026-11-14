// SPDX-License-Identifier: MIT

//! The tab strip: C7's `BrowserTabs` across the window, over the shell's tabs
//! ([`shell::tabs`]), with the GTK window's tab commands (`TabWindow` in
//! `src/ui/window/composition/tabs.rs`).
//!
//! - **One strip for the window** ([`Scope::Window`]): its drag, its reveal and the closed
//!   tabs are the window's, as GTK's `TabStrip` and `closed_tabs` are.
//! - **It shows with two tabs or more,** and then holds New tab and the window's Close, which
//!   the header leaves out (`refresh_chrome`).
//! - **A tab is named after its folder** (`tab_label`): `~` for home, `/` for the root, else
//!   the folder's name, and "Home" with none. A reopened tab is named after the location it
//!   waits on until its folder opens (`await_location`).
//! - **Showing another tab ends an open rename as Enter does,** as GTK's field commits when
//!   the keyboard leaves it (driven on 2026-11-14). A Columns mirror waiting to open a child
//!   column opens it when the tab is shown again, where GTK's hidden tab opens it at once:
//!   the tab shows the same column either way (driven the same day).
//! - **Closing** records the tab's location with its place, twenty at most, newest last; the
//!   window's last tab closes the window. A tab whose browser has an operation running is
//!   not closed (`has_background_operations`); GTK's notice for it is not drawn yet.
//! - **Keys** are read at the GTK window's first stage ([`Stage::TabStrip`](crate::commands::Stage)),
//!   from anywhere but a modal layer: `keys.rs`.
//!
//! Not yet: the number badges, drops on the strip and the spring (the tabs take no drop),
//! opening a folder in a new tab, and tab sessions. A4's package file has the plan.

use std::path::Path;

use rune::{El, Tx};
use strata_components::tabs::{
    Accepts, BrowserTabs, BrowserTabsState, ClosedTabs, Tab, TabId, TabsData, TabsIntent,
    TabsLabels,
};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::keys::{Keys, bindings::TAB_NEW};
use strata_kit::icons::IconName;

use crate::{
    App, launch,
    location::labels::with_keys,
    rename,
    shell::{
        self,
        tabs::{self as tabs_of, shown_tab, tab_browser, tabs},
    },
    slots::{Frame, Part, Place, Scope, Setup, Strip, TabKey},
};

mod keys;

/// The strip's state: C7's, the closed tabs, and the locations reopened tabs wait on.
pub(crate) struct TabStrip {
    state: BrowserTabsState,
    closed: ClosedTabs<Location>,
    /// A tab opened at a location that has not opened yet, named after it meanwhile.
    awaited: Vec<(TabKey, Location)>,
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
            awaited: Vec::new(),
        }
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        forget_arrived(app);
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
        new_tab_help: with_keys("New tab (%{keys})", Some("New tab"), Keys::One(TAB_NEW.shown()))
            .into(),
    };
    let strip = BrowserTabs::new(
        &data,
        &app.tab_strip.state,
        |app: &mut App| &mut app.tab_strip.state,
        |app| &mut app.tab_strip.state.drag,
    )
    .spring_delay(None)
    .labels(labels)
    .on_intent(|app: &mut App, intent, tx: &mut Tx| {
        route(app, intent, tx);
    });
    let band = strip.build_layered(&app.look, frame.cx, frame.layer);
    Some(band.children(close))
}

/// Each tab, in the strip's order, named as GTK names it. No tab takes a drop yet.
fn named(app: &App) -> Vec<Tab<'static>> {
    let home = Location::local(launch::home_directory());
    tabs(app)
        .iter()
        .map(|&key| {
            let location = located(app, key);
            let (name, icon) = (tab_label(location.as_ref()), icon(location.as_ref(), &home));
            Tab::new(id(key), name, icon).accepts(Accepts::NONE)
        })
        .collect()
}

/// Where tab `key` is, or the location it waits on (GTK's `Tab::location`).
fn located(app: &App, key: TabKey) -> Option<Location> {
    let browser = tab_browser(app, key)?;
    browser.active_location().or_else(|| awaited(app, key).cloned())
}

fn awaited(app: &App, key: TabKey) -> Option<&Location> {
    let mut awaited = app.tab_strip.awaited.iter();
    awaited
        .find(|(waiting, _)| *waiting == key)
        .map(|(_, location)| location)
}

/// A tab whose folder opened waits on nothing: it is named after its folder from now on.
fn forget_arrived(app: &mut App) {
    let arrived: Vec<TabKey> = app
        .tab_strip
        .awaited
        .iter()
        .filter(|(key, _)| {
            tab_browser(app, *key).is_none_or(|browser| browser.active_location().is_some())
        })
        .map(|(key, _)| *key)
        .collect();
    app.tab_strip
        .awaited
        .retain(|(key, _)| !arrived.contains(key));
}

/// GTK's `tab_label`: `~` for home, `/` for the root, a folder's name, and "Home" with none.
fn tab_label(location: Option<&Location>) -> String {
    let Some(location) = location else {
        return tr("Home");
    };
    let Some(path) = location.native_path() else {
        return location.display_name();
    };
    if Some(path) == std::env::var_os("HOME").as_deref().map(Path::new) {
        return "~".into();
    }
    path.file_name()
        .map_or_else(|| "/".into(), |name| name.to_string_lossy().into_owned())
}

fn icon(location: Option<&Location>, home: &Location) -> IconName {
    match location {
        None => IconName::Home,
        Some(location) if location == home => IconName::Home,
        Some(location) if location.native_path() == Some(Path::new("/")) => IconName::Drive,
        Some(location) if location.uri_value().is_some_and(|uri| uri.starts_with("trash://")) => {
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
    let end = tabs(app).len();
    leaving(app, tx);
    tabs_of::open(app, location, end, tx);
}

/// Closes tab `key` (`close`): not while its browser has an operation running; the window's
/// last closes the window; any other is recorded, where it stood, for a reopen.
pub(crate) fn close(app: &mut App, key: TabKey, tx: &mut Tx) -> bool {
    let Some(at) = tabs(app).iter().position(|tab| *tab == key) else {
        return false;
    };
    let busy = tab_browser(app, key).is_some_and(|browser| browser.has_background_operations());
    if busy {
        tracing::info!(
            tab = key.0,
            "a tab with file operations running is not closed"
        );
        return false;
    }
    if tabs(app).len() == 1 {
        shell::csd_of(app).close(tx);
        return false;
    }
    let location = located(app, key);
    if key == shown_tab(app) {
        leaving(app, tx);
    }
    if !tabs_of::close(app, key, tx) {
        return false;
    }
    app.tab_strip.awaited.retain(|(waiting, _)| *waiting != key);
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
    leaving(app, tx);
    let location = closed.item;
    let key = tabs_of::open(app, Some(location.clone()), at, tx);
    let waits = tab_browser(app, key).is_some_and(|browser| browser.active_location().is_none());
    if waits {
        app.tab_strip.awaited.push((key, location));
    }
}

#[cfg(test)]
mod tests;
