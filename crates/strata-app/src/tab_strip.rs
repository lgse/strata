// SPDX-License-Identifier: MIT

//! The tab strip: C7's `BrowserTabs` across the window, over the shell's tabs
//! ([`shell::tabs`]), with the GTK window's tab commands (`TabWindow` in
//! `src/ui/window/composition/tabs.rs`).
//!
//! - **One strip for the window** ([`Scope::Window`]): its drag, its reveal and the closed
//!   tabs are the window's, as GTK's `TabStrip` and `closed_tabs` are.
//! - **It shows with two tabs or more,** and then holds New tab and the window's Close, which
//!   the header leaves out (`refresh_chrome`); with one tab the header holds C7's
//!   [`NewTabButton`] so a drag onto New tab still opens a second tab.
//! - **A tab is named after its folder** ([`tabs_of::label`]): `~` for home, `/` for the root,
//!   else the folder's name, and "Home" with none. A tab opened at a location is named after it
//!   until its folder opens (`await_location`).
//! - **Opening a folder in a new tab** (`open-in-tab`, `open.rs`): a place (a sidebar row, a
//!   crumb, a view's folder row on middle click), the folders a menu's row acts on, or
//!   Ctrl+Enter on the cursor's folder. The tab opens at the end and is shown, as GTK's
//!   `open_in_new_tab` adds it.
//! - **Drops and the spring** (`drops.rs`): a file drag resting on an inactive accepting tab for
//!   [`HOVER_DELAY`] (450 ms) selects it; dropping files transfers them into that tab's folder
//!   and selects it; sidebar and folder-only view drags open tabs on New tab and the strip.
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
//! - **Tab sessions** live in [`shell::tabs::session`] over `strata_desktop::tabs_session`.
//!
//! Not yet: A19's drag binder, cross-volume confirmation and foreign content saving.

use std::{borrow::Cow, cell::RefCell, path::Path};

use rune::{Color, El, Tx};
use strata_components::{
    contract::dnd::Dnd,
    tabs::{
        BrowserTabs, BrowserTabsState, ClosedTabs, HOVER_DELAY, NewTabButton, Tab, TabIcon, TabId,
        TabOpen, TabWords, TabsData, TabsIntent, TabsLabels, Via, new_tab_key,
    },
};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::{
    keys::{
        Keys, Modifiers,
        bindings::{TAB_NEW, TAB_NUMBERS},
    },
    settings::Preferences,
};
use strata_kit::icons::IconName;

use crate::{
    App, buttons, launch,
    location::labels::with_keys,
    rename,
    shell::{
        self,
        tabs::{self as tabs_of, located, shown_tab, tabs},
    },
    slots::{Frame, Part, Place, Scope, Setup, Strip, TabKey, path_customization},
};

mod drops;
mod keys;
pub(crate) mod open;

/// The strip's state: C7's, its drop classification, and the closed tabs.
pub(crate) struct TabStrip {
    state: BrowserTabsState,
    drops: drops::Drops,
    closed: ClosedTabs<Location>,
}

impl Part for TabStrip {
    const PLACE: Place<App> = Place::Tabs(Strip {
        build,
        new_tab: Some(header_new_tab),
        shows: |app| tabs(app).len() > 1,
    });

    const SCOPE: Scope = Scope::Window;

    const COMMANDS: &'static [crate::commands::Action<App>] = keys::COMMANDS;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            state: BrowserTabsState::default(),
            drops: drops::Drops::new(setup),
            closed: ClosedTabs::new(),
        }
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        tabs_of::forget_arrived(app);
        if drops::blocked(app) {
            app.tab_strip.state.end_drag();
        }
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

/// Overrides the drag-and-drop binder the strip and the header's New tab button attach their zones
/// through (defaults to [`crate::dnd::binder`]).
#[cfg(test)]
pub(crate) fn set_dnd(app: &mut App, dnd: Dnd<App>) {
    app.tab_strip.drops.dnd = Some(dnd);
}

/// Tells the strip and the header's New tab button whether an in-process drag that opens tabs is
/// running (`Some(TabOpen::AtGap)` for a sidebar row, `Some(TabOpen::AtEnd)` for a folder-only
/// view drag, `None` when none runs), lighting New tab while it runs and clearing any open gap or
/// hover when it ends.
pub(crate) fn set_open_drag(app: &mut App, open: Option<TabOpen>) {
    app.tab_strip.drops.open_drag = open;
    app.tab_strip
        .state
        .set_open_drag(open.is_some() && !drops::blocked(app));
    if open.is_none() {
        app.tab_strip.state.end_drag();
    }
}

fn active_dnd(app: &App) -> Dnd<App> {
    app.tab_strip
        .drops
        .dnd
        .clone()
        .unwrap_or_else(|| crate::dnd::binder(app))
}

fn id(key: TabKey) -> TabId {
    TabId(key.0)
}

fn key(id: TabId) -> TabKey {
    TabKey(id.0)
}

fn labels() -> TabsLabels {
    TabsLabels {
        list: tr("Tabs").into(),
        new_tab: tr("New tab").into(),
        new_tab_help: with_keys(
            "New tab (%{keys})",
            Some("New tab"),
            Keys::One(TAB_NEW.shown()),
        )
        .into(),
    }
}

thread_local! {
    /// [`TabWords`] holds `&'static str`, so an owned translation is cached once per locale.
    static WORDS: RefCell<Vec<(String, TabWords)>> = const { RefCell::new(Vec::new()) };
}

fn cached_words(locale: String, close: String) -> TabWords {
    WORDS.with_borrow_mut(|made| {
        if let Some((_, words)) = made.iter().find(|(made, _)| *made == locale) {
            return *words;
        }
        let words = TabWords {
            close: Box::leak(close.into_boxed_str()),
            ..TabWords::default()
        };
        made.push((locale, words));
        words
    })
}

/// The strip's close-button accessible name in the active locale (`strip.rs` in GTK).
fn words() -> TabWords {
    match rust_i18n::t!("Close tab") {
        Cow::Borrowed(close) => TabWords {
            close,
            ..TabWords::default()
        },
        Cow::Owned(close) => cached_words(rust_i18n::locale().to_string(), close),
    }
}

/// The single-tab header's New tab button: C7's [`NewTabButton`] keyed `key` (`"header-new-tab"`)
/// so header focus navigation and drops onto New tab both work while the strip is hidden.
fn header_new_tab(app: &App, key: &'static str) -> El {
    let el = NewTabButton::new(
        &app.tab_strip.state,
        |app: &mut App| &mut app.tab_strip.state,
        tabs(app).len(),
    )
    .key(key)
    .dnd(active_dnd(app))
    .tab_open(drops::tab_open)
    .labels(labels())
    .on_intent(|app: &mut App, intent, tx: &mut Tx| {
        route(app, intent, tx);
    })
    .build(&app.look);
    buttons::on_return(key, el)
}

/// The strip while it shows, with the window's Close after New tab.
fn build(app: &App, close: Option<El>, frame: &mut Frame<'_, App>) -> Option<El> {
    if tabs(app).len() < 2 {
        return None;
    }
    let tabs = named(app);
    let data = TabsData::new(&tabs, Some(id(shown_tab(app))));
    let spring = (!drops::blocked(app)).then_some(HOVER_DELAY);
    let strip = BrowserTabs::new(
        &data,
        &app.tab_strip.state,
        |app: &mut App| &mut app.tab_strip.state,
        |app| &mut app.tab_strip.state.drag,
    )
    .dnd(active_dnd(app))
    .drop_action(drops::drop_action)
    .content_types(crate::dnd::CONTENT_TYPES)
    .tab_open(drops::tab_open)
    .badges(badges_shown(app))
    .spring_delay(spring)
    .labels(labels())
    .words(words())
    .on_intent(|app: &mut App, intent, tx: &mut Tx| {
        route(app, intent, tx);
    });
    let band = strip.build_layered(&app.look, frame.cx, frame.layer);
    debug_assert_eq!(new_tab_key(app.tab_strip.state.key()), "tabs-new");
    Some(buttons::on_return_within("tabs-new", band.children(close)))
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

/// Each tab, in the strip's order, named as GTK names it, with the drops its active folder takes.
fn named(app: &App) -> Vec<Tab<'static>> {
    let home = Location::local(launch::home_directory());
    let preferences = shell::preferences(app);
    tabs(app)
        .iter()
        .map(|&key| {
            let location = located(app, key);
            let name = tabs_of::label(location.as_ref());
            let (icon, tint) = tab_icon(location.as_ref(), &home, preferences);
            Tab::new(id(key), name, icon)
                .tint(tint)
                .accepts(drops::accepts(app, key))
        })
        .collect()
}

fn tab_icon(
    location: Option<&Location>,
    home: &Location,
    preferences: &Preferences,
) -> (TabIcon, Option<Color>) {
    let fallback = icon(location, home);
    let Some(path) = location.and_then(Location::native_path) else {
        return (TabIcon::Name(fallback), None);
    };
    let (color, glyph) = path_customization(preferences, path);
    let resolved = glyph.map_or(TabIcon::Name(fallback), |glyph| glyph.to_tab_icon());
    (resolved, color)
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
        TabsIntent::Select {
            id,
            via: Via::Spring,
        } => !drops::blocked(app) && select(app, key(id), tx),
        TabsIntent::Select { id, .. } => select(app, key(id), tx),
        TabsIntent::Close { id } => close(app, key(id), tx),
        TabsIntent::New => {
            if drops::blocked(app) {
                return false;
            }
            new_tab(app, tx);
            true
        }
        TabsIntent::Reorder { id, to, .. } => tabs_of::move_to(app, key(id), to),
        // The shell titles the window for the tab shown.
        TabsIntent::Title(_) => false,
        // The volumes a hovered drop needs are looked up as its action is asked (`dnd::policy`).
        TabsIntent::DropHover { .. } => false,
        TabsIntent::DropInto { id, dropped } => drops::drop_into(app, id, dropped, tx),
        TabsIntent::OpenTabs { at, dropped } => drops::open_tabs(app, at, dropped, tx),
        TabsIntent::TearOut { .. } | TabsIntent::TearMove { .. } => false,
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

/// Opens a tab at the end at the shown tab's folder, or at the default folder with none
/// (`TabWindow::new_tab`).
pub(crate) fn new_tab(app: &mut App, tx: &mut Tx) {
    if drops::blocked(app) {
        return;
    }
    match app.browser.active_location() {
        Some(location) => {
            open_at(app, Some(location), tabs(app).len(), tx);
        }
        None => {
            let key = open_at(app, None, tabs(app).len(), tx);
            tabs_of::session::open_default_folder(app, key, tx);
        }
    }
}

/// Opens `location` in a tab at the end, shown (`open_in_new_tab`).
pub(crate) fn open_in_new_tab(app: &mut App, location: Location, tx: &mut Tx) {
    if drops::blocked(app) {
        return;
    }
    open_at(app, Some(location), tabs(app).len(), tx);
}

fn open_at(app: &mut App, location: Option<Location>, at: usize, tx: &mut Tx) -> TabKey {
    leaving(app, tx);
    tabs_of::open(app, location, at, tx)
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
    if drops::blocked(app) {
        return;
    }
    let Some(closed) = app.tab_strip.closed.reopen() else {
        return;
    };
    let at = closed.index_in(tabs(app).len());
    open_at(app, Some(closed.item), at, tx);
}

#[cfg(test)]
mod tests;
