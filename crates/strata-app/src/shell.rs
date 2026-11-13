// SPDX-License-Identifier: MIT

//! The window: Strata's header over the parts registered in its slots, and what the window
//! keeps for itself.
//!
//! The header is the one strata-components' X1 demo draws
//! (`crates/strata-gallery/src/scenes/x1/shell.rs`). Each of its five actions runs a command
//! of the registry ([`crate::commands`]) and is inert until a part registers that command;
//! the window buttons go to the window manager. The window's keys are the registry's too.
//! Everything else is a part: [`crate::slots`] says where each slot is and what a part gets
//! from the shell.

use std::rc::Rc;

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
    settings::sort_preferences,
};
use strata_fs::{
    adapters::{LocalFileSource, LocalOperationProvider},
    app::{Browser, BrowserEvent},
};
use strata_kit::{
    icons::IconName,
    look::{Look, LookParams},
};
use strata_platform::csd::{self, Csd};

use crate::{
    App, Built, PARTS, buttons,
    commands::{self, Live, On},
    events, folder_views,
    location::{self, labels::with_keys},
    settings::Saved,
    slots::{Entry, Frame, Heard, Ids, Inbox, Registry, Setup, ShownView, Slot, View},
    startup::UiThread,
};

/// The window buttons' accessible names, translated once: the language changes only on
/// restart. The header's actions name their keys, so they are named at each run of the view.
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
    settings: Saved,
    events: Inbox<App>,
    /// The registered parts, with the view that is shown.
    parts: Registry<'static, App>,
    /// Which view is shown, as the listeners and the parts' timers read it.
    shown: ShownView,
    /// The table that names an entry for the components: every part holds the same one.
    ids: Ids,
    /// The browser's active location as of the last `poll`: what the title names.
    location: Option<Location>,
    title: String,
    csd: Csd,
    /// Whether the window has keyboard focus.
    active: bool,
    /// What the key catalog reads of the settings at each press and label.
    keys: Rc<Live>,
    labels: Labels,
    /// How many of the browser's events, and of the rows they copied, `apply_queued` has
    /// seen, for its trace line.
    applied: (u64, u64),
    /// The window's decoded thumbnails, which every view that draws them reads, each through
    /// a viewer of its own ([`crate::thumbnails`] has the render pool, one per process).
    thumbnails: ThumbnailStore,
    /// The per-folder views, one store for the process ([`crate::folder_views`]).
    folder_views: folder_views::Kept,
}

/// The window's own share of an event: the columns or the active one changed, so the window
/// may show another location. It listens after the parts.
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
}

impl App {
    /// A browser over the local file source, as `src/ui/window/launch.rs` builds one, with
    /// the saved sort and hidden-files choice, every registered part, and every event queued
    /// for [`apply_queued`].
    pub(crate) fn new(ui: &UiThread, csd: Csd, settings: Saved) -> Self {
        Self::over(PARTS, ui, csd, settings)
    }

    /// The window over the table `parts`: the registered parts, or a test's made-up ones. The
    /// state of every registered part is built either way, and only the table's hooks run.
    pub(crate) fn over(
        parts: &'static [Entry<App>],
        _ui: &UiThread,
        csd: Csd,
        mut settings: Saved,
    ) -> Self {
        let events = Inbox::new();
        let later = events.later();
        settings.report_to(later.clone());
        let saved = settings.preferences();
        // The saved date format, which the dates every view shows read
        // (`src/ui/preferences.rs` sets it as it loads); a live change must set it again.
        strata_core::util::set_date_format(strata_core::util::DateFormat::parse(
            &saved.date_format,
        ));
        let browser = Browser::with_preferences(Rc::new(LocalFileSource), sort_preferences(saved));
        browser.set_operation_provider(Rc::new(LocalOperationProvider));
        let ids = Ids::default();
        debug_assert_eq!(Registry::check(parts, commands::own()), Ok(()));
        let parts = Registry::new(parts, &saved.browser_mode);
        let shown = parts.shown_now();
        let setup = Setup {
            settings: saved,
            ids: &ids,
            later: &later,
            shown: &shown,
        };
        // Before the first load, so that its cursor lands on the first row a grouped view
        // shows (`rebuild.rs` in `src/ui/browser_modes`). A view shown later is mounted by
        // `show_view`, which does the same.
        let groups = parts
            .shown()
            .is_some_and(|view| view.supports_type_grouping);
        browser.set_type_grouping(saved.group_by_type && groups);
        let mut listeners = parts.listeners(&setup);
        listeners.push(Box::new(hear));
        events.observe(&browser, listeners);
        let queue = setup.later.clone();
        // The browser's sort or hidden-files choice changed: the saved settings follow it.
        browser.observe_preferences(move |view| {
            queue.then(move |app: &mut App, _: &mut Tx| {
                app.window.settings.store_view(view);
                false
            });
        });
        let built = Built::new(&setup);
        let keys = Live::registered(saved, first_window_reports().as_deref());
        let thumbnails = thumbnail_store(saved);
        let folder_views = folder_views::Kept::load(later.clone());
        let mut look = Look::fallback(true);
        follow_saved_look(&mut look, saved);
        let window = Window {
            settings,
            events,
            parts,
            shown,
            ids,
            location: None,
            title: title_for(None),
            csd,
            active: true,
            keys,
            labels: Labels::translated(),
            applied: (0, 0),
            thumbnails,
            folder_views,
        };
        Self::assemble(look, browser, window, built)
    }

    pub(crate) fn open(&self, location: Location) {
        self.browser.navigate(location);
    }

    /// Names the window for the browser's active location. False when it has not changed.
    fn retitle(&mut self, tx: &mut Tx) -> bool {
        let location = self.browser.active_location();
        if location == self.window.location {
            return false;
        }
        self.window.title = title_for(location.as_ref());
        tx.set_title(self.window.title.clone());
        self.window.location = location;
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

/// The saved settings as they are now, and the way to change one:
/// `shell::saved(app).change(|settings| …)` changes the values and queues the save of what
/// changed. Call it from a handler or a queued update, never from a listener. A part calls
/// `change` and `preferences` and nothing else of [`Saved`].
///
/// The save's result is heard when GLib is next pumped and the queue applied. After a change
/// made in a handler that is the `poll` that follows. After one made in a queued update it
/// is the next pump, whenever that comes: the update ran after its pump had asked GLib what
/// to wait for. Nothing is lost on disk; until the result is heard the keys count as
/// unsaved, and a change made meanwhile writes them again.
pub(crate) fn saved(app: &mut App) -> &mut Saved {
    &mut app.window.settings
}

/// Sets the look's tokens from what of the saved settings this interface follows: the text
/// size (`TextSize`, 8 to 48 px). The rest of [`LookParams`] stays at Strata's defaults until
/// its settings are followed (density, roundness, the font, the desktop's text scale), and
/// the glow and animations switches stay as the look has them, which a host may have turned
/// off (`Offscreen::without_animations`).
fn follow_saved_look(look: &mut Look, saved: &strata_desktop::settings::Preferences) {
    let text_size = u16::try_from(saved.text_size.root_font_px()).unwrap_or(u16::MAX);
    look.apply_params(&LookParams {
        text_size,
        element_glow: look.glow.on,
        animations: look.animations,
        ..LookParams::default()
    });
}

/// The look follows the saved settings as they are now: a part that changes one the look
/// reads, the text size, calls this after `saved(app).change(…)`, as GTK's theme follows its
/// preference (`src/ui/theme.rs`).
pub(crate) fn follow_look(app: &mut App) {
    let App { look, window, .. } = app;
    follow_saved_look(look, window.settings.preferences());
}

/// The saved settings as they are now, for a build or a reader that does not change them.
pub(crate) fn preferences(app: &App) -> &strata_desktop::settings::Preferences {
    app.window.settings.preferences()
}

/// The GTK window's rule (`src/ui/window/composition/tabs.rs`): the folder, then the name.
fn title_for(location: Option<&Location>) -> String {
    location.map_or_else(
        || "Strata".to_owned(),
        |location| format!("{} — Strata", location.display_name()),
    )
}

pub(crate) fn csd_of(app: &mut App) -> &mut Csd {
    &mut app.window.csd
}

/// The store and the pool as the saved settings set them: thumbnails on or off, and the
/// number of workers (`thumbnail_workers`, which GTK hands the pool the same way).
fn thumbnail_store(saved: &strata_desktop::settings::Preferences) -> ThumbnailStore {
    strata_preview::sandbox::browser::set_worker_limit(saved.thumbnail_workers);
    let mut store = ThumbnailStore::new(Settings::default());
    let _ = store.set_enabled(saved.show_thumbnails);
    store
}

impl Window {
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
}

/// The window's thumbnails, as a view's build reads them.
pub(crate) fn thumbnails(app: &App) -> &ThumbnailStore {
    &app.window.thumbnails
}

/// The view that is shown, as the parts that work in it see it: the rename editor, a context
/// menu, the QA backend.
pub(crate) fn shown_view(app: &App) -> Option<&'static View<App>> {
    app.window.parts.shown()
}

/// Shows the view that answers to `mode` in place of the one shown, while the window is open,
/// as GTK's `set_view_mode` switches (`src/ui/browser/view_modes.rs`). False when that view
/// is shown already, or none answers to it.
///
/// - What the view that leaves changed goes to the browser, and a visual range ends
///   (`leave_visual`).
/// - A view whose child column follows the selection ([`View::returns`]) is shown again as
///   GTK's `rebuild_columns_view` does: a child the selection no longer holds closes, before
///   the rebuild, and the lone selected folder opens as the child after it, while the view
///   mirrors.
/// - The browser groups by file type for the view now shown, as at start.
/// - The view is rebuilt from the browser ([`events::replay`]) through its own
///   [`View::apply`], and hears the browser from then on. The rebuild gives it the keyboard,
///   as a listing it is given does.
/// - Every active part follows the view now shown ([`Part::switched`](crate::slots::Part)):
///   the location bar names the folder it shows.
///
/// The caller saves the mode and ends what is open across the switch, the rename editor.
pub(crate) fn show_view(app: &mut App, mode: &str, tx: &mut Tx) -> bool {
    let mut parts = app.window.parts;
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
    app.window.parts = parts;
    app.window.shown.follow(parts);
    rebuild(app, view, tx);
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

/// The browser groups by file type for `view` as the saved setting asks, and `view` is
/// rebuilt from the browser ([`events::replay`]) through its own [`View::apply`].
fn rebuild(app: &mut App, view: &View<App>, tx: &mut Tx) {
    let grouping = preferences(app).group_by_type && view.supports_type_grouping;
    app.browser.set_type_grouping(grouping);
    let rebuilt = events::replay(&app.browser, &app.window.ids);
    for change in rebuilt.changes {
        (view.apply)(app, change, tx);
    }
}

/// The saved "Group by file type" changed: the view shown follows it, as GTK's List view
/// rebuilds its pane when the setting changes (`sync_type_grouping` in
/// `src/ui/browser_modes/rebuild.rs`). The browser groups for it, and it is rebuilt from the
/// browser.
pub(crate) fn regroup(app: &mut App, tx: &mut Tx) {
    let Some(view) = shown_view(app) else {
        return;
    };
    // The view shown goes on hearing the browser here, unlike in `show_view`: what is queued
    // lands first, so none of it lands again over the rebuild.
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

/// The keyboard leaves the shown view at its start, as Left in the first column asks: the
/// sidebar takes it. False when there is no sidebar, or none that can take it, and the
/// keyboard stays where it is.
pub(crate) fn leave_view_at_start(app: &App, tx: &mut Tx) -> bool {
    let Some(key) = app
        .window
        .parts
        .sidebar()
        .and_then(|sidebar| (sidebar.focus)(app))
    else {
        return false;
    };
    tx.focus(key, true);
    true
}

/// How much of the files' area the sidebar holds back beyond what it draws: its full width
/// less its rail's while it is railed ([`crate::slots::Sidebar::held_back`]), and nothing with
/// no sidebar.
pub(crate) fn sidebar_held_back(app: &App) -> f32 {
    app.window
        .parts
        .sidebar()
        .map_or(0.0, |sidebar| (sidebar.held_back)(app))
}

/// The key of the shown view's element that takes the keyboard: where a field or a menu
/// gives it back.
pub(crate) fn view_focus(app: &App) -> Option<&'static str> {
    shown_view(app).map(|view| view.focus)
}

fn on_window_action(app: &mut App, action: WindowAction, tx: &mut Tx) {
    let csd = &mut app.window.csd;
    let outcome = match action {
        WindowAction::Minimize => csd.minimize(tx),
        WindowAction::ToggleMaximize => csd.toggle_maximized(tx),
        WindowAction::Close => csd.close(tx),
    };
    tracing::debug!(?action, ?outcome, "window button");
}

/// One of the header's five actions: the key its element ends with, its icon, its face, and
/// the command a press runs (`commands::run`). What the command does is whichever part
/// registers it; until one does the button is drawn and inert.
struct Button {
    key: &'static str,
    icon: IconName,
    face: fn(&App) -> Face,
    command: &'static str,
}

/// What a header action shows assistive technology, with its keys as bound now: GTK's names
/// (`Header::new` in `src/ui/window/composition/layout.rs`).
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
    icon: IconName::Sidebar,
    face: crate::sidebar::toggle_face,
    command: "toggle-sidebar",
};

/// After the location bar, in the header's order.
const TRAILING: [Button; 4] = [
    Button {
        key: "new-tab",
        icon: IconName::Plus,
        // GTK names it plainly; only its tooltip names the keys.
        face: |_| Face {
            tooltip: Some(with_keys(
                "New tab (%{keys})",
                Some("New tab"),
                Keys::One(TAB_NEW.shown()),
            )),
            ..Face::named(tr("New tab"))
        },
        command: "tab-new",
    },
    Button {
        key: "search",
        icon: IconName::Search,
        face: |_| {
            Face::named(with_keys(
                "Search (%{keys})",
                Some("Search"),
                Keys::One(SEARCH.shown()),
            ))
        },
        command: "search",
    },
    Button {
        key: "appearance",
        icon: IconName::Menu,
        face: crate::appearance::face,
        command: "appearance",
    },
    Button {
        key: "settings",
        icon: IconName::Settings,
        face: |_| Face::named(tr("Settings")),
        command: "settings",
    },
];

/// The window buttons the header shows, of those the window manager can work.
fn window_buttons() -> WindowButtons {
    WindowButtons::default()
}

/// The key of the header's last element, after which GTK's `child_focus(TabForward)` in the
/// header finds nothing (`TopBarNavigation::move_focus`): the last window button shown, else
/// the last action.
pub(crate) fn header_last(app: &App) -> String {
    let shown = Shown::new(window_buttons(), &app.chrome());
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

/// The header, with the location slot's element in its middle.
fn bar(app: &App, chrome: &WindowChrome, location: Option<El>) -> El {
    let look = &app.look;
    let labels = &app.window.labels;
    let action = |button: &Button| {
        let command = button.command;
        let face = (button.face)(app);
        let action = match &face.tooltip {
            Some(tooltip) => header::action(look, button.icon, tooltip).label(face.name),
            None => header::action(look, button.icon, &face.name),
        };
        let action = action
            .on_press_tx(move |app: &mut App, tx: &mut Tx| {
                commands::run(app, command, On::Focus, tx);
            })
            .build(&look.kit);
        let action = match face.checked {
            Some(on) => action.checked(on),
            None => action,
        };
        buttons::on_return(format!("header-{}", button.key), action)
    };
    let header = Header::new("header", look).leading(action(&LEADING));
    let header = match location {
        Some(location) => header.center(location),
        None => header,
    };
    let header = TRAILING
        .iter()
        .fold(header, |header, button| header.action(action(button)));
    header
        .window_buttons(window_buttons(), chrome, &labels.window, on_window_action)
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
    let builds = app.window.parts.of(slot);
    let builds = builds.filter_map(|part| part.place.build());
    builds.filter_map(|build| build(app, frame)).collect()
}

impl Placed {
    /// Asks every active part for its element, in the order [`Slot`] declares the slots and,
    /// within one, in the order the parts are registered: the layer is filled in that order.
    fn build(app: &App, frame: &mut Frame<'_, App>) -> Self {
        let tabs = built(app, Slot::Tabs, frame).pop();
        let location = built(app, Slot::Location, frame).pop();
        let sidebar = built(app, Slot::Sidebar, frame).pop();
        let mut files = built(app, Slot::View, frame).pop();
        if let Some(wrap) = app.window.parts.drawer() {
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
        .fill(app.look.kit.c.bg_base)
        .on_pointer_move_local(|app: &mut App, pointer, tx: &mut Tx| {
            if let Some(pointer_at) = shown_view(app).and_then(|view| view.pointer_at) {
                pointer_at(app, (pointer.window_x, pointer.window_y));
            }
            tx.live_only();
        });
    // The same keyed wrappers with a sidebar or without one, so the view keeps its place in
    // the tree when the sidebar comes or goes: a view whose path changed would be mounted
    // anew, and lose its scroll and focus. Beside the sidebar the files take what it leaves.
    let files = col()
        .key("body-files")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .children(files);
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
    let chrome = app.chrome();
    let placed = Placed::build(app, frame);
    let bar = csd::header(bar(app, &chrome, placed.location), &app.window.csd, csd_of);
    let content = col()
        .w_full()
        .h_full()
        .children(placed.tabs)
        .child(bar)
        .child(body(app, placed.sidebar, placed.files))
        .children(placed.footer)
        .children(placed.anchors);
    let window = csd::decorate(
        header::frame("window-frame", content, look, &chrome),
        &app.window.csd,
        csd_of,
    );
    rune_kit::root(window, layer)
        // What the focused element and those around it left of a key, ahead of Rune's own
        // focus navigation. The root takes no focus and no press for it.
        .on_key(commands::from_element)
        .on_window_focus(|app: &mut App, focused, tx: &mut Tx| {
            app.window.active = focused;
            location::window_focused(app, focused, tx);
        })
}

/// Rune's `poll` hook: what the browser's observers queued, then each part's own timers,
/// then whatever those queued: a part's poll may ask a service that answers at once, and
/// nothing is left in the queue for a frame that may not come.
pub(crate) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let mut ran = settle(app, tx);
    folder_views::poll(app, clock, tx);
    let parts = app.window.parts;
    for part in parts.active() {
        ran |= (part.poll)(app, clock, tx);
    }
    if !app.window.events.is_empty() {
        ran |= settle(app, tx);
    }
    ran
}

/// Hands the browser what the parts changed. A host calls it before it lets GLib dispatch, so
/// what the browser's events say of its cursor and selection is never older than the view's.
pub(crate) fn publish(app: &mut App) {
    let parts = app.window.parts;
    for part in parts.active() {
        (part.publish)(app);
    }
}

/// Brings the two sides in step: what the parts changed goes to the browser, what the
/// browser's observers queued goes to the parts, and what that left a part to say (a cursor
/// the view was waiting to place) goes to the browser. Returns whether the view has to run.
pub(crate) fn settle(app: &mut App, tx: &mut Tx) -> bool {
    publish(app);
    let changed = apply_queued(app, tx);
    publish(app);
    changed
}

/// Applies what the browser's observers queued while GLib dispatched, or while a handler
/// called the browser.
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
    while let Some(update) = app.window.events.pop() {
        changed |= update(app, tx);
    }
    let parts = app.window.parts;
    for part in parts.active() {
        (part.caught_up)(app);
    }
    changed
}

/// The registered parts, with the view that is shown: what the command registry reads each
/// part's commands and its hold on the keyboard from.
pub(crate) fn parts(app: &App) -> Registry<'static, App> {
    app.window.parts
}

/// The settings file to log a saved keymap's unusable entries against, for the process's
/// first window only: GTK logs them once, when its preference manager first builds the
/// tables.
fn first_window_reports() -> Option<std::path::PathBuf> {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let first = !REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed);
    first.then(strata_desktop::settings::settings_path)
}

/// What the key catalog reads of the settings, brought up to the saved values as they are
/// now: the GTK application reads its preference manager at each press and each label.
pub(crate) fn live_keys(app: &App) -> &Live {
    let live = &app.window.keys;
    live.follow(app.window.settings.preferences());
    live
}

#[cfg(test)]
mod tests;
