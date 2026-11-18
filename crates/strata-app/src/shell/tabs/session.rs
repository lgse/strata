// SPDX-License-Identifier: MIT

//! Persisting and restoring the open tabs in `tabs.toml` (`strata_desktop::tabs_session`), by
//! the GTK window's rules (`src/ui/window/launch.rs`, `src/ui/window/composition/tabs.rs` and
//! `docs/rune/shared-state.md`, "Tab sessions"):
//!
//! - `load_restorable()` runs at start-up before the window opens (`crate::launch::initial_for`),
//!   or, for a window a later launch opens, off the interface thread ([`queue_present_start`]);
//!   never on it in `poll` or an update.
//! - Only a window opened with no argument sets `persist = true` and restores a saved session:
//!   the process's own launch, or a later one, as every window GTK's `present` opens does. A
//!   launch on a folder or a file leaves `tabs.toml` untouched.
//! - Nothing saves while `restoring` is true, and an empty location list is never saved.
//! - While `restore_tabs` is off, `persist_session` removes `tabs.toml` instead of saving, and
//!   turning `restore_tabs` off live removes it at once (`crate::settings::Saved::change`).
//! - A restored local folder proven missing or replaced by a file discards its tab when other
//!   tabs remain, or opens the default folder when it is the last tab.

use std::{path::PathBuf, time::Duration};

use rune::Tx;
use strata_core::model::Location;
use strata_desktop::tabs_session::{self, RestoredSession};
use strata_fs::{
    adapters::{mount_lanes, probes},
    app::Browser,
    services::LocationValidationError,
};

use super::{
    App, Here, close, give_back_the_keyboard, located, open, preferences, publish, set_awaited,
    settle, show, shown_tab, tab_browser, tabs,
};
use crate::slots::TabKey;

/// Marks `app` as the plain-launch window that persists its tabs, and queues its initial
/// restore or default-folder open for the first pump or poll.
pub(crate) fn queue_plain_start(app: &mut App, restored: Option<RestoredSession>) {
    app.window.tabs.persist = true;
    let later = app.window.events.later();
    later.then(move |app, tx| start_plain(app, restored, tx));
    // Wakes the windowed host's embedded GLib loop so `pump` runs without waiting on input.
    glib::idle_add_local_once(|| {});
}

/// Starts a window a later launch with no argument opened as a plain launch's: GTK's `present`
/// gives every such window `restore_tabs`, so it restores the saved session and persists its
/// tabs (`present_target` in `src/ui/window/launch.rs`). The window opens after start-up, so
/// `tabs.toml` is read off the interface thread, and the start waits for it.
pub(crate) fn queue_present_start(app: &mut App) {
    app.window.tabs.persist = true;
    let later = app.window.events.later();
    let restore = preferences(app).restore_tabs;
    glib::MainContext::default().spawn_local(async move {
        let restored = if restore {
            let read = gio::spawn_blocking(tabs_session::load_restorable).await;
            read.ok().flatten()
        } else {
            None
        };
        later.then(move |app, tx| start_plain(app, restored, tx));
    });
}

/// Opens a plain-launch window's tabs: restores `restored` when `restore_tabs` is on and a
/// session was loaded at start-up, else opens the default folder (`FirstFolder::open` in
/// `src/ui/window/launch.rs`).
pub(crate) fn start_plain(app: &mut App, restored: Option<RestoredSession>, tx: &mut Tx) -> bool {
    app.window.tabs.persist = true;
    if preferences(app).restore_tabs
        && let Some(session) = restored.filter(|session| !session.tabs.is_empty())
    {
        restore_session(app, session, tx);
        return true;
    }
    persist_session(app);
    start_shown_tab(app, tx);
    open_default_folder(app, shown_tab(app), tx);
    true
}

fn start_shown_tab(app: &mut App, tx: &mut Tx) {
    let parts = app.window.here.parts;
    for part in parts.active() {
        (part.tab_shown)(app, tx);
    }
    give_back_the_keyboard(app, tx);
}

/// Builds every tab in `session` and selects `session.active`, holding `restoring` until all
/// tabs are in place so no partial session overwrites `tabs.toml` (`TabWindow::restore_session`).
pub(crate) fn restore_session(app: &mut App, session: RestoredSession, tx: &mut Tx) {
    if session.tabs.is_empty() || super::super::modal_open(app) {
        return;
    }
    app.window.tabs.restoring = true;
    let active = session.active.min(session.tabs.len() - 1);
    for (index, location) in session.tabs.into_iter().enumerate() {
        let key = if index == 0 {
            shown_tab(app)
        } else {
            open(app, None, tabs(app).len(), tx)
        };
        reopen_tab(app, key, location, tx);
    }
    let chosen = tabs(app)[active];
    if chosen != shown_tab(app) {
        show(app, chosen, tx);
    } else {
        super::super::retitle_now(app, tx);
        start_shown_tab(app, tx);
    }
    app.window.tabs.restoring = false;
}

fn reopen_tab(app: &mut App, key: TabKey, location: Location, tx: &mut Tx) {
    if location.native_path().is_none() {
        navigate_tab(app, key, location, tx);
        return;
    }
    set_awaited(app, key, Some(location.clone()));
    if key == shown_tab(app) {
        app.retitle(tx);
    }
    open_checked(app, CheckedTab { key, location }, tx, |app, key, tx| {
        set_awaited(app, key, None);
        if tabs(app).len() > 1 && !super::super::modal_open(app) {
            close(app, key, tx);
        } else {
            if key == shown_tab(app) {
                super::super::retitle_now(app, tx);
            }
            open_default_folder(app, key, tx);
        }
    });
}

/// Opens the saved `default_directory` in tab `key` once checked, clearing `default_directory`
/// and falling back to home only if the folder is proven gone (`open_default_folder` in
/// `src/ui/window/launch.rs`).
pub(crate) fn open_default_folder(app: &mut App, key: TabKey, tx: &mut Tx) {
    let home = Location::local(crate::launch::home_directory());
    let Some(saved) = preferences(app).default_directory.clone() else {
        navigate_tab(app, key, home, tx);
        return;
    };
    let checked = CheckedTab {
        key,
        location: Location::local(&saved),
    };
    open_checked(app, checked, tx, move |app, key, tx| {
        if preferences(app).default_directory.as_ref() == Some(&saved) {
            super::super::saved(app).change(|prefs| prefs.default_directory = None);
        }
        navigate_tab(app, key, home, tx);
    });
}

struct CheckedTab {
    key: TabKey,
    location: Location,
}

fn open_checked(
    app: &mut App,
    CheckedTab { key, location }: CheckedTab,
    tx: &mut Tx,
    absent: impl FnOnce(&mut App, TabKey, &mut Tx) + 'static,
) {
    let Some(path) = location.native_path().map(PathBuf::from) else {
        navigate_tab(app, key, location, tx);
        return;
    };
    let Some(browser) = tab_browser(app, key) else {
        return;
    };
    let generation = browser.bump_navigation_generation();
    let later = app.window.events.later();
    glib::MainContext::default().spawn_local(async move {
        let gone = folder_is_gone(path).await;
        later.then(move |app, tx| {
            let Some(browser) = tab_browser(app, key).cloned() else {
                return false;
            };
            if browser.navigation_generation() != generation {
                return false;
            }
            if gone {
                absent(app, key, tx);
            } else {
                navigate_tab(app, key, location, tx);
            }
            true
        });
    });
    // `start_plain` runs in `settle` after the current `pump` queried GLib, so wake the next
    // `poll` to step the new local task onto its worker thread.
    tx.wake_after(Duration::ZERO);
}

async fn folder_is_gone(path: PathBuf) -> bool {
    glib::future_with_timeout(mount_lanes::STALLED_AFTER, async {
        match probes::folder(&path).await {
            Err(LocationValidationError::NotDirectory) => true,
            Err(LocationValidationError::Missing) => probes::folder_was_deleted(&path).await,
            _ => false,
        }
    })
    .await
    .unwrap_or(false)
}

fn navigate_tab(app: &mut App, key: TabKey, location: Location, tx: &mut Tx) {
    if key == shown_tab(app) {
        publish(app);
        app.browser.navigate_location(location, true);
        settle(app, tx);
        return;
    }
    let Some(browser) = tab_browser(app, key).cloned() else {
        return;
    };
    browser.navigate_location(location, true);
    settle(app, tx);
}

/// Follows each tab's `active_location()`, clearing `awaited` once a folder arrives and saving
/// the session when any tab's active location changed (`observe_tab_location` in
/// `src/ui/window/composition/tabs.rs`).
pub(crate) fn observe_locations(app: &mut App) {
    let mut changed = observe_tab(&mut app.window.here, &app.browser);
    for (_, parked) in &mut app.window.tabs.parked {
        changed |= observe_tab(&mut parked.here, &parked.browser);
    }
    if changed {
        persist_session(app);
    }
}

fn observe_tab(here: &mut Here, browser: &Browser) -> bool {
    let current = browser.active_location();
    if current.is_some() {
        here.awaited = None;
    }
    if browser.navigation_update_in_progress() || here.observed == current {
        return false;
    }
    here.observed = current;
    true
}

/// Queues a write of `tabs.toml` for the persisting window (`TabWindow::persist_session`).
pub(crate) fn persist_session(app: &App) {
    if !app.window.tabs.persist || app.window.tabs.restoring {
        return;
    }
    if !preferences(app).restore_tabs {
        tabs_session::remove();
        return;
    }
    let shown = shown_tab(app);
    let mut locations = Vec::with_capacity(tabs(app).len());
    let mut active = 0;
    for &key in tabs(app) {
        let Some(location) = located(app, key) else {
            continue;
        };
        if key == shown {
            active = locations.len();
        }
        locations.push(location);
    }
    if locations.is_empty() {
        return;
    }
    tabs_session::save(&locations, active);
}

#[cfg(test)]
mod tests;
