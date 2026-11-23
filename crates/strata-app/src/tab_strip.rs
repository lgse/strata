// SPDX-License-Identifier: MIT

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

#[cfg(test)]
pub(crate) fn set_dnd(app: &mut App, dnd: Dnd<App>) {
    app.tab_strip.drops.dnd = Some(dnd);
}

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

fn words() -> TabWords {
    match rust_i18n::t!("Close tab") {
        Cow::Borrowed(close) => TabWords {
            close,
            ..TabWords::default()
        },
        Cow::Owned(close) => cached_words(rust_i18n::locale().to_string(), close),
    }
}

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

fn badges_shown(app: &App) -> bool {
    let mods = shell::held_modifiers(app);
    let mut held = Modifiers::empty();
    held.set(Modifiers::SHIFT_MASK, mods.shift);
    held.set(Modifiers::CONTROL_MASK, mods.ctrl || mods.cmd);
    held.set(Modifiers::ALT_MASK, mods.alt);
    held.set(Modifiers::SUPER_MASK, mods.logo);
    held == TAB_NUMBERS.modifiers()
}

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
