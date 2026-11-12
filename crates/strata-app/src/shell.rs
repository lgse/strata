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
    self, Header, HeaderLabels, WindowAction, WindowButtons, WindowChrome,
};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::settings::sort_preferences;
use strata_fs::{
    adapters::{LocalFileSource, LocalOperationProvider},
    app::{Browser, BrowserEvent},
};
use strata_kit::{icons::IconName, look::Look};
use strata_platform::csd::{self, Csd};

use crate::{
    App, Built, PARTS,
    commands::{self, Live, On},
    settings::Saved,
    slots::{Entry, Frame, Heard, Ids, Inbox, Registry, Setup, Slot, View},
    startup::UiThread,
};

/// The header's accessible names, translated once: the language changes only on restart.
struct Labels {
    window: HeaderLabels,
    toggle_sidebar: String,
    new_tab: String,
    search: String,
    appearance: String,
    settings: String,
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
            toggle_sidebar: tr("Toggle sidebar"),
            new_tab: tr("New tab"),
            search: tr("Search"),
            appearance: tr("Appearance"),
            settings: tr("Settings"),
        }
    }
}

/// What the window keeps for itself, beside the parts' state.
pub(crate) struct Window {
    settings: Saved,
    events: Inbox<App>,
    /// The registered parts, with the view that is shown.
    parts: Registry<'static, App>,
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
        let browser = Browser::with_preferences(Rc::new(LocalFileSource), sort_preferences(saved));
        browser.set_operation_provider(Rc::new(LocalOperationProvider));
        let ids = Ids::default();
        let setup = Setup {
            settings: saved,
            ids: &ids,
            later: &later,
        };
        debug_assert_eq!(Registry::check(parts, commands::own()), Ok(()));
        let parts = Registry::new(parts, &saved.browser_mode);
        // Before the first load, so that its cursor lands on the first row a grouped view
        // shows (`rebuild.rs` in `src/ui/browser_modes`). The one place a view is mounted.
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
        let keys = Live::registered(saved);
        let window = Window {
            settings,
            events,
            parts,
            ids,
            location: None,
            title: title_for(None),
            csd,
            active: true,
            keys,
            labels: Labels::translated(),
            applied: (0, 0),
        };
        Self::assemble(Look::fallback(true), browser, window, built)
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

/// The view that is shown, as the parts that work in it see it: the rename editor, a context
/// menu, the QA backend.
pub(crate) fn shown_view(app: &App) -> Option<&'static View<App>> {
    app.window.parts.shown()
}

/// The id the components know the entry at `location` by.
pub(crate) fn id_of(app: &App, location: &Location) -> EntryId {
    app.window.ids.of(location)
}

/// The keyboard leaves the shown view at its start, as Left in the first column asks: the
/// sidebar takes it. False when there is no sidebar, and the keyboard stays where it is.
pub(crate) fn leave_view_at_start(app: &App, tx: &mut Tx) -> bool {
    let Some(sidebar) = app.window.parts.sidebar() else {
        return false;
    };
    tx.focus(sidebar.focus, true);
    true
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

/// One of the header's five actions: the key its element ends with, its icon, its name, and
/// the command a press runs (`commands::run`). What the command does is whichever part
/// registers it; until one does the button is drawn and inert.
struct Button {
    key: &'static str,
    icon: IconName,
    label: fn(&Labels) -> &str,
    command: &'static str,
}

/// Before the location bar.
const LEADING: Button = Button {
    key: "sidebar-toggle",
    icon: IconName::Sidebar,
    label: |labels| &labels.toggle_sidebar,
    command: "toggle-sidebar",
};

/// After the location bar, in the header's order.
const TRAILING: [Button; 4] = [
    Button {
        key: "new-tab",
        icon: IconName::Plus,
        label: |labels| &labels.new_tab,
        command: "tab-new",
    },
    Button {
        key: "search",
        icon: IconName::Search,
        label: |labels| &labels.search,
        command: "search",
    },
    Button {
        key: "appearance",
        icon: IconName::Menu,
        label: |labels| &labels.appearance,
        command: "appearance",
    },
    Button {
        key: "settings",
        icon: IconName::Settings,
        label: |labels| &labels.settings,
        command: "settings",
    },
];

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
        header::action(look, button.icon, (button.label)(labels))
            .on_press_tx(move |app: &mut App, tx: &mut Tx| {
                commands::run(app, command, On::Focus, tx);
            })
            .build(&look.kit)
            .key(format!("header-{}", button.key))
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
        .window_buttons(
            WindowButtons::default(),
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
    match sidebar {
        Some(sidebar) => {
            let beside = row().w_full().grow(1.0).min_h(0.0).child(sidebar);
            body.child(beside.children(files))
        }
        None => body.children(files),
    }
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
        .on_window_focus(|app: &mut App, focused, _: &mut Tx| {
            app.window.active = focused;
        })
}

/// Rune's `poll` hook: what the browser's observers queued, then each part's own timers,
/// then whatever those queued: a part's poll may ask a service that answers at once, and
/// nothing is left in the queue for a frame that may not come.
pub(crate) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let mut ran = settle(app, tx);
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

/// What the key catalog reads of the settings, brought up to the saved values as they are
/// now: the GTK application reads its preference manager at each press and each label.
pub(crate) fn live_keys(app: &App) -> &Live {
    let live = &app.window.keys;
    live.follow(app.window.settings.preferences());
    live
}

#[cfg(test)]
mod tests;
