// SPDX-License-Identifier: MIT

use std::{collections::VecDeque, path::Path, rc::Rc};

use rune::Tx;
use strata_core::{i18n::tr, model::Location};
use strata_fs::app::Browser;

use super::{App, preferences, publish, settle};
use crate::{
    TabParts, pane,
    slots::{Queued, Registry, Setup, ShownView, Slot, Source, TabKey, Update},
};

pub(crate) mod session;

pub(crate) const HELD_EVENTS: usize = 512;

pub(crate) const HELD_ROWS: u64 = 20_000;

/// The shell's own bits of a tab: which view it shows, and what its views share.
pub(crate) struct Here {
    pub(crate) key: TabKey,
    /// The registered parts, with the view this tab shows.
    pub(super) parts: Registry<'static, App>,
    /// Which view this tab shows, as its listeners and its parts' timers read it.
    pub(super) shown: ShownView,
    pub(super) restore: pane::Memory,
    /// Whether the browser groups by file type for the view, as last mounted.
    pub(super) grouped: bool,
    /// The element that had the keyboard when the tab was hidden.
    focus: Option<String>,
    awaited: Option<Location>,
    /// The browser's active location last seen by [`session::observe_locations`].
    observed: Option<Location>,
}

impl Here {
    pub(super) fn new(key: TabKey, parts: Registry<'static, App>) -> Self {
        Self {
            key,
            parts,
            shown: parts.shown_now(),
            restore: pane::Memory::default(),
            grouped: false,
            focus: None,
            awaited: None,
            observed: None,
        }
    }
}

/// What a hidden tab's browser and services queued, held until it is shown.
#[derive(Default)]
pub(crate) struct Held {
    updates: VecDeque<(Source, Update<App>)>,
    events: usize,
    rows: u64,
    /// The browser's events outran what is held: they were dropped, and the view is rebuilt.
    outran: bool,
    #[cfg_attr(
        not(feature = "qa"),
        expect(dead_code, reason = "only the QA backend's invariant reads it")
    )]
    since: u64,
    routed: u64,
    dropped: u64,
}

impl Held {
    /// Nothing held yet for a tab hidden now, with `since` updates queued for it until now.
    fn since(since: u64) -> Self {
        Self {
            since,
            ..Self::default()
        }
    }

    fn hold(&mut self, source: Source, update: Update<App>) {
        self.routed += 1;
        match source {
            Source::Event { .. } if self.outran => self.dropped += 1,
            Source::Event { rows } => {
                self.events += 1;
                self.rows += rows;
                self.updates.push_back((source, update));
                if self.events > HELD_EVENTS || self.rows > HELD_ROWS {
                    self.outrun();
                }
            }
            Source::Service => self.updates.push_back((source, update)),
        }
    }

    /// Drops every held event: the tab's view is rebuilt from its browser when it is shown.
    fn outrun(&mut self) {
        let before = self.updates.len();
        self.updates
            .retain(|(source, _)| matches!(source, Source::Service));
        self.dropped += (before - self.updates.len()) as u64;
        self.outran = true;
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.updates.len()
    }

    #[cfg(test)]
    pub(crate) fn outran(&self) -> bool {
        self.outran
    }

    #[cfg(feature = "qa")]
    pub(crate) fn accounted(&self, (queued, waiting): (u64, u64)) -> bool {
        let kept = self.updates.len() as u64 + self.dropped == self.routed;
        kept && queued == self.since + self.routed + waiting
    }
}

/// A hidden tab.
pub(crate) struct Parked {
    browser: Rc<Browser>,
    here: Here,
    parts: TabParts,
    held: Held,
}

impl Parked {
    pub(super) fn new(browser: Rc<Browser>, here: Here, parts: TabParts) -> Self {
        Self {
            browser,
            here,
            parts,
            held: Held::default(),
        }
    }
}

/// The window's tabs, in the strip's order, and the hidden ones' states.
pub(crate) struct Tabs {
    order: Vec<TabKey>,
    parked: Vec<(TabKey, Parked)>,
    issued: u64,
    /// Whether this window persists its tabs to `tabs.toml`: a plain launch only.
    persist: bool,
    /// True while [`session::restore_session`] builds the saved tabs so none saves midway.
    restoring: bool,
}

impl Tabs {
    /// The first tab's key, and the tabs with it alone.
    pub(super) fn first() -> (TabKey, Self) {
        let key = TabKey(1);
        let tabs = Self {
            order: vec![key],
            parked: Vec::new(),
            issued: 1,
            persist: false,
            restoring: false,
        };
        (key, tabs)
    }

    fn issue(&mut self) -> TabKey {
        self.issued += 1;
        TabKey(self.issued)
    }

    fn parked_mut(&mut self, key: TabKey) -> Option<&mut Parked> {
        let found = self.parked.iter_mut().find(|(parked, _)| *parked == key);
        found.map(|(_, parked)| parked)
    }

    fn take(&mut self, key: TabKey) -> Option<Parked> {
        let at = self.parked.iter().position(|(parked, _)| *parked == key)?;
        Some(self.parked.remove(at).1)
    }

    pub(super) fn for_each_parked(
        &self,
        mut f: impl FnMut(&Rc<Browser>, Option<&'static crate::slots::View<App>>),
    ) {
        for (_, parked) in &self.parked {
            f(&parked.browser, parked.here.parts.shown());
        }
    }

    pub(super) fn has_held_updates(&self) -> bool {
        self.parked
            .iter()
            .any(|(_, parked)| !parked.held.updates.is_empty())
    }

    pub(super) fn collect_parked_ids(
        &self,
        out: &mut impl Extend<strata_components::contract::EntryId>,
    ) {
        for (_, parked) in &self.parked {
            if let Some(columns) = &parked.parts.columns {
                columns.collect_ids(out);
            }
            if let Some(icons) = &parked.parts.icons {
                icons.single.pane.collect_ids(out);
            }
            if let Some(list) = &parked.parts.list {
                list.single.pane.collect_ids(out);
            }
            if let Some(filter) = &parked.parts.filter {
                filter.pane.collect_ids(out);
            }
            if let Some(peek) = &parked.parts.peek {
                peek.collect_ids(out);
            }
            parked.here.restore.collect_ids(out);
        }
    }

    /// Holds `queued` for its hidden tab. False when the tab is gone: it is dropped.
    pub(super) fn hold(&mut self, key: TabKey, queued: Queued<App>) -> bool {
        let Some(parked) = self.parked_mut(key) else {
            return false;
        };
        parked.held.hold(queued.source, queued.update);
        true
    }

    /// The hidden tabs, each with what is held for it.
    #[cfg(feature = "qa")]
    pub(crate) fn held(&self) -> impl Iterator<Item = (TabKey, &Held)> {
        self.parked.iter().map(|(key, parked)| (*key, &parked.held))
    }
}

/// The tabs, in the strip's order.
pub(crate) fn tabs(app: &App) -> &[TabKey] {
    &app.window.tabs.order
}

/// The tab shown.
pub(crate) fn shown_tab(app: &App) -> TabKey {
    app.window.here.key
}

/// The browser of tab `key`, shown or hidden.
pub(crate) fn tab_browser(app: &App, key: TabKey) -> Option<&Rc<Browser>> {
    if key == app.window.here.key {
        return Some(&app.browser);
    }
    let mut parked = app.window.tabs.parked.iter();
    parked
        .find(|(parked, _)| *parked == key)
        .map(|(_, parked)| &parked.browser)
}

/// The shell's bits of tab `key`, shown or hidden.
fn here_of(app: &App, key: TabKey) -> Option<&Here> {
    if key == app.window.here.key {
        return Some(&app.window.here);
    }
    let mut parked = app.window.tabs.parked.iter();
    parked
        .find(|(parked, _)| *parked == key)
        .map(|(_, parked)| &parked.here)
}

pub(crate) fn located(app: &App, key: TabKey) -> Option<Location> {
    let browser = tab_browser(app, key)?;
    let awaited = || here_of(app, key).and_then(|here| here.awaited.clone());
    browser.active_location().or_else(awaited)
}

pub(super) fn set_awaited(app: &mut App, key: TabKey, awaited: Option<Location>) {
    if key == app.window.here.key {
        app.window.here.awaited = awaited;
        return;
    }
    if let Some(parked) = app.window.tabs.parked_mut(key) {
        parked.here.awaited = awaited;
    }
}

pub(crate) fn forget_arrived(app: &mut App) {
    session::observe_locations(app);
}

pub(crate) fn is_busy(app: &App, key: TabKey) -> bool {
    tab_browser(app, key).is_some_and(|browser| browser.has_background_operations())
}

pub(crate) fn operations_active(app: &mut App) {
    let error = crate::error_dialog::Error::new(
        tr("File operations are still active"),
        tr(
            "Wait for these operations to finish, or cancel them before closing this tab or window. Cancellation does not undo completed changes.",
        ),
    );
    crate::error_dialog::show(app, error);
}

pub(crate) fn label(location: Option<&Location>) -> String {
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

pub(crate) fn open(app: &mut App, location: Option<Location>, at: usize, tx: &mut Tx) -> TabKey {
    let key = app.window.tabs.issue();
    let saved = preferences(app).clone();
    let parts = app.window.here.parts.fresh(&saved.browser_mode);
    let mut here = Here::new(key, parts);
    let browser = super::browser_for(&saved);
    let later = app.window.events.later_for(Some(key));
    let setup = Setup {
        settings: &saved,
        ids: &app.window.ids,
        worn: &app.window.worn,
        later: &later,
        shown: &here.shown,
    };
    let tab_parts = TabParts::new(&setup);
    let window = &app.window;
    here.grouped = super::attach(
        super::TabAttach {
            events: &window.events,
            folder_views: &window.folder_views,
            power_saver: window.power_saver,
            presence: super::presence::of(window),
        },
        (&browser, parts),
        &setup,
        key,
    );
    let at = at.min(app.window.tabs.order.len());
    app.window.tabs.order.insert(at, key);
    let mut parked = Parked::new(browser, here, tab_parts);
    parked.held = Held::since(0);
    app.window.tabs.parked.push((key, parked));
    show(app, key, tx);
    if let Some(location) = location {
        publish(app);
        app.browser.navigate(location.clone());
        if app.browser.active_location().is_none() {
            app.window.here.awaited = Some(location);
            app.retitle(tx);
        }
        settle(app, tx);
        session::persist_session(app);
    }
    key
}

pub(crate) fn show(app: &mut App, key: TabKey, tx: &mut Tx) -> bool {
    if key == app.window.here.key {
        return false;
    }
    let Some(mut parked) = app.window.tabs.take(key) else {
        return false;
    };
    leave(app, tx);
    let held = std::mem::take(&mut parked.held);
    std::mem::swap(&mut app.browser, &mut parked.browser);
    std::mem::swap(&mut app.window.here, &mut parked.here);
    app.swap_parts(&mut parked.parts);
    let left = parked.here.key;
    // What still waits in the queue for it is routed to it from now on.
    let (queued, waiting) = app.window.events.queued_for(left);
    parked.held = Held::since(queued - waiting);
    app.window.tabs.parked.push((left, parked));
    app.window.shown.follow(app.window.here.parts);
    catch_up(app, held, tx);
    let parts = app.window.here.parts;
    super::retitle_now(app, tx);
    for part in parts.active() {
        (part.tab_shown)(app, tx);
    }
    for part in parts.active() {
        (part.switched)(app, tx);
    }
    give_back_the_keyboard(app, tx);
    settle(app, tx);
    session::persist_session(app);
    true
}

fn leave(app: &mut App, tx: &mut Tx) {
    publish(app);
    let parts = app.window.here.parts;
    let owned = parts.active().any(|part| (part.keyboard)(app).is_some());
    let focus = tx.focused().filter(|key| !owned && parts.in_panes(key));
    app.window.here.focus = focus.map(str::to_owned);
    for part in parts.active() {
        (part.tab_hidden)(app, tx);
    }
    publish(app);
}

/// Applies what the tab shown now was held, and what changed while it was hidden.
fn catch_up(app: &mut App, held: Held, tx: &mut Tx) {
    let saved = preferences(app);
    let (sort, mode) = (
        strata_desktop::settings::sort_preferences(saved),
        saved.browser_mode.clone(),
    );
    let outran = held.outran;
    for (_, update) in held.updates {
        update(app, tx);
    }
    let parts = app.window.here.parts;
    for part in parts.active() {
        (part.caught_up)(app);
    }
    app.browser.apply_default_preferences(sort);
    let shows_mode = super::shown_view(app).is_some_and(|view| (view.shows)(&mode));
    if !shows_mode {
        crate::rename::cancel(app, tx);
        super::show_view(app, &mode, tx);
    } else if outran || super::grouping_changed(app) {
        super::regroup(app, tx);
    }
}

fn can_restore_focus(app: &App, key: &str) -> bool {
    let parts = app.window.here.parts;
    if !parts.in_panes(key) {
        return false;
    }
    let in_sidebar = parts.of(Slot::Sidebar).any(|entry| {
        key.strip_prefix(entry.name)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
    });
    !in_sidebar
        || parts
            .sidebar()
            .and_then(|sidebar| (sidebar.focus)(app))
            .is_some()
}

fn give_back_the_keyboard(app: &mut App, tx: &mut Tx) {
    let key = app
        .window
        .here
        .focus
        .take()
        .filter(|key| can_restore_focus(app, key));
    let key = key.or_else(|| super::view_focus(app).map(str::to_owned));
    if let Some(key) = key {
        tx.focus(key, true);
    }
}

pub(crate) fn close(app: &mut App, key: TabKey, tx: &mut Tx) -> bool {
    let order = &app.window.tabs.order;
    let Some(at) = order.iter().position(|tab| *tab == key) else {
        return false;
    };
    if order.len() == 1 {
        return false;
    }
    if key == app.window.here.key {
        let next = order.get(at + 1).or_else(|| order.get(at - 1)).copied();
        if let Some(next) = next {
            show(app, next, tx);
        }
    }
    app.window.tabs.order.remove(at);
    drop(app.window.tabs.take(key));
    super::ids_prune::prune_if_needed(app);
    session::persist_session(app);
    true
}

pub(crate) fn move_to(app: &mut App, key: TabKey, to: usize) -> bool {
    let order = &mut app.window.tabs.order;
    let Some(at) = order.iter().position(|tab| *tab == key) else {
        return false;
    };
    order.remove(at);
    let to = to.min(order.len());
    order.insert(to, key);
    if at != to {
        session::persist_session(app);
    }
    true
}

#[cfg(test)]
mod tests;
