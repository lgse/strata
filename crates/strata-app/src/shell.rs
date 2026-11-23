// SPDX-License-Identifier: MIT

use std::{path::Path, rc::Rc};

use rune::{Cx, El, Tx, col, row};
use strata_components::contract::EntryId;
use strata_components::header::{
    self, Header, HeaderLabels, Shown, WindowAction, WindowButtons, WindowChrome,
};
use strata_components::thumbnails::{Settings, ThumbnailStore};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::{
    keys::{
        Keys,
        bindings::{SEARCH, TAB_NEW},
    },
    settings::{TrashedCustomizations, sort_preferences, trashed::TrashedValues},
};
use strata_fs::{
    adapters::{LocalFileSource, LocalOperationProvider},
    app::{Browser, BrowserEvent, FreshnessInput},
};
use strata_kit::icons::IconName;
use strata_platform::{
    backdrop_blur::BackdropBlur,
    csd::{self, Csd},
};

use crate::{
    App, Built, PARTS, buttons,
    commands::{self, Holds, Live, On},
    events, folder_views, launch,
    location::{self, labels::with_keys},
    pane,
    settings::Saved,
    slots::{
        Entry, Frame, Heard, Ids, Inbox, Later, Registry, Setup, ShownView, Slot, TabKey, View,
        Worn,
    },
    startup::UiThread,
};

pub(crate) mod close_guard;
mod held;
mod ids_prune;
mod preferences;
mod presence;
mod siblings;
pub(crate) mod tabs;
mod thumbnail_budget;

pub(crate) use held::{held_modifiers, modifiers};
use preferences::{PowerMonitorGuard, WindowServices, sync_browser_freshness};
pub(crate) use preferences::{
    follow_another_window, follow_look, follow_look_tx, interface_renderer_restart_required,
    language_restart_required, reload_omarchy, sync_default_preferences, sync_folder_sorts,
    sync_freshness, sync_live_preferences, themes,
};
pub(crate) use presence::window_shown;
pub(crate) use siblings::{share_settings, window_id, window_later};
use tabs::{Here, Tabs};

struct Labels {
    window: HeaderLabels,
}

impl Labels {
    fn translated() -> Self {
        Self {
            window: HeaderLabels {
                minimize: tr("Minimize window"),
                maximize: tr("Maximize window"),
                restore: tr("Restore window"),
                maximize_or_restore: tr("Maximize or restore window"),
                close: tr("Close window"),
            },
        }
    }
}

/// What the window keeps for itself, beside the parts' state.
pub(crate) struct Window {
    /// Which of the process's windows this is: none has the same.
    id: u64,
    settings: Saved,
    events: Inbox<App>,
    /// The shown tab's own bits: the view it shows, and what its views share.
    here: Here,
    /// The window's tabs, and what each hidden one keeps.
    tabs: Tabs,
    /// Which view the shown tab shows, as a part the window keeps one of reads it later.
    shown: ShownView,
    /// The table that names an entry for the components: every part holds the same one.
    ids: Ids,
    /// The saved item colours and icons, indexed by [`EntryId`].
    worn: Rc<Worn>,
    /// Customizations of trashed items kept in memory for Put back or undo during this session.
    trashed_customizations: TrashedValues<TrashedCustomizations>,
    /// Whether queued item relocations, removals or restores mutated `settings` this batch.
    customizations_dirty: bool,
    /// The browser's active location as of the last `poll`: what the title names.
    location: Option<Location>,
    title: String,
    csd: Csd,
    backdrop_blur: BackdropBlur,
    /// Whether the window has keyboard focus.
    active: bool,
    /// Whether the window is on screen ([`presence`]).
    on_screen: bool,
    /// The modifier keys held down in the window ([`held`]).
    held: rune::Mods,
    /// What the key catalog reads of the settings at each press and label.
    keys: Rc<Live>,
    labels: Labels,
    applied: (u64, u64),
    thumbnails: ThumbnailStore,
    /// The store's place in the process's budget ([`thumbnail_budget`]).
    _thumbnail_share: thumbnail_budget::Share,
    /// The per-folder views, one store for the process ([`crate::folder_views`]).
    folder_views: folder_views::Kept,
    /// Theme library, Omarchy state, and system appearance monitor (`crate::theme`).
    themes: crate::theme::ThemeEngine,
    /// Whether the host allows motion (`Offscreen::without_animations` clears it).
    animations_allowed: bool,
    /// System power-saver state (`gio::PowerProfileMonitor`).
    power_saver: bool,
    _power_monitor: Option<PowerMonitorGuard>,
    /// Whether a modal layer was open at the last poll ([`grab`]).
    grab: grab::Grab,
    size: (f32, f32),
}

/// Tail chunk size for Rune's staged publication: first paint still lands 128 rows
/// immediately, and larger tails avoid repeated O(k) virtual-list key scans (Rune #203).
const RUNE_TAIL_PUBLISH_CHUNK: usize = 131_072;

fn browser_for(saved: &strata_desktop::settings::Preferences) -> Rc<Browser> {
    let browser = Browser::with_preferences(Rc::new(LocalFileSource), sort_preferences(saved));
    browser.set_publication_chunks(RUNE_TAIL_PUBLISH_CHUNK, RUNE_TAIL_PUBLISH_CHUNK);
    browser.set_operation_provider(Rc::new(LocalOperationProvider));
    browser
}

pub(super) struct TabAttach<'a> {
    pub(super) events: &'a Inbox<App>,
    pub(super) folder_views: &'a folder_views::Kept,
    pub(super) power_saver: bool,
    /// The window's presence, which the new tab's browser starts with ([`presence`]).
    pub(super) presence: strata_fs::app::Presence,
}

fn attach(
    ctx: TabAttach<'_>,
    (browser, parts): (&Rc<Browser>, Registry<'static, App>),
    setup: &Setup<'_>,
    key: TabKey,
) -> bool {
    let grouped = parts
        .shown()
        .is_some_and(|view| mount(browser, view, setup.settings));
    let mut listeners = parts.listeners(setup);
    listeners.push(Box::new(hear));
    ctx.events.observe_for(browser, listeners, Some(key));
    let queue = ctx.events.later();
    // The browser's sort or hidden-files choice changed: the saved settings follow it.
    browser.observe_preferences(move |view| {
        queue.then(move |app: &mut App, _: &mut Tx| {
            app.window.settings.store_view(view);
            false
        });
    });
    let queue = ctx.events.later();
    browser.observe(move |event| follow_item_customizations(event, &queue));
    ctx.folder_views.follow_sorts(browser, setup.settings);
    sync_browser_freshness(
        browser,
        parts.shown(),
        setup.settings.watch_network_folders,
        ctx.power_saver,
    );
    presence::give(browser, ctx.presence);
    grouped
}

fn follow_item_customizations(event: &BrowserEvent, queue: &Later<App>) {
    match event {
        BrowserEvent::ItemRelocated { from, to, merged } => {
            let Some(from) = from.native_path().map(Path::to_path_buf) else {
                return;
            };
            let (to, merged) = (to.native_path().map(Path::to_path_buf), *merged);
            queue.then(move |app: &mut App, _: &mut Tx| {
                app.window.customizations_dirty |= app.window.settings.mutate(|preferences| {
                    preferences.relocate_item_customizations(&from, to.as_deref(), merged)
                });
                false
            });
        }
        BrowserEvent::ItemRemoved {
            location,
            trash_identity,
        } => {
            let Some(path) = location.native_path().map(Path::to_path_buf) else {
                return;
            };
            let trash_identity = *trash_identity;
            queue.then(move |app: &mut App, _: &mut Tx| {
                let Window {
                    settings,
                    trashed_customizations,
                    customizations_dirty,
                    ..
                } = &mut app.window;
                *customizations_dirty |= settings.mutate(|preferences| {
                    preferences.forget_item_customizations(
                        &path,
                        trash_identity,
                        trashed_customizations,
                    )
                });
                false
            });
        }
        BrowserEvent::ItemRestored { location, identity } => {
            let (Some(path), Some(identity)) =
                (location.native_path().map(Path::to_path_buf), *identity)
            else {
                return;
            };
            queue.then(move |app: &mut App, _: &mut Tx| {
                let Window {
                    settings,
                    trashed_customizations,
                    customizations_dirty,
                    ..
                } = &mut app.window;
                *customizations_dirty |= settings.mutate(|preferences| {
                    preferences.restore_item_customizations(&path, identity, trashed_customizations)
                });
                false
            });
        }
        _ => {}
    }
}

fn hear(event: &BrowserEvent, _: &Browser, heard: &mut Heard<'_, App>) {
    let moved = matches!(
        event,
        BrowserEvent::Reset
            | BrowserEvent::ColumnsTruncated { .. }
            | BrowserEvent::ColumnAdded { .. }
            | BrowserEvent::ColumnsRelocated { .. }
            | BrowserEvent::FocusChanged { .. }
            | BrowserEvent::SelectionSetChanged { .. }
    );
    if moved {
        heard.then(App::retitle);
    }
    if let BrowserEvent::FolderSortChosen {
        location,
        sort_key,
        sort_direction,
        name_comparison,
    } = event
    {
        let (location, options) = (
            location.clone(),
            (*sort_key, *sort_direction, *name_comparison),
        );
        heard.then(move |app: &mut App, tx: &mut Tx| {
            folder_views::chosen(app, &location, options, tx);
            false
        });
    }
}

pub(crate) fn reopened(app: &mut App) {
    app.window.on_screen = true;
    app.window.active = false;
    presence::follow(app);
    if transparency(app).varies_with_focus() {
        follow_look(app);
    }
    window_later(app).then(|app, tx| {
        retitle_now(app, tx);
        true
    });
}

/// Names the window for the shown tab, whether or not its location changed.
fn retitle_now(app: &mut App, tx: &mut Tx) {
    app.window.location = None;
    app.retitle(tx);
}

fn grouping_changed(app: &App) -> bool {
    shown_view(app).is_some_and(|view| {
        let wanted = preferences(app).group_by_type && view.supports_type_grouping;
        wanted != app.window.here.grouped
    })
}

impl App {
    pub(crate) fn new(ui: &UiThread, csd: Csd, settings: Saved) -> Self {
        Self::over(PARTS, ui, csd, settings)
    }

    pub(crate) fn new_chooser(
        ui: &UiThread,
        csd: Csd,
        settings: Saved,
        spec: (
            crate::portal::ChooserRequest,
            Rc<crate::chooser::source::ChooserFileSource>,
            Option<crate::chooser::state::DestinationHost>,
            crate::portal::Completion,
        ),
    ) -> Self {
        let (request, source, destination_host, completion) = spec;
        let saved = settings.preferences();
        let browser = Browser::with_preferences(source.clone(), sort_preferences(saved));
        browser.set_chooser_mode(true);
        browser.set_operation_provider(Rc::new(LocalOperationProvider));
        let title = request.title.clone();
        let mut app = Self::over_browser(PARTS, csd, settings, (browser, title));
        let _ = ui;
        crate::chooser::mount_request(&mut app, request, source, (destination_host, completion));
        app
    }

    pub(crate) fn over(
        parts: &'static [Entry<App>],
        _ui: &UiThread,
        csd: Csd,
        settings: Saved,
    ) -> Self {
        let browser = browser_for(settings.preferences());
        Self::over_browser(parts, csd, settings, (browser, title_for(None)))
    }

    fn over_browser(
        parts: &'static [Entry<App>],
        csd: Csd,
        mut settings: Saved,
        (browser, title): (Rc<Browser>, String),
    ) -> Self {
        let events = Inbox::new();
        let services = WindowServices::load(&mut settings, &events.later());
        let saved = settings.preferences();
        strata_core::util::set_date_format(strata_core::util::DateFormat::parse(
            &saved.date_format,
        ));
        let (built, mut window, look) =
            Window::build(parts, csd, settings, (events, &browser, services));
        window.title = title;
        Self::assemble(look, browser, window, built)
    }

    /// The window's size in whole points, once it has been laid out.
    pub(crate) fn window_size(&self) -> Option<(i32, i32)> {
        let (width, height) = self.window.size;
        #[expect(clippy::cast_possible_truncation, reason = "a window's size fits i32")]
        let size = (width.round() as i32, height.round() as i32);
        (size.0.min(size.1) > 0).then_some(size)
    }

    pub(crate) fn open(&self, location: Location) {
        self.browser.navigate(location);
    }

    pub(crate) fn start(&mut self, initial: launch::Initial) {
        match initial {
            launch::Initial::Explicit(location) => self.open(location),
            launch::Initial::Plain(restored) => tabs::session::queue_plain_start(self, restored),
            launch::Initial::Present => tabs::session::queue_present_start(self),
            launch::Initial::Argument(location) => {
                crate::application::open_argument(self, location)
            }
            launch::Initial::Reveal(request) => crate::application::open_reveal(self, request),
        }
    }

    pub(super) fn retitle(&mut self, tx: &mut Tx) -> bool {
        let location = tabs::located(self, self.window.here.key);
        if location == self.window.location {
            return false;
        }
        if !self.browser.is_chooser_mode() {
            self.window.title = title_for(location.as_ref());
        }
        tx.set_title(self.window.title.clone());
        self.window.location = location;
        ids_prune::prune_if_needed(self);
        true
    }

    fn chrome(&self) -> WindowChrome {
        let state = self.window.csd.state();
        let capabilities = self.window.csd.capabilities();
        WindowChrome {
            maximized: state.maximized,
            fullscreen: state.fullscreen,
            can_minimize: capabilities.minimize,
            can_maximize: capabilities.maximize,
            active: self.window.active,
        }
    }
}

impl App {
    pub(crate) fn take_yield_paint(&self) -> bool {
        self.window.events.take_yield_paint()
    }
}

/// What the offscreen host reads.
#[cfg(any(test, feature = "qa"))]
impl App {
    /// The window title this state last asked for.
    pub(crate) fn title(&self) -> &str {
        &self.window.title
    }

    /// Whether the browser has sent something `poll` has not applied.
    pub(crate) fn has_queued_changes(&self) -> bool {
        !self.window.events.is_empty()
    }
}

#[cfg(feature = "qa")]
pub(crate) fn hidden_tabs(app: &App) -> impl Iterator<Item = (TabKey, bool)> {
    let events = &app.window.events;
    let held = app.window.tabs.held();
    held.map(|(key, held)| (key, held.accounted(events.queued_for(key))))
}

/// What the tests read.
#[cfg(test)]
impl App {
    pub(crate) fn browser(&self) -> &Rc<Browser> {
        &self.browser
    }

    pub(crate) fn location(&self) -> Option<&Location> {
        self.window.location.as_ref()
    }

    /// How many rows the browser's events have had copied out of it.
    pub(crate) fn rows_copied(&self) -> u64 {
        self.window.events.copied()
    }

    /// The way into the window's queue, as `Setup::later` hands it to a part.
    pub(crate) fn later(&self) -> crate::slots::Later<Self> {
        self.window.events.later()
    }
}

pub(crate) fn saved(app: &mut App) -> &mut Saved {
    &mut app.window.settings
}

/// Turns off motion for the host (`Offscreen::without_animations`), overriding `reduce_motion`.
#[cfg(feature = "qa")]
pub(crate) fn disable_animations(app: &mut App) {
    app.window.animations_allowed = false;
    app.look.animations = false;
}

/// The saved settings as they are now, for a build or a reader that does not change them.
pub(crate) fn preferences(app: &App) -> &strata_desktop::settings::Preferences {
    app.window.settings.preferences()
}

/// The saved surface transparency and frosted glass settings.
pub(crate) fn transparency(app: &App) -> strata_desktop::settings::TransparencyPreferences {
    app.window.settings.transparency()
}

/// The window's saved item colours and icons, indexed by [`EntryId`].
pub(crate) fn worn(app: &App) -> &Rc<Worn> {
    &app.window.worn
}

pub(crate) fn refresh_worn(app: &mut App) {
    let next = app.window.worn.version().wrapping_add(1);
    app.window.worn =
        Worn::from_preferences(app.window.settings.preferences(), &app.window.ids, next);
    let parts = app.window.here.parts;
    for part in parts.active() {
        (part.customized)(app);
    }
}

/// Changes an item's saved colour or icon, rebuilds [`worn`], and tells the active parts.
pub(crate) fn change_customization(
    app: &mut App,
    change: impl FnOnce(&mut strata_desktop::settings::Preferences),
) {
    app.window.settings.change(change);
    refresh_worn(app);
}

fn title_for(location: Option<&Location>) -> String {
    location.map_or_else(
        || "Strata".to_owned(),
        |location| format!("{} — Strata", tabs::label(Some(location))),
    )
}

pub(crate) fn csd_of(app: &mut App) -> &mut Csd {
    &mut app.window.csd
}

pub(crate) fn bind_backdrop_blur(app: &mut App, blur: BackdropBlur) {
    app.window.backdrop_blur.detach();
    blur.set_enabled(app.look.transparency.frosted_glass);
    app.window.backdrop_blur = blur;
}

/// Detaches the window's compositor backdrop-blur handle before its native surface closes.
pub(crate) fn detach_backdrop_blur(app: &mut App) {
    app.window.backdrop_blur.detach();
}

#[cfg(test)]
pub(crate) fn backdrop_blur_enabled(app: &App) -> bool {
    app.window.backdrop_blur.is_enabled()
}

fn thumbnail_store(saved: &strata_desktop::settings::Preferences) -> ThumbnailStore {
    strata_preview::sandbox::browser::set_worker_limit(saved.thumbnail_workers);
    let mut store = ThumbnailStore::new(Settings::default().budget(thumbnail_budget::share()));
    let _ = store.set_enabled(saved.show_thumbnails);
    store
}

struct FirstTab {
    built: Built,
    here: Here,
    tabs: Tabs,
    shown: ShownView,
    ids: Ids,
    worn: Rc<Worn>,
    folder_views: folder_views::Kept,
}

impl FirstTab {
    fn build(
        parts: &'static [Entry<App>],
        saved: &strata_desktop::settings::Preferences,
        events: &Inbox<App>,
        (browser, power_saver): (&Rc<Browser>, bool),
    ) -> Self {
        let later = events.later();
        let ids = Ids::default();
        let worn = Worn::from_preferences(saved, &ids, 0);
        debug_assert_eq!(Registry::check(parts, commands::own()), Ok(()));
        let registry = Registry::new(parts, &saved.browser_mode);
        let (key, tabs) = Tabs::first();
        let mut here = Here::new(key, registry);
        let shown = registry.shown_now();
        let tab_later = events.later_for(Some(key));
        let tab = Setup {
            settings: saved,
            ids: &ids,
            worn: &worn,
            later: &tab_later,
            shown: &here.shown,
        };
        let window_setup = Setup {
            later: &later,
            shown: &shown,
            ..tab
        };
        let folder_views = folder_views::Kept::load(later.clone());
        here.grouped = attach(
            TabAttach {
                events,
                folder_views: &folder_views,
                power_saver,
                // A window starts with the keyboard, as `Window::build` says it does.
                presence: strata_fs::app::Presence::Focused,
            },
            (browser, registry),
            &tab,
            key,
        );
        let built = Built::new(&tab, &window_setup);
        Self {
            built,
            here,
            tabs,
            shown,
            ids,
            worn,
            folder_views,
        }
    }
}

impl Window {
    fn build(
        parts: &'static [Entry<App>],
        csd: Csd,
        settings: Saved,
        (events, browser, services): (Inbox<App>, &Rc<Browser>, WindowServices),
    ) -> (Built, Self, strata_kit::look::Look) {
        let saved = settings.preferences();
        let first = FirstTab::build(parts, saved, &events, (browser, services.power_saver));
        let keys = Live::registered(saved, first_window_reports().as_deref());
        let thumbnail_share = thumbnail_budget::Share::join(&events.later());
        let thumbnails = thumbnail_store(saved);
        let look = services.themes.build_look(
            saved,
            crate::theme::LookContext {
                transparency: settings.transparency(),
                animations_allowed: true,
                window_active: true,
            },
        );
        let backdrop_blur = BackdropBlur::new();
        backdrop_blur.set_enabled(look.transparency.frosted_glass);
        let window = Self {
            id: siblings::next_id(),
            settings,
            events,
            here: first.here,
            tabs: first.tabs,
            shown: first.shown,
            ids: first.ids,
            worn: first.worn,
            trashed_customizations: TrashedValues::default(),
            customizations_dirty: false,
            location: None,
            title: title_for(None),
            csd,
            backdrop_blur,
            active: true,
            on_screen: true,
            held: rune::Mods::default(),
            keys,
            labels: Labels::translated(),
            applied: (0, 0),
            thumbnails,
            _thumbnail_share: thumbnail_share,
            folder_views: first.folder_views,
            themes: services.themes,
            animations_allowed: true,
            power_saver: services.power_saver,
            _power_monitor: services.power_monitor,
            grab: grab::Grab::default(),
            size: (0.0, 0.0),
        };
        (first.built, window, look)
    }

    /// The window's thumbnails, for a view's handlers: borrow this field with the view's own.
    pub(crate) fn thumbnails_mut(&mut self) -> &mut ThumbnailStore {
        &mut self.thumbnails
    }

    pub(crate) fn folder_views(&self) -> &folder_views::Kept {
        &self.folder_views
    }

    pub(crate) fn folder_views_mut(&mut self) -> &mut folder_views::Kept {
        &mut self.folder_views
    }

    pub(crate) fn themes_mut(&mut self) -> &mut crate::theme::ThemeEngine {
        &mut self.themes
    }
}

/// Where each folder was left, which the single-pane views share.
pub(crate) fn restore_memory(app: &App) -> &pane::Memory {
    &app.window.here.restore
}

/// The window's thumbnails, as a view's build reads them.
pub(crate) fn thumbnails(app: &App) -> &ThumbnailStore {
    &app.window.thumbnails
}

pub(crate) fn shown_view(app: &App) -> Option<&'static View<App>> {
    app.window.here.parts.shown()
}

pub(crate) fn show_view(app: &mut App, mode: &str, tx: &mut Tx) -> bool {
    let mut parts = app.window.here.parts;
    if !parts.show(mode) {
        return false;
    }
    let Some(view) = parts.shown() else {
        return false;
    };
    publish(app);
    app.browser.leave_visual();
    // Heard by the view that leaves: the view that comes reads what it left.
    let child = view
        .returns
        .and_then(|_| app.browser.follow_selection_on_return());
    app.window.here.parts = parts;
    app.window.here.shown.follow(parts);
    app.window.shown.follow(parts);
    // What the view showed last before it was switched away is stale: it is not remembered.
    app.window.here.restore.set_replaying(true);
    rebuild(app, view, tx);
    app.window.here.restore.set_replaying(false);
    let opens = view.returns.is_some_and(|mirrors| mirrors(app));
    if let Some((depth, location)) = child.filter(|_| opens) {
        app.browser.show_child(depth, location);
    }
    settle(app, tx);
    for part in parts.active() {
        (part.switched)(app, tx);
    }
    true
}

fn rebuild(app: &mut App, view: &View<App>, tx: &mut Tx) {
    let parts = app.window.here.parts;
    for part in parts.active() {
        (part.customized)(app);
    }
    app.window.here.grouped = mount(&app.browser, view, preferences(app));
    let rebuilt = events::replay(&app.browser, &app.window.ids);
    for change in rebuilt.changes {
        (view.apply)(app, change, tx);
    }
    for part in parts.active() {
        (part.caught_up)(app);
    }
}

fn mount(
    browser: &Rc<Browser>,
    view: &View<App>,
    saved: &strata_desktop::settings::Preferences,
) -> bool {
    let grouped = saved.group_by_type && view.supports_type_grouping;
    browser.set_type_grouping(grouped);
    browser.set_freshness(FreshnessInput {
        scope: view.rechecks,
        watch_network_folders: saved.watch_network_folders,
        ..browser.freshness_input()
    });
    grouped
}

pub(crate) fn regroup(app: &mut App, tx: &mut Tx) {
    let Some(view) = shown_view(app) else {
        return;
    };
    settle(app, tx);
    rebuild(app, view, tx);
    settle(app, tx);
}

/// The id the components know the entry at `location` by.
pub(crate) fn id_of(app: &App, location: &Location) -> EntryId {
    app.window.ids.of(location)
}

/// The id the entry at `location` has been given, if any, without issuing one.
#[cfg(feature = "qa")]
pub(crate) fn known_id(app: &App, location: &Location) -> Option<EntryId> {
    app.window.ids.known(location)
}

pub(crate) fn leave_view_at_start(app: &App, tx: &mut Tx) -> bool {
    let Some(key) = app
        .window
        .here
        .parts
        .sidebar()
        .and_then(|sidebar| (sidebar.focus)(app))
    else {
        return false;
    };
    tx.focus(key, true);
    true
}

pub(crate) fn sidebar_held_back(app: &App) -> f32 {
    app.window
        .here
        .parts
        .sidebar()
        .map_or(0.0, |sidebar| (sidebar.held_back)(app))
}

pub(crate) fn view_focus(app: &App) -> Option<&'static str> {
    shown_view(app).map(|view| view.focus)
}

pub(crate) fn listing_viewports(app: &App) -> Vec<String> {
    match view_focus(app) {
        Some(crate::columns::VIEW) => app.columns.listing_viewports(),
        Some(key) => vec![strata_components::collection::element::viewport_path(key)],
        None => Vec::new(),
    }
}

fn on_window_action(app: &mut App, action: WindowAction, tx: &mut Tx) {
    let csd = &mut app.window.csd;
    let outcome = match action {
        WindowAction::Minimize => csd.minimize(tx),
        WindowAction::ToggleMaximize => csd.toggle_maximized(tx),
        WindowAction::Close => return close_window(app, tx),
    };
    tracing::debug!(?action, ?outcome, "window button");
}

pub(crate) fn close_window(app: &mut App, tx: &mut Tx) {
    let last = crate::host::windowed::is_last_window(window_id(app));
    if let Some(refusal) = close_guard::refusal(app, last) {
        close_guard::tell(app, refusal);
        return;
    }
    let outcome = app.window.csd.close(tx);
    tracing::debug!(?outcome, "window closed");
}

struct Button {
    key: &'static str,
    icon: fn(&App) -> IconName,
    face: fn(&App) -> Face,
    pressed_when_checked: bool,
    command: &'static str,
}

pub(crate) struct Face {
    pub(crate) name: String,
    /// The tooltip, which is also the description, where it is not the name.
    pub(crate) tooltip: Option<String>,
    /// Whether a toggle button is on; None for a plain button.
    pub(crate) checked: Option<bool>,
}

impl Face {
    fn named(name: String) -> Self {
        Self {
            name,
            tooltip: None,
            checked: None,
        }
    }
}

/// Before the location bar.
const LEADING: Button = Button {
    key: "sidebar-toggle",
    icon: |_| IconName::Sidebar,
    face: crate::sidebar::toggle_face,
    pressed_when_checked: false,
    command: "toggle-sidebar",
};

/// The key of the header's New tab, which the strip holds while it shows.
const NEW_TAB: &str = "new-tab";

/// After the location bar, in the header's order.
const TRAILING: [Button; 4] = [
    Button {
        key: NEW_TAB,
        icon: |_| IconName::Plus,
        face: |_| Face {
            tooltip: Some(with_keys(
                "New tab (%{keys})",
                Some("New tab"),
                Keys::One(TAB_NEW.shown()),
            )),
            ..Face::named(tr("New tab"))
        },
        pressed_when_checked: false,
        command: "tab-new",
    },
    Button {
        key: "search",
        icon: |_| IconName::Search,
        face: |_| {
            Face::named(with_keys(
                "Search (%{keys})",
                Some("Search"),
                Keys::One(SEARCH.shown()),
            ))
        },
        pressed_when_checked: false,
        command: "search",
    },
    Button {
        key: "appearance",
        icon: crate::appearance::button_icon,
        face: crate::appearance::face,
        pressed_when_checked: true,
        command: "appearance",
    },
    Button {
        key: "settings",
        icon: |_| IconName::Settings,
        face: |_| Face::named(tr("Settings")),
        pressed_when_checked: false,
        command: "settings",
    },
];

/// The window buttons enabled by the user's preferences (or the desktop session's default).
fn configured_window_buttons(app: &App) -> WindowButtons {
    let prefs = preferences(app);
    WindowButtons {
        minimize: prefs.window_show_minimize,
        maximize: prefs.window_show_maximize,
        close: prefs.window_show_close,
    }
}

fn window_buttons(app: &App) -> WindowButtons {
    let buttons = configured_window_buttons(app);
    WindowButtons {
        close: buttons.close && !strip_shows(app),
        ..buttons
    }
}

fn strip_shows(app: &App) -> bool {
    let strip = app.window.here.parts.strip();
    strip.is_some_and(|strip| (strip.shows)(app))
}

/// The window's Close for the strip while it shows, named as the header names it.
fn strip_close(app: &App, chrome: &WindowChrome) -> Option<El> {
    let shown = Shown::new(configured_window_buttons(app), chrome);
    if !(shown.close && strip_shows(app)) {
        return None;
    }
    let look = &app.look;
    let close = header::action(look, IconName::Close, &app.window.labels.window.close)
        .on_press_tx(|app: &mut App, tx: &mut Tx| {
            on_window_action(app, WindowAction::Close, tx);
        })
        .build(&look.accent_icon_kit());
    Some(buttons::on_return("window-close".to_owned(), close))
}

pub(crate) fn header_last(app: &App) -> String {
    if app.browser.is_chooser_mode() {
        return "header-chooser-close".to_owned();
    }
    let shown = Shown::new(window_buttons(app), &app.chrome());
    let buttons = [
        (shown.close, "close"),
        (shown.maximize, "maximize"),
        (shown.minimize, "minimize"),
    ];
    let button = buttons
        .into_iter()
        .find_map(|(shown, key)| shown.then_some(key));
    let last = button.unwrap_or(TRAILING[TRAILING.len() - 1].key);
    format!("header-{last}")
}

/// The commands the header's buttons run, in the header's order.
#[cfg(test)]
pub(crate) fn header_commands() -> impl Iterator<Item = &'static str> {
    let buttons = std::iter::once(&LEADING).chain(&TRAILING);
    buttons.map(|button| button.command)
}

fn build_header_action(app: &App, button: &Button) -> El {
    let look = &app.look;
    let command = button.command;
    let icon = (button.icon)(app);
    let face = (button.face)(app);
    let action = match &face.tooltip {
        Some(tooltip) => header::action(look, icon, tooltip).label(face.name),
        None => header::action(look, icon, &face.name),
    };
    let action = action
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            commands::run(app, command, On::Focus, tx);
        })
        .build(&look.accent_icon_kit());
    let action = match face.checked {
        Some(true) if button.pressed_when_checked => action.checked(true).fill(look.kit.c.pressed),
        Some(on) => action.checked(on),
        None => action,
    };
    buttons::on_return(format!("header-{}", button.key), action)
}

fn chooser_bar(app: &App, location: Option<El>) -> El {
    let look = &app.look;
    let mut header = Header::new("header", look);
    if !app.chooser.is_confined() {
        header = header.leading(build_header_action(app, &LEADING));
    }
    if let Some(location) = location {
        header = header.center(location);
    }
    let appearance = TRAILING
        .iter()
        .find(|button| button.key == "appearance")
        .expect("appearance action");
    let cancel_label = with_keys(
        "Cancel file selection (%{keys})",
        Some("Cancel file selection"),
        Keys::One(strata_desktop::keys::bindings::ESCAPE.shown()),
    );
    let close = buttons::on_return(
        "header-chooser-close".to_owned(),
        header::action(look, IconName::Close, &cancel_label)
            .on_press_tx(|app: &mut App, tx: &mut Tx| {
                crate::chooser::cancel(app, tx);
            })
            .build(&look.accent_icon_kit()),
    );
    header
        .action(build_header_action(app, appearance))
        .action(close)
        .build()
}

/// The header, with the location slot's element in its middle.
fn bar(app: &App, chrome: &WindowChrome, location: Option<El>) -> El {
    if app.browser.is_chooser_mode() {
        return chooser_bar(app, location);
    }
    let look = &app.look;
    let labels = &app.window.labels;
    let header = Header::new("header", look).leading(build_header_action(app, &LEADING));
    let header = match location {
        Some(location) => header.center(location),
        None => header,
    };
    // While the strip shows it holds New tab; otherwise the strip may build it as a drop target.
    let strip_new_tab = app
        .window
        .here
        .parts
        .strip()
        .and_then(|strip| strip.new_tab);
    let in_strip = strip_shows(app).then_some(NEW_TAB);
    let tenxer = preferences(app).tenxer_mode;
    let trailing = TRAILING
        .iter()
        .filter(|button| Some(button.key) != in_strip)
        .filter(|button| !(tenxer && button.key == "search"));
    let header = trailing.fold(header, |header, button| {
        let built = (button.key == NEW_TAB)
            .then(|| strip_new_tab.map(|new_tab| new_tab(app, "header-new-tab")))
            .flatten()
            .unwrap_or_else(|| build_header_action(app, button));
        header.action(built)
    });
    header
        .window_buttons(
            window_buttons(app),
            chrome,
            &labels.window,
            on_window_action,
        )
        .build()
}

/// What the registered parts built for one run of the view, slot by slot.
struct Placed {
    tabs: Option<El>,
    location: Option<El>,
    sidebar: Option<El>,
    /// The shown view, inside the drawer's split when there is a drawer.
    files: Option<El>,
    footer: Vec<El>,
    /// What the overlays and the dialogs leave in the tree, their surfaces being in the layer.
    anchors: Vec<El>,
}

/// What the parts of `slot` build, in the order they are registered.
fn built(app: &App, slot: Slot, frame: &mut Frame<'_, App>) -> Vec<El> {
    let builds = app.window.here.parts.of(slot);
    let builds = builds.filter_map(|part| part.place.build());
    builds.filter_map(|build| build(app, frame)).collect()
}

impl Placed {
    fn build(app: &App, chrome: &WindowChrome, frame: &mut Frame<'_, App>) -> Self {
        let close = strip_close(app, chrome);
        let strip = app.window.here.parts.strip();
        let tabs = strip.and_then(|strip| (strip.build)(app, close, frame));
        let location = built(app, Slot::Location, frame).pop();
        let sidebar = built(app, Slot::Sidebar, frame).pop();
        let mut files = built(app, Slot::View, frame).pop();
        if let Some(wrap) = app.window.here.parts.drawer() {
            files = files.map(|files| wrap(app, files, frame));
        }
        let footer = built(app, Slot::Footer, frame);
        let mut anchors = built(app, Slot::Overlays, frame);
        anchors.extend(built(app, Slot::Dialogs, frame));
        Self {
            tabs,
            location,
            sidebar,
            files,
            footer,
            anchors,
        }
    }
}

/// Under the header: the sidebar, then the files.
fn body(app: &App, sidebar: Option<El>, files: Option<El>) -> El {
    let body = col()
        .key("body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .on_pointer_move_local(|app: &mut App, pointer, tx: &mut Tx| {
            if let Some(pointer_at) = shown_view(app).and_then(|view| view.pointer_at) {
                pointer_at(app, (pointer.window_x, pointer.window_y));
            }
            tx.live_only();
        });
    let mut files_col = col()
        .key("body-files")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0);
    if files.is_none() {
        files_col = files_col.fill(app.look.kit.c.bg_base);
    }
    let files = files_col.children(files);
    let beside = row()
        .key("body-beside")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .children(sidebar)
        .child(files);
    body.child(beside)
}

pub(crate) fn view(app: &App, cx: &mut Cx<App>) -> El {
    // A label drawn in this run reads the map and type-to-search as they are now.
    live_keys(app);
    let look = &app.look;
    let mut layer = Vec::new();
    let frame = &mut Frame {
        cx,
        layer: &mut layer,
    };
    let reduced = !look.animations || frame.cx.system_reduced_motion();
    frame.cx.reduced_motion(reduced);
    frame.cx.help_look(look.help_look());
    let chrome = app.chrome();
    let placed = Placed::build(app, &chrome, frame);
    let bar = csd::header(bar(app, &chrome, placed.location), &app.window.csd, csd_of);
    let tab = col()
        .key(format!("tab-{}", app.window.here.key.0))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(bar)
        .child(body(app, placed.sidebar, placed.files))
        .children(placed.footer)
        .children(placed.anchors);
    let content = col()
        .w_full()
        .h_full()
        .children(placed.tabs)
        .child(tab)
        .children(crate::dnd::keep_alive(app));
    let window = csd::decorate(
        header::frame("window-frame", content, look, &chrome),
        &app.window.csd,
        csd_of,
    );
    rune_kit::root(window, layer)
        .on_key(commands::from_element)
        .on_layout(record_window_size)
        // The window's zoom wheel, for what no scroll view holds (`appearance::wheel`).
        .on_wheel(crate::appearance::wheel::heard)
        .on_window_focus(|app: &mut App, focused, tx: &mut Tx| {
            let focus_changed = app.window.active != focused;
            app.window.active = focused;
            presence::follow(app);
            if focus_changed && transparency(app).varies_with_focus() {
                follow_look_tx(app, tx);
            }
            if !focused && app.window.held != rune::Mods::default() {
                modifiers(app, rune::Mods::default(), tx);
            }
            location::window_focused(app, focused, tx);
        })
}

fn record_window_size(app: &mut App, [_, _, width, height]: [f32; 4]) {
    app.window.size = (width, height);
}

pub(crate) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let mut ran = settle(app, tx);
    folder_views::poll(app, clock, tx);
    let parts = app.window.here.parts;
    for part in parts.active() {
        ran |= (part.poll)(app, clock, tx);
    }
    if !app.window.events.is_empty() {
        ran |= settle(app, tx);
    }
    grab::follow(app);
    ran
}

pub(crate) fn publish(app: &mut App) {
    let parts = app.window.here.parts;
    for part in parts.active() {
        (part.publish)(app);
    }
}

pub(crate) fn settle(app: &mut App, tx: &mut Tx) -> bool {
    publish(app);
    let changed = apply_queued(app, tx);
    publish(app);
    changed
}

fn apply_queued(app: &mut App, tx: &mut Tx) -> bool {
    let window = &mut app.window;
    if window.events.is_empty() {
        return false;
    }
    let (heard, copied) = (window.events.heard(), window.events.copied());
    tracing::trace!(
        events = heard - window.applied.0,
        rows = copied - window.applied.1,
        "browser events reached the interface"
    );
    window.applied = (heard, copied);
    let mut changed = false;
    while let Some(queued) = app.window.events.pop() {
        match queued.tab {
            Some(tab) if tab != app.window.here.key => {
                changed |= app.window.tabs.hold(tab, queued);
            }
            _ => changed |= (queued.update)(app, tx),
        }
    }
    tabs::session::observe_locations(app);
    if std::mem::take(&mut app.window.customizations_dirty) {
        app.window.settings.commit();
        refresh_worn(app);
        changed = true;
    }
    let parts = app.window.here.parts;
    for part in parts.active() {
        (part.caught_up)(app);
    }
    changed
}

pub(crate) fn parts(app: &App) -> Registry<'static, App> {
    app.window.here.parts
}

/// Whether any active part holds a modal layer (`TabWindow::blocked`).
pub(crate) fn modal_open(app: &App) -> bool {
    parts(app)
        .active()
        .any(|part| matches!((part.keyboard)(app), Some(Holds::Modal { .. })))
}

fn first_window_reports() -> Option<std::path::PathBuf> {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let first = !REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed);
    first.then(strata_desktop::settings::settings_path)
}

pub(crate) fn live_keys(app: &App) -> &Live {
    let live = &app.window.keys;
    live.follow(app.window.settings.preferences());
    live
}

mod grab;
#[cfg(test)]
mod tests;
