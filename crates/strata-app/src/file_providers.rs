// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use rune::{Pixels, Tx};
use strata_components::{context_menu::Section, contract::EntryId, progress_dock::Notice};
use strata_core::i18n::tr;
use strata_fs::services::file_providers as protocol;

use crate::{
    App,
    error_dialog::{self, Error},
    pane::Pane,
    shell,
    slots::{Later, Part, Place, Scope, Setup},
};

mod freshness;
mod hub;
pub(crate) mod menus;

use hub::{Effect, Hub, Pending, Provider, request, spawn_discovery};
use menus::OpenMenu;

const MAX_VISIBLE_SLOTS: usize = 1024;
const NOTICE_SECONDS: f64 = 4.0;
const TICK_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Default)]
struct Presented {
    version: u64,
    badge_key: Option<(String, String)>,
    badge: Option<Pixels>,
    description: String,
}

impl Presented {
    fn is_decorated(&self) -> bool {
        self.badge_key.is_some() || !self.description.is_empty()
    }
}

pub(crate) struct FileProviders {
    later: Later<App>,
    started: bool,
    timer: Option<Rc<Cell<bool>>>,
    hub: Hub,
    open_menu: OpenMenu,
    presented: HashMap<EntryId, Presented>,
    next_version: u64,
}

impl Drop for FileProviders {
    fn drop(&mut self) {
        self.stop_timer();
    }
}

impl FileProviders {
    fn ensure_started(&mut self) {
        if !self.started {
            self.started = true;
            self.hub.discovery = Some(spawn_discovery());
            self.start_timer();
        }
    }

    fn stop_timer(&mut self) {
        if let Some(token) = self.timer.take() {
            token.set(false);
        }
    }

    fn start_timer(&mut self) {
        if self.timer.is_some() {
            return;
        }
        let token = Rc::new(Cell::new(true));
        self.timer = Some(token.clone());
        let later = self.later.clone();
        glib::timeout_add_local(TICK_INTERVAL, move || {
            if !token.get() {
                return glib::ControlFlow::Break;
            }
            later.then(tick);
            glib::ControlFlow::Continue
        });
    }

    fn clear_all_presentations(&mut self) -> bool {
        let mut changed = false;
        for entry in self.presented.values_mut() {
            if entry.is_decorated() {
                self.next_version += 1;
                entry.version = self.next_version;
                entry.badge_key = None;
                entry.badge = None;
                entry.description.clear();
                changed = true;
            }
        }
        changed
    }

    fn update_presentations(&mut self, visible: &[(EntryId, String)]) -> bool {
        let mut changed = false;
        let mut seen_ids = HashSet::with_capacity(visible.len());
        for (id, path) in visible {
            seen_ids.insert(*id);
            changed |= self.present_entry(*id, path);
        }
        changed | self.clear_unseen(&seen_ids)
    }

    fn present_entry(&mut self, id: EntryId, path: &str) -> bool {
        let badges = self.hub.badges(path);
        let description = badges
            .iter()
            .map(|b| format!("{}: {}", b.provider, b.description))
            .collect::<Vec<_>>()
            .join("; ");
        let first = badges.first();
        let badge_key = first.map(|b| (b.provider.to_owned(), b.icon_id.to_owned()));
        let badge = first.map(|b| b.pixels.clone());
        if badge_key.is_none() && description.is_empty() {
            return self
                .presented
                .remove(&id)
                .is_some_and(|entry| entry.is_decorated());
        }
        let entry = self.presented.entry(id).or_default();
        if entry.badge_key == badge_key && entry.description == description {
            return false;
        }
        self.next_version += 1;
        entry.version = self.next_version;
        entry.badge_key = badge_key;
        entry.badge = badge;
        entry.description = description;
        true
    }

    fn clear_unseen(&mut self, seen_ids: &HashSet<EntryId>) -> bool {
        let mut changed = false;
        self.presented.retain(|id, entry| {
            if seen_ids.contains(id) {
                return true;
            }
            if entry.is_decorated() {
                changed = true;
            }
            false
        });
        changed
    }
}

fn tick(app: &mut App, tx: &mut Tx) -> bool {
    if app.file_providers.timer.is_none() {
        return false;
    }
    let mut effects = Vec::new();
    app.file_providers.hub.drain(&mut effects);
    if app.file_providers.hub.idle() {
        app.file_providers.stop_timer();
        let cleared = app.file_providers.clear_all_presentations();
        let menu_changed = refresh_open_menu(app, &mut effects);
        let showed = apply_effects(app, effects, tx);
        return cleared || menu_changed || showed;
    }
    let visible = visible_native_entries(app);
    let paths: Vec<String> = visible.iter().map(|(_, p)| p.clone()).collect();
    app.file_providers
        .hub
        .request_missing_decorations(&paths, &mut effects);
    let dec_changed = app.file_providers.update_presentations(&visible);
    let menu_changed = refresh_open_menu(app, &mut effects);
    let showed = apply_effects(app, effects, tx);
    dec_changed || menu_changed || showed
}

fn refresh_open_menu(app: &mut App, effects: &mut Vec<Effect>) -> bool {
    if !app.menu.state.is_open() {
        app.file_providers.open_menu.clear();
        return false;
    }
    let Some(key) = app.file_providers.open_menu.key().cloned() else {
        return false;
    };
    let result = app.file_providers.hub.menu(&key, effects);
    let built = menus::build_sections_and_leaves(&app.file_providers.hub.providers, result);
    let slot = app.menu.provider_slot();
    app.file_providers
        .open_menu
        .refresh(&mut app.menu.state, slot, built)
}

fn apply_effects(app: &mut App, effects: Vec<Effect>, tx: &mut Tx) -> bool {
    let had_effects = !effects.is_empty();
    for effect in effects {
        match effect {
            Effect::Dialog(detail) => {
                error_dialog::show(
                    app,
                    Error::information(
                        tr("File availability"),
                        tr("Reported by the file provider"),
                        detail,
                    ),
                );
            }
            Effect::Notice { heading, message } => {
                let notice = Notice::new(message)
                    .heading(heading)
                    .shown_for(NOTICE_SECONDS);
                crate::transfer::show_notice(app, notice, tx);
            }
        }
    }
    had_effects
}

fn visible_native_entries(app: &App) -> Vec<(EntryId, String)> {
    let rows = match shell::preferences(app).browser_mode.as_str() {
        "list" | "explorer" => pane_visible_rows(&app.list.single.pane, app),
        "icons" | "grid" => pane_visible_rows(&app.icons.single.pane, app),
        _ => app.columns.visible_rows(),
    };
    let Some(view_rows) = shell::shown_view(app).map(|v| &v.rows) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(rows.len().min(MAX_VISIBLE_SLOTS));
    for (depth, id) in rows {
        if out.len() >= MAX_VISIBLE_SLOTS {
            break;
        }
        let location = crate::filter::targets::at(app, depth, id)
            .map(|e| e.location)
            .or_else(|| {
                let pos = (view_rows.position)(app, (depth, id))?;
                Some(app.browser.entry_at(depth, pos)?.location)
            });
        if let Some(path) = location
            .as_ref()
            .and_then(|loc| valid_native_path(loc.native_path()))
        {
            out.push((id, path));
        }
    }
    out
}

fn pane_visible_rows(pane: &Pane, app: &App) -> Vec<(usize, EntryId)> {
    if app.filter.pane.shows() {
        let Some((_, depth)) = crate::filter::targets::selected_with_depth(app) else {
            return Vec::new();
        };
        let source = app.filter.pane.source();
        let len = source.len().min(protocol::PATH_LIMIT);
        return (0..len).map(|i| (depth, source.id_at(i))).collect();
    }
    let (Some(depth), Some(listing)) = (pane.depth(), pane.listing()) else {
        return Vec::new();
    };
    use strata_components::contract::EntrySource as _;
    let len = listing.len();
    let vis = pane.collection().visible_range();
    let range = if vis.is_empty() {
        0..len.min(protocol::PATH_LIMIT)
    } else {
        vis.start..vis.end.min(len)
    };
    range.map(|i| (depth, listing.id_at(i))).collect()
}

fn valid_native_path(path: Option<&Path>) -> Option<String> {
    let text = path?.to_str()?;
    (Path::new(text).is_absolute() && text.len() <= 16384 && !text.contains('\0'))
        .then(|| text.to_owned())
}

/// Starts watching a newly opened context menu and returns its initial provider sections.
pub(crate) fn begin_menu(
    app: &mut App,
    paths: Option<Vec<PathBuf>>,
    background: bool,
    tx: &mut Tx,
) -> Vec<Section> {
    app.file_providers.ensure_started();
    let key = paths.and_then(|paths| menus::menu_key(&paths, background));
    let mut effects = Vec::new();
    let initial = match &key {
        Some(k) => app.file_providers.hub.menu(k, &mut effects),
        None => Vec::new(),
    };
    let built = menus::build_sections_and_leaves(&app.file_providers.hub.providers, initial);
    let sections = app.file_providers.open_menu.begin(key, built);
    apply_effects(app, effects, tx);
    sections
}

/// Dispatches an activated `"provider:<idx>:<id>"` item from the context menu.
pub(crate) fn activate_menu_action(app: &mut App, action: &str, tx: &mut Tx) {
    let Some(chosen) = app.file_providers.open_menu.take_activation(action) else {
        return;
    };
    if chosen.provider >= app.file_providers.hub.providers.len() {
        return;
    }
    let mut req = request("activate", chosen.paths, chosen.background);
    req.action = Some(chosen.action_id);
    req.context = chosen.context;
    let effect =
        app.file_providers
            .hub
            .send(chosen.provider, req, Pending::Activate(chosen.provider));
    app.file_providers.start_timer();
    apply_effects(app, effect.into_iter().collect(), tx);
}

/// Stops running providers and rediscovers registrations under the providers config directory.
pub(crate) fn reload(app: &mut App) {
    app.file_providers.ensure_started();
    app.file_providers.hub.reload(spawn_discovery());
    app.file_providers.clear_all_presentations();
    app.file_providers.start_timer();
}

pub(crate) fn badge_version(app: &App, id: EntryId) -> u64 {
    app.file_providers
        .presented
        .get(&id)
        .map_or(0, |p| p.version)
}

pub(crate) fn badge_pixels(app: &App, id: EntryId) -> Option<Pixels> {
    app.file_providers
        .presented
        .get(&id)
        .and_then(|p| p.badge.clone())
}

pub(crate) fn badge_description(app: &App, id: EntryId) -> Option<&str> {
    app.file_providers
        .presented
        .get(&id)
        .map(|p| p.description.as_str())
        .filter(|d| !d.is_empty())
}

pub(crate) fn combine_description(base: Option<String>, extra: Option<&str>) -> Option<String> {
    match (
        base.filter(|b| !b.is_empty()),
        extra.filter(|e| !e.is_empty()),
    ) {
        (Some(base), Some(extra)) => Some(format!("{base}, {extra}")),
        (Some(base), None) => Some(base),
        (None, Some(extra)) => Some(extra.to_owned()),
        (None, None) => None,
    }
}

impl Part for FileProviders {
    const PLACE: Place<App> = Place::Overlays(|_, _| None);
    const SCOPE: Scope = Scope::Window;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            started: false,
            timer: None,
            hub: Hub::new(),
            open_menu: OpenMenu::default(),
            presented: HashMap::new(),
            next_version: 0,
        }
    }

    fn poll(app: &mut App, _: f64, _: &mut Tx) -> bool {
        app.file_providers.ensure_started();
        false
    }
}

#[cfg(test)]
mod tests;
