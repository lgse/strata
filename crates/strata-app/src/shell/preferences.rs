// SPDX-License-Identifier: MIT

//! Window-level preference propagation, theme/look updates, and GIO power-saver tracking.

use std::{rc::Rc, time::Duration};

use rune::Tx;
use strata_core::{model::ViewPreferences, util::DateFormat};
use strata_fs::app::{Browser, FreshnessInput};

use super::{App, Window, preferences, shown_view};
use crate::{
    settings::Saved,
    slots::{Later, View},
    theme::ThemeEngine,
};

pub(super) struct PowerMonitorGuard {
    monitor: gio::PowerProfileMonitor,
    handler: Option<glib::SignalHandlerId>,
}

impl Drop for PowerMonitorGuard {
    fn drop(&mut self) {
        use gio::prelude::*;
        if let Some(handler) = self.handler.take() {
            self.monitor.disconnect(handler);
        }
    }
}

pub(super) struct WindowServices {
    pub(super) themes: ThemeEngine,
    pub(super) power_saver: bool,
    pub(super) power_monitor: Option<PowerMonitorGuard>,
}

impl WindowServices {
    pub(super) fn load(settings: &mut Saved, later: &Later<App>) -> Self {
        settings.report_to(later.clone());
        let themes = ThemeEngine::load(settings, later.clone());
        let (power_saver, power_monitor) = follow_power_saver(later.clone());
        Self {
            themes,
            power_saver,
            power_monitor,
        }
    }
}

#[cfg(test)]
fn follow_power_saver(_: Later<App>) -> (bool, Option<PowerMonitorGuard>) {
    (false, None)
}

#[cfg(not(test))]
fn follow_power_saver(later: Later<App>) -> (bool, Option<PowerMonitorGuard>) {
    use gio::prelude::*;
    let monitor = gio::PowerProfileMonitor::get_default();
    let enabled = monitor.is_power_saver_enabled();
    let handler = monitor.connect_power_saver_enabled_notify(move |monitor| {
        let saving = monitor.is_power_saver_enabled();
        later.then(move |app: &mut App, _: &mut Tx| {
            if app.window.power_saver != saving {
                app.window.power_saver = saving;
                sync_freshness(app);
            }
            false
        });
    });
    (
        enabled,
        Some(PowerMonitorGuard {
            monitor,
            handler: Some(handler),
        }),
    )
}

/// The look follows the saved settings as they are now: a part that changes one the look
/// reads calls this after `saved(app).change(…)`, as GTK's theme follows its preference.
pub(crate) fn follow_look(app: &mut App) {
    let App { look, window, .. } = app;
    window.themes.apply(
        look,
        window.settings.preferences(),
        window.animations_allowed,
    );
    crate::preview::sync_theme(app);
}

/// Updates the look with colour motion and font family switching on `tx`.
pub(crate) fn follow_look_tx(app: &mut App, tx: &mut Tx) {
    let App { look, window, .. } = app;
    let next = window
        .themes
        .build_look(window.settings.preferences(), window.animations_allowed);
    window.themes.apply_tx(look, next, tx);
    crate::preview::refresh_header(app);
    crate::preview::sync_theme(app);
}

/// Syncs live preferences (`date_format`, `List` relative-date timer, and active `Preview`
/// backend/wrap/header) after a Settings edit or external reload.
pub(crate) fn sync_live_preferences(app: &mut App, tx: &mut Tx) {
    let format = DateFormat::parse(&preferences(app).date_format);
    strata_core::util::set_date_format(format);
    app.list.sync_date_format(format);
    if app.list.is_relative_dates() {
        tx.wake_after(Duration::ZERO);
    }
    crate::preview::sync_preferences(app);
}

/// Another window of the process changed the settings: this one reads them again, and what
/// follows a change made here follows this one too, as every GTK window binds to the one
/// `PreferenceManager`. What a part reads at use follows with the values themselves.
pub(crate) fn follow_another_window(app: &mut App, tx: &mut Tx) -> bool {
    if !app.window.settings.follow() {
        return false;
    }
    tracing::trace!(
        window = app.window.id,
        "followed another window's change of the settings"
    );
    follow_look_tx(app, tx);
    sync_live_preferences(app, tx);
    let sort = strata_desktop::settings::sort_preferences(preferences(app));
    sync_default_preferences(app, sort);
    sync_freshness(app);
    super::refresh_worn(app);
    true
}

pub(crate) fn themes(app: &App) -> &ThemeEngine {
    &app.window.themes
}

pub(crate) fn reload_omarchy(app: &mut App) -> bool {
    let Window {
        themes, settings, ..
    } = &mut app.window;
    themes.reload_omarchy(settings)
}

pub(crate) fn language_restart_required(app: &App) -> bool {
    app.window.settings.language_restart_required()
}

pub(crate) fn interface_renderer_restart_required(app: &App) -> bool {
    app.window.settings.interface_renderer_restart_required()
}

pub(super) fn sync_browser_freshness(
    browser: &Rc<Browser>,
    view: Option<&'static View<App>>,
    watch_network_folders: bool,
    power_saver: bool,
) {
    let current = browser.freshness_input();
    browser.set_freshness(FreshnessInput {
        scope: view.map_or(current.scope, |shown| shown.rechecks),
        watch_network_folders,
        power_saver,
        ..current
    });
}

/// Applies `watch_network_folders` and `power_saver` across every tab's browser.
pub(crate) fn sync_freshness(app: &mut App) {
    let watch_network = preferences(app).watch_network_folders;
    let power_saver = app.window.power_saver;
    let shown = shown_view(app);
    sync_browser_freshness(&app.browser, shown, watch_network, power_saver);
    app.window.tabs.for_each_parked(|browser, view| {
        sync_browser_freshness(browser, view, watch_network, power_saver);
    });
}

/// Applies `remember_folder_views` across every tab's browser and resyncs open columns' sorts.
pub(crate) fn sync_folder_sorts(app: &mut App) {
    let saved = preferences(app);
    app.window.folder_views.sync_sorts(&app.browser, saved);
    app.browser.resync_column_sorts();
    app.window.tabs.for_each_parked(|browser, _| {
        app.window.folder_views.sync_sorts(browser, saved);
        browser.resync_column_sorts();
    });
}

/// Applies default sort preferences across every tab's browser.
pub(crate) fn sync_default_preferences(app: &mut App, sort: ViewPreferences) {
    app.browser.apply_default_preferences(sort);
    app.browser.resync_column_sorts();
    app.window.tabs.for_each_parked(|browser, _| {
        browser.apply_default_preferences(sort);
        browser.resync_column_sorts();
    });
}
