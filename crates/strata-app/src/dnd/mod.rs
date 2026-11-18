// SPDX-License-Identifier: MIT

//! Window-scoped drag and drop (`A19`, `Scope::Window`):
//! - Attaches `strata_platform::dnd::Dnd` on the windowed host (`host::windowed`) and drives an
//!   in-window keyed `DropZone` router on the headless offscreen host (`host::offscreen`) as well
//!   as for sidebar row tab-opening drags.
//! - Tracks the active local drag's entries, folder snapshot, row fade marks (`RowMarks.dragging`),
//!   and `tab_strip::set_open_drag` (`TabOpen::AtEnd` for folder-only view drags, `TabOpen::AtGap`
//!   for sidebar location drags).
//! - Shares volume-lookup caching, `Copy`/`Move`/`Ask` transfer dispatch, Trash drops, and 750 ms
//!   spring-loaded opening across views, tabs, location crumbs, and sidebar rows.

use std::collections::HashSet;

use gio::prelude::*;
use rune::{AppHandle, Drag, El, Mods, Tx, WindowHandle};
use strata_components::{
    collection::element::RowMarks,
    contract::{
        Dnd as Binder, EntryId,
        dnd::{DragPayload, DragSource, DropActions},
    },
    sidebar::PinVerdict,
    tabs::TabOpen,
};
use strata_core::model::{FileEntry, Location};
use strata_desktop::keys::bindings::{ESCAPE, ESCAPE_ANY};
use strata_fs::{adapters::gio_file_for_location, services::has_web_scheme};

use crate::{
    App, clipboard,
    commands::{Action, Call, Keys, Outcome, Stage},
    shell,
    slots::{Frame, Later, Part, Place, Scope, Setup, TabKey},
    tab_strip,
};

pub(crate) mod adapter;
mod content;
mod drag;
pub(crate) mod policy;
mod registry;
mod router;
#[cfg(test)]
mod tests;

pub(crate) use content::CONTENT_TYPES;
pub(crate) use policy::{
    FileDrop, commit, commit_file_drop, commit_trash_drop, drop_action, dropped_locations,
    locations_from_uris, trash_action,
};
pub(crate) use registry::PointZone;

struct ActiveDrag {
    tab: TabKey,
    ids: HashSet<EntryId>,
    folders: Option<Vec<(Location, String)>>,
    sidebar_row: bool,
}

pub(crate) struct Dnd {
    later: Later<App>,
    wiring: router::Wiring,
    volumes: policy::VolumeCache,
    spring: policy::Spring,
    out: Option<ActiveDrag>,
}

fn nothing(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    None
}

impl Part for Dnd {
    const PLACE: Place<App> = Place::Overlays(nothing);
    const SCOPE: Scope = Scope::Window;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            wiring: router::Wiring::new(),
            volumes: policy::VolumeCache::default(),
            spring: policy::Spring::default(),
            out: None,
        }
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let sprung = policy::poll_spring(app, clock, tx);
        let wiring = app.dnd.wiring.clone();
        wiring.poll_stand_in(app, tx) | sprung
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("dnd-cancel", cancel_by_key)
        .command(ESCAPE)
        .keys(&[Keys::at(Stage::TabStrip, &[ESCAPE_ANY]).when(|app, _| is_dragging(app))])];
}

fn cancel_by_key(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if !is_dragging(app) {
        return Outcome::Declined;
    }
    let wiring = app.dnd.wiring.clone();
    wiring.cancel(app, tx, true);
    on_drag_ended(app, tx);
    Outcome::Ran
}

pub(crate) fn is_dragging(app: &App) -> bool {
    app.dnd.out.is_some() || app.dnd.wiring.is_dragging()
}

pub(crate) fn is_sidebar_drag(app: &App) -> bool {
    app.dnd.out.as_ref().is_some_and(|out| out.sidebar_row)
}

pub(crate) fn local_pin_folders(app: &App) -> Option<&[(Location, String)]> {
    app.dnd.out.as_ref()?.folders.as_deref()
}

pub(crate) fn binder(app: &App) -> Binder<App> {
    app.dnd.wiring.binder()
}

pub(crate) fn drop_target_by_point(app: &App, el: El, key: &str, zoned: PointZone) -> El {
    app.dnd.wiring.drop_target_by_point(el, key, zoned)
}

pub(crate) fn bind(app: &mut App, p1: strata_platform::dnd::Dnd<App>) {
    app.dnd.wiring.bind_windowed(p1);
}

pub(crate) fn attach(
    p1: &strata_platform::dnd::Dnd<App>,
    window: &WindowHandle,
    handle: &AppHandle<App>,
) {
    if let Err(error) = p1.attach(window, handle) {
        tracing::warn!(%error, "unable to attach system drag and drop");
    }
}

pub(crate) fn shutdown() {
    if let Err(error) = strata_platform::dnd::shutdown() {
        tracing::debug!(%error, "system drag-and-drop shutdown");
    }
}

pub(crate) fn modifiers_changed(app: &mut App, tx: &mut Tx) {
    let wiring = app.dnd.wiring.clone();
    wiring.repick(tx);
}

pub(crate) fn repick(app: &mut App, tx: &mut Tx) {
    let wiring = app.dnd.wiring.clone();
    wiring.repick(tx);
}

pub(crate) fn keep_alive(app: &App) -> Option<El> {
    app.dnd.wiring.keep_alive(app)
}

pub(crate) fn keep_alive_drag(app: &mut App, d: &mut Drag, tx: &mut Tx) {
    let wiring = app.dnd.wiring.clone();
    wiring.keep_alive_drag(app, d, tx);
}

pub(crate) fn drag_phase(app: &mut App, d: &Drag, key: &str, tx: &mut Tx) -> bool {
    let wiring = app.dnd.wiring.clone();
    wiring.drag_phase(
        app,
        drag::Gesture {
            drag: d,
            source: key,
        },
        tx,
    )
}

pub(crate) fn with_marks(app: &App, id: EntryId, mut marks: RowMarks) -> RowMarks {
    let tab = shell::tabs::shown_tab(app);
    marks.dragging = app
        .dnd
        .out
        .as_ref()
        .is_some_and(|out| !out.sidebar_row && out.tab == tab && out.ids.contains(&id));
    marks
}

pub(crate) fn can_pin_location(location: &Location) -> bool {
    !clipboard::paths::is_trash(location)
        && !location.is_recent_location()
        && !location.uri_value().is_some_and(has_web_scheme)
}

fn build_entries_payload(
    app: &App,
    entries: &[FileEntry],
    mods: Mods,
) -> Option<(DragPayload, ActiveDrag)> {
    if entries.is_empty() {
        return None;
    }
    let mut ids = Vec::with_capacity(entries.len());
    let mut uris = Vec::with_capacity(entries.len());
    for entry in entries {
        ids.push(shell::id_of(app, &entry.location));
        uris.push(gio_file_for_location(&entry.location).uri().into());
    }
    let can_move = entries
        .iter()
        .all(|entry| clipboard::paths::can_remove(&entry.location));
    let actions = adapter::actions_for_mods(mods, can_move);
    let payload = DragPayload::new(ids.clone(), uris).actions(actions);
    let all_folders = entries.iter().all(FileEntry::is_directory);
    let folders = (all_folders
        && entries
            .iter()
            .all(|entry| can_pin_location(&entry.location)))
    .then(|| {
        entries
            .iter()
            .map(|entry| (entry.location.clone(), entry.display_name.clone()))
            .collect()
    });
    let active = ActiveDrag {
        tab: shell::tabs::shown_tab(app),
        ids: ids.into_iter().collect(),
        folders,
        sidebar_row: false,
    };
    Some((payload, active))
}

pub(crate) fn begin_entries(app: &mut App, entries: Vec<FileEntry>, scale: f32, tx: &mut Tx) {
    let mods = shell::held_modifiers(app);
    let Some((payload, active)) = build_entries_payload(app, &entries, mods) else {
        return;
    };
    let all_folders = entries.iter().all(FileEntry::is_directory);
    let icon = adapter::drag_icon(entries.len(), all_folders, scale);
    let wiring = app.dnd.wiring.clone();
    install_active_drag(app, active, all_folders.then_some(TabOpen::AtEnd));
    let launch = drag::Launch::new(payload, |app, _, tx| on_drag_ended(app, tx)).icon(icon);
    if wiring.start(app, launch, tx).is_err() {
        on_drag_ended(app, tx);
    }
}

pub(crate) fn preview_drag_source() -> DragSource<App> {
    DragSource::new(|app: &App| {
        let entry = crate::preview::current_entry(app)?;
        let mods = shell::held_modifiers(app);
        let (payload, _) = build_entries_payload(app, std::slice::from_ref(entry), mods)?;
        Some(payload)
    })
}

pub(super) fn on_preview_drag_start(app: &mut App, _tx: &mut Tx) {
    let Some(entry) = crate::preview::current_entry(app).cloned() else {
        return;
    };
    let mods = shell::held_modifiers(app);
    if let Some((_, active)) = build_entries_payload(app, std::slice::from_ref(&entry), mods) {
        let open = entry.is_directory().then_some(TabOpen::AtEnd);
        install_active_drag(app, active, open);
    }
}

pub(crate) fn sidebar_drag_start(app: &mut App, location: Location) {
    let id = shell::id_of(app, &location);
    let uri: String = gio_file_for_location(&location).uri().into();
    let payload =
        DragPayload::new(vec![id], vec![uri]).actions(DropActions::COPY | DropActions::MOVE);
    let active = ActiveDrag {
        tab: shell::tabs::shown_tab(app),
        ids: HashSet::from([id]),
        folders: None,
        sidebar_row: true,
    };
    let wiring = app.dnd.wiring.clone();
    let launch = drag::Launch::new(payload, |app, _, tx| on_drag_ended(app, tx));
    wiring.start_sidebar_drag(app, launch);
    install_active_drag(app, active, Some(TabOpen::AtGap));
}

pub(crate) fn sidebar_drag_move(app: &mut App, inside_sidebar: bool, at: [f32; 2], tx: &mut Tx) {
    let wiring = app.dnd.wiring.clone();
    if inside_sidebar {
        wiring.leave_hover(app, tx);
    } else {
        wiring.move_to(at, tx);
    }
}

pub(crate) fn sidebar_drag_end(app: &mut App, dropped_outside: Option<[f32; 2]>, tx: &mut Tx) {
    let wiring = app.dnd.wiring.clone();
    match dropped_outside {
        Some(at) => wiring.drop_at(at, tx),
        None => {
            wiring.cancel(app, tx, false);
            on_drag_ended(app, tx);
        }
    }
}

fn install_active_drag(app: &mut App, active: ActiveDrag, open: Option<TabOpen>) {
    let verdict = match (&active.folders, active.sidebar_row) {
        (_, true) | (None, false) => PinVerdict::Refused,
        (Some(_), false) => PinVerdict::Folders,
    };
    app.sidebar.set_local_pin_verdict(verdict);
    app.dnd.volumes.clear();
    app.dnd.out = Some(active);
    tab_strip::set_open_drag(app, open);
    // GTK's item drag source cancels the peek as its drag begins.
    crate::peek::close(app);
}

pub(super) fn on_drag_ended(app: &mut App, _tx: &mut Tx) {
    // And as it ends; where it was dropped peeks only once the pointer arrives on a row again.
    crate::peek::close(app);
    crate::peek::hold_rest(app);
    app.dnd.out = None;
    app.dnd.spring.clear();
    app.dnd.volumes.clear();
    app.sidebar.end_external_drag();
    app.columns.clear_drag_over();
    tab_strip::set_open_drag(app, None);
}
