// SPDX-License-Identifier: MIT

//! The window: Strata's header over the folder it shows, and the state behind them.
//!
//! The header is the one strata-components' X1 demo draws
//! (`crates/strata-gallery/src/scenes/x1/shell.rs`). Its actions do nothing yet; the window
//! buttons go to the window manager. The body names the folder until the Columns view lands.

use std::rc::Rc;

use rune::{Cx, El, Tx, col, text};
use strata_components::header::{
    self, Header, HeaderLabels, WindowAction, WindowButtons, WindowChrome,
};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::settings::{Preferences, sort_preferences};
use strata_fs::{
    adapters::{LocalFileSource, LocalOperationProvider},
    app::Browser,
};
use strata_kit::{icons::IconName, look::Look};
use strata_platform::csd::{self, Csd};

use crate::{
    events::{Change, Inbox},
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

pub(crate) struct App {
    look: Look,
    browser: Rc<Browser>,
    events: Inbox,
    /// The browser's active location as of the last `poll`: what the body and the title name.
    location: Option<Location>,
    title: String,
    csd: Csd,
    /// Whether the window has keyboard focus.
    active: bool,
    labels: Labels,
    /// How many of the browser's events `apply_queued` has seen, for its trace line.
    applied: u64,
}

impl App {
    /// A browser over the local file source, as `src/ui/window/launch.rs` builds one, with
    /// the saved sort and hidden-files choice and with every event queued for
    /// [`apply_queued`].
    pub(crate) fn new(_ui: &UiThread, csd: Csd, settings: &Preferences) -> Self {
        let browser =
            Browser::with_preferences(Rc::new(LocalFileSource), sort_preferences(settings));
        browser.set_operation_provider(Rc::new(LocalOperationProvider));
        Self {
            look: Look::fallback(true),
            events: Inbox::observing(&browser),
            browser,
            location: None,
            title: title_for(None),
            csd,
            active: true,
            labels: Labels::translated(),
            applied: 0,
        }
    }

    pub(crate) fn open(&self, location: Location) {
        self.browser.navigate(location);
    }

    fn apply(&mut self, change: Change, tx: &mut Tx) -> bool {
        match change {
            Change::Path => {
                let location = self.browser.active_location();
                if location == self.location {
                    return false;
                }
                self.title = title_for(location.as_ref());
                tx.set_title(self.title.clone());
                self.location = location;
                true
            }
        }
    }

    fn chrome(&self) -> WindowChrome {
        let state = self.csd.state();
        let capabilities = self.csd.capabilities();
        WindowChrome {
            maximized: state.maximized,
            fullscreen: state.fullscreen,
            can_minimize: capabilities.minimize,
            can_maximize: capabilities.maximize,
            active: self.active,
        }
    }
}

/// What the offscreen host and its tests read.
#[cfg(test)]
impl App {
    pub(crate) fn browser(&self) -> &Rc<Browser> {
        &self.browser
    }

    pub(crate) fn location(&self) -> Option<&Location> {
        self.location.as_ref()
    }

    /// The window title this state last asked for.
    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    /// Whether the browser has sent something `poll` has not applied.
    pub(crate) fn has_queued_changes(&self) -> bool {
        !self.events.is_empty()
    }
}

/// The GTK window's rule (`src/ui/window/composition/tabs.rs`): the folder, then the name.
fn title_for(location: Option<&Location>) -> String {
    location.map_or_else(
        || "Strata".to_owned(),
        |location| format!("{} — Strata", location.display_name()),
    )
}

pub(crate) fn csd_of(app: &mut App) -> &mut Csd {
    &mut app.csd
}

fn on_window_action(app: &mut App, action: WindowAction, tx: &mut Tx) {
    let outcome = match action {
        WindowAction::Minimize => app.csd.minimize(tx),
        WindowAction::ToggleMaximize => app.csd.toggle_maximized(tx),
        WindowAction::Close => app.csd.close(tx),
    };
    tracing::debug!(?action, ?outcome, "window button");
}

fn bar(app: &App, chrome: &WindowChrome) -> El {
    let look = &app.look;
    let labels = &app.labels;
    let action = |part: &str, icon: IconName, label: &str| {
        header::action(look, icon, label)
            .build(&look.kit)
            .key(format!("header-{part}"))
    };
    Header::new("header", look)
        .leading(action(
            "sidebar-toggle",
            IconName::Sidebar,
            &labels.toggle_sidebar,
        ))
        .action(action("new-tab", IconName::Plus, &labels.new_tab))
        .action(action("search", IconName::Search, &labels.search))
        .action(action("appearance", IconName::Menu, &labels.appearance))
        .action(action("settings", IconName::Settings, &labels.settings))
        .window_buttons(
            WindowButtons::default(),
            chrome,
            &labels.window,
            on_window_action,
        )
        .build()
}

fn body(app: &App) -> El {
    let look = &app.look;
    let folder = col()
        .key("body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .fill(look.kit.c.bg_base)
        .items_center()
        .justify_center();
    match &app.location {
        Some(location) => folder.child(
            text(location.display_name())
                .key("body-folder")
                .font(look.kit.ty.ui_medium)
                .color(look.kit.c.text_base)
                .max_lines(1),
        ),
        None => folder,
    }
}

pub(crate) fn view(app: &App, cx: &mut Cx<App>) -> El {
    let look = &app.look;
    let reduced = !look.animations || cx.system_reduced_motion();
    cx.reduced_motion(reduced);
    let chrome = app.chrome();
    let bar = csd::header(bar(app, &chrome), &app.csd, csd_of);
    let window = csd::decorate(
        header::frame(
            "window-frame",
            col().w_full().h_full().child(bar).child(body(app)),
            look,
            &chrome,
        ),
        &app.csd,
        csd_of,
    );
    rune_kit::root(window, []).on_window_focus(|app: &mut App, focused, _: &mut Tx| {
        app.active = focused;
    })
}

/// Rune's `poll` hook, where a host has nothing to do before it.
#[cfg(test)]
pub(crate) fn poll(app: &mut App, _clock: f64, tx: &mut Tx) -> bool {
    apply_queued(app, tx)
}

/// Applies what the browser's observers queued while GLib dispatched. Returns whether the
/// view has to run.
pub(crate) fn apply_queued(app: &mut App, tx: &mut Tx) -> bool {
    let heard = app.events.heard();
    if heard != app.applied {
        tracing::trace!(
            events = heard - app.applied,
            "browser events reached the interface"
        );
        app.applied = heard;
    }
    let mut changed = false;
    while let Some(change) = app.events.pop() {
        changed |= app.apply(change, tx);
    }
    changed
}
