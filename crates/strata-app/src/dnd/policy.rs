// SPDX-License-Identifier: MIT

//! Shared drop policy across views, tabs, location crumbs, and sidebar rows:
//! - Volume relation lookup and caching (`lookup_drop_volumes`).
//! - Copy / move / cross-volume prompt (`choose_action` -> `transfer::start_drop_transfer` or
//!   `drop_dialog::prompt`).
//! - Trash drop validation and async `FileEntry` lookup (`trash_dropped`).
//! - 750 ms spring-loaded opening on hovered view folder rows and sidebar folder/pin rows.

use std::{cell::RefCell, time::Duration};

use rune::{Mods, Tx};
use strata_components::{
    collection::element::DropOver,
    contract::{
        EntryId,
        dnd::{
            ActionInput, CrossDevice, DropAction, DropActions, DropData, DropHover, DropOffer,
            Dropped, URI_LIST, VolumeRelation, choose_action,
        },
    },
    sidebar::{DropSite as SidebarDropSite, ItemId},
};
use strata_core::{i18n::tr, model::Location};
use strata_fs::{
    adapters::{
        DropVolumeQuery, DropVolumes, location_for_file, lookup_drop_volumes, query_file_entry,
    },
    services::{
        CrossVolumeDropStrategy, VolumeRelation as FsVolumeRelation, WebLink, has_web_scheme,
        transferable_drop_sources,
    },
};

use super::content;
use crate::{
    App, clipboard, drop_dialog,
    error_dialog::{self, Error},
    shell,
    slots::{Later, TabKey},
    transfer,
};

const VOLUME_CACHE_CAP: usize = 16;
const SPRING_DELAY_SECS: f64 = 0.75;

struct CachedVolumes {
    serial: u64,
    query: DropVolumeQuery,
    state: DropVolumes,
}

#[derive(Default)]
pub(crate) struct VolumeCache(RefCell<Vec<CachedVolumes>>);

impl VolumeCache {
    pub(crate) fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub(crate) fn ensure_and_relation(
        &self,
        later: &Later<App>,
        serial: u64,
        query: &DropVolumeQuery,
    ) -> VolumeRelation {
        if let Some(found) = self
            .0
            .borrow()
            .iter()
            .find(|v| v.serial == serial && &v.query == query)
        {
            return dnd_volume(found.state.relation());
        }
        let later = later.clone();
        let expected = query.clone();
        let state = lookup_drop_volumes(query, move |lookup| {
            later.then(move |app, tx| {
                let mut cache = app.dnd.volumes.0.borrow_mut();
                if let Some(entry) = cache
                    .iter_mut()
                    .find(|v| v.serial == serial && v.query == expected)
                {
                    entry.state = DropVolumes::Ready(lookup);
                    drop(cache);
                    super::repick(app, tx);
                    return true;
                }
                false
            });
        });
        let relation = dnd_volume(state.relation());
        let mut cache = self.0.borrow_mut();
        if cache.len() >= VOLUME_CACHE_CAP {
            cache.remove(0);
        }
        cache.push(CachedVolumes {
            serial,
            query: query.clone(),
            state,
        });
        relation
    }

    fn take_matching(&self, query: &DropVolumeQuery) -> Option<DropVolumes> {
        let mut cache = self.0.borrow_mut();
        let pos = cache.iter().position(|v| &v.query == query)?;
        Some(cache.remove(pos).state)
    }
}

pub(crate) fn valid_drop_destination(location: &Location) -> bool {
    !clipboard::paths::is_trash(location) && !location.is_recent_location()
}

/// A modal dialog is open, or a sidebar row is being dragged: neither drops files.
fn drops_blocked(app: &App) -> bool {
    shell::modal_open(app) || super::is_sidebar_drag(app)
}

fn refuses_file_drop(app: &App, destination: &Location) -> bool {
    drops_blocked(app) || !valid_drop_destination(destination)
}

/// Web links drop as `.desktop` files: only into a local folder, and only when each parses.
fn web_link_action(destination: &Location, uris: &[String]) -> Option<DropAction> {
    let local = destination.native_path().is_some() && !uris.is_empty();
    let parses = uris.iter().all(|uri| WebLink::parse(uri).is_some());
    (local && parses).then_some(DropAction::Copy)
}

pub(crate) fn drop_action(
    app: &App,
    destination: &Location,
    offer: &DropOffer,
    mods: Mods,
) -> Option<DropAction> {
    if refuses_file_drop(app, destination) {
        return None;
    }
    if content::offered(offer) {
        return destination
            .native_path()
            .is_some()
            .then_some(DropAction::Copy);
    }
    let cross = cross_device(app);
    let Some(uris) = offer.uris.as_deref() else {
        return choose_action(ActionInput::new(
            mods,
            offer.actions,
            VolumeRelation::Unknown,
            cross,
        ));
    };
    if uris.iter().any(|uri| has_web_scheme(uri)) {
        return web_link_action(destination, uris);
    }
    let sources = locations_from_uris(uris)?;
    let transferable = transferable_drop_sources(destination, &sources);
    if transferable.is_empty() {
        return None;
    }
    let query = DropVolumeQuery::new(destination, &transferable);
    let relation = app
        .dnd
        .volumes
        .ensure_and_relation(&app.dnd.later, offer.serial, &query);
    let action = choose_action(ActionInput::new(mods, offer.actions, relation, cross))?;
    can_commit_action(action, &transferable).then_some(action)
}

/// What a drop does when the volumes are the same, and when they are not known.
#[derive(Clone, Copy)]
struct Choices {
    same: ActionInput,
    unknown: ActionInput,
}

impl Choices {
    fn of(app: &App, dropped: &Dropped) -> Self {
        let cross = cross_device(app);
        let input =
            |relation| ActionInput::new(dropped.mods, dropped.offer.actions, relation, cross);
        Self {
            same: input(VolumeRelation::Same),
            unknown: input(VolumeRelation::Unknown),
        }
    }

    /// The volumes cannot change what the drop does.
    fn settled(self) -> bool {
        choose_action(self.same) == choose_action(self.unknown)
    }

    /// Either volume relation leaves an action the sources allow.
    fn possible(self, sources: &[Location]) -> bool {
        [self.same, self.unknown]
            .into_iter()
            .any(|input| choose_action(input).is_some_and(|a| can_commit_action(a, sources)))
    }
}

/// What runs once a drop is known to commit, before it does: a tab drop selects its tab.
pub(crate) type BeforeCommit = Box<dyn FnOnce(&mut App, &mut Tx)>;

/// A file drop onto `destination`.
pub(crate) struct FileDrop {
    destination: Location,
    before: Option<BeforeCommit>,
}

impl FileDrop {
    pub(crate) fn to(destination: Location) -> Self {
        Self {
            destination,
            before: None,
        }
    }

    pub(crate) fn before(mut self, before: impl FnOnce(&mut App, &mut Tx) + 'static) -> Self {
        self.before = Some(Box::new(before));
        self
    }
}

/// A drop that will commit, with the files it moves or copies.
struct Commit {
    destination: Location,
    sources: Vec<Location>,
    before: Option<BeforeCommit>,
}

impl Commit {
    fn begin(&mut self, app: &mut App, tx: &mut Tx) {
        if let Some(before) = self.before.take() {
            before(app, tx);
        }
    }
}

pub(crate) fn commit_file_drop(
    app: &mut App,
    destination: Location,
    dropped: &Dropped,
    tx: &mut Tx,
) -> bool {
    commit(app, FileDrop::to(destination), dropped, tx)
}

/// Commits `drop`: at once when the volumes cannot change what it does, else once they are
/// looked up. True when it will commit.
pub(crate) fn commit(app: &mut App, drop: FileDrop, dropped: &Dropped, tx: &mut Tx) -> bool {
    let FileDrop {
        destination,
        before,
    } = drop;
    if refuses_file_drop(app, &destination) {
        return false;
    }
    if let Some(content) = content::read(dropped) {
        if let Some(before) = before.filter(|_| content::takes(&destination, &content)) {
            before(app, tx);
        }
        return content::commit(app, destination, content, tx);
    }
    let Some(sources) = dropped_locations(&dropped.data) else {
        return false;
    };
    let sources = transferable_drop_sources(&destination, &sources);
    if sources.is_empty() {
        return false;
    }
    let commit = Commit {
        destination,
        sources,
        before,
    };
    let choices = Choices::of(app, dropped);
    if choices.settled() {
        return commit_unknown_volume(app, commit, choices.unknown, tx);
    }
    if !choices.possible(&commit.sources) {
        return false;
    }
    resolve_with_volume_lookup(app, commit, choices.same, tx);
    true
}

/// Commits a drop whose action the volumes cannot change, without looking them up.
fn commit_unknown_volume(
    app: &mut App,
    mut commit: Commit,
    unknown: ActionInput,
    tx: &mut Tx,
) -> bool {
    let Some(action) = choose_action(unknown).filter(|&a| can_commit_action(a, &commit.sources))
    else {
        return false;
    };
    commit.begin(app, tx);
    apply_drop_action(
        app,
        commit.destination,
        commit.sources,
        (action, FsVolumeRelation::Unknown),
    );
    shell::settle(app, tx);
    true
}

fn resolve_with_volume_lookup(app: &mut App, mut commit: Commit, base: ActionInput, tx: &mut Tx) {
    commit.begin(app, tx);
    let Commit {
        destination,
        sources,
        ..
    } = commit;
    let query = DropVolumeQuery::new(&destination, &sources);
    let volumes = app
        .dnd
        .volumes
        .take_matching(&query)
        .unwrap_or_else(|| lookup_drop_volumes(&query, |_| {}));
    match volumes {
        DropVolumes::Ready(lookup) => {
            apply_resolved_lookup(app, (destination, sources), base, lookup.relation);
        }
        DropVolumes::Pending(pending) => {
            let later = transfer::later(app);
            pending.wait(move |relation| {
                later.then(move |app, tx| {
                    apply_resolved_lookup(app, (destination, sources), base, relation);
                    shell::settle(app, tx);
                    true
                });
            });
        }
    }
    shell::settle(app, tx);
}

fn apply_resolved_lookup(
    app: &mut App,
    (destination, sources): (Location, Vec<Location>),
    base: ActionInput,
    relation: FsVolumeRelation,
) {
    let input = ActionInput::new(
        base.mods,
        base.offered,
        dnd_volume(relation),
        base.cross_device,
    );
    if let Some(action) = choose_action(input).filter(|&a| can_commit_action(a, &sources)) {
        apply_drop_action(app, destination, sources, (action, relation));
    }
}

pub(crate) fn apply_drop_action(
    app: &mut App,
    destination: Location,
    sources: Vec<Location>,
    (action, relation): (DropAction, FsVolumeRelation),
) {
    match action {
        DropAction::Copy => transfer::start_drop_transfer(app, destination, sources, false),
        DropAction::Move => transfer::start_drop_transfer(app, destination, sources, true),
        DropAction::Ask => drop_dialog::prompt(app, destination, sources, relation),
        DropAction::Link => {}
    }
}

pub(crate) fn can_trash_sources(sources: &[Location]) -> bool {
    !sources.is_empty()
        && sources.iter().all(|source| {
            source.parent().is_some()
                && !clipboard::paths::is_trash(source)
                && !source.is_administrator()
                && !source.uri_value().is_some_and(has_web_scheme)
        })
}

pub(crate) fn trash_action(app: &App, offer: &DropOffer) -> Option<DropAction> {
    if drops_blocked(app) || !offer.actions.contains(DropActions::MOVE) {
        return None;
    }
    match offer.uris.as_deref() {
        Some(uris) => {
            let sources = locations_from_uris(uris)?;
            (sources.len() == uris.len() && can_trash_sources(&sources)).then_some(DropAction::Move)
        }
        None => offer.offers(URI_LIST).then_some(DropAction::Move),
    }
}

pub(crate) fn commit_trash_drop(app: &mut App, dropped: &Dropped, tx: &mut Tx) -> bool {
    if drops_blocked(app) {
        return false;
    }
    let Some(sources) = dropped_locations(&dropped.data) else {
        return false;
    };
    if !can_trash_sources(&sources) {
        return false;
    }
    let later = transfer::later(app);
    glib::MainContext::default().spawn_local(async move {
        let mut entries = Vec::with_capacity(sources.len());
        for source in sources {
            match query_file_entry(source).await {
                Ok(entry) => entries.push(entry),
                Err(error) => {
                    let message = error.to_string();
                    later.then(move |app, _| {
                        error_dialog::show(app, Error::new(tr("Unable to move to Trash"), message));
                        true
                    });
                    return;
                }
            }
        }
        later.then(move |app, tx| {
            shell::publish(app);
            crate::trash::trash_dropped(app, entries);
            shell::settle(app, tx);
            true
        });
    });
    shell::settle(app, tx);
    true
}

pub(crate) fn can_commit_action(action: DropAction, sources: &[Location]) -> bool {
    match action {
        DropAction::Move => sources.iter().all(clipboard::paths::can_remove),
        DropAction::Copy | DropAction::Ask => true,
        DropAction::Link => false,
    }
}

pub(crate) fn cross_device(app: &App) -> CrossDevice {
    match CrossVolumeDropStrategy::parse(&shell::preferences(app).cross_volume_drop_strategy) {
        CrossVolumeDropStrategy::Copy => CrossDevice::Copy,
        CrossVolumeDropStrategy::Move => CrossDevice::Move,
        CrossVolumeDropStrategy::Ask => CrossDevice::Ask,
    }
}

pub(crate) fn dnd_volume(relation: FsVolumeRelation) -> VolumeRelation {
    match relation {
        FsVolumeRelation::Same => VolumeRelation::Same,
        FsVolumeRelation::Different => VolumeRelation::Different,
        FsVolumeRelation::Unknown => VolumeRelation::Unknown,
    }
}

pub(crate) fn dropped_locations(data: &DropData) -> Option<Vec<Location>> {
    let local_uris = data
        .local
        .as_ref()
        .map(|payload| payload.uris.as_slice())
        .filter(|uris| !uris.is_empty());
    match local_uris {
        Some(uris) => locations_from_uris(uris),
        None => locations_from_uris(&data.uris()),
    }
}

pub(crate) fn locations_from_uris(uris: &[String]) -> Option<Vec<Location>> {
    if uris.is_empty() || uris.iter().any(|uri| has_web_scheme(uri)) {
        return None;
    }
    let locations: Vec<Location> = uris
        .iter()
        .filter_map(|uri| location_for_file(&gio::File::for_uri(uri)))
        .collect();
    (!locations.is_empty()).then_some(locations)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SpringTarget {
    SidebarRow {
        tab: TabKey,
        item: ItemId,
    },
    ViewRow {
        tab: TabKey,
        depth: usize,
        id: EntryId,
    },
}

#[derive(Default)]
pub(crate) struct Spring {
    state: Option<(SpringTarget, f64, bool)>,
}

impl Spring {
    pub(crate) fn clear(&mut self) {
        self.state = None;
    }
}

fn current_spring_target(app: &App) -> Option<SpringTarget> {
    if !super::is_dragging(app) || drops_blocked(app) {
        return None;
    }
    let tab = shell::tabs::shown_tab(app);
    if let Some((SidebarDropSite::Item(item), DropHover::Accepting | DropHover::Rejecting)) =
        app.sidebar.drop_over()
    {
        let is_trash = matches!(item, ItemId::Place(id) if id == "trash");
        if !is_trash {
            return Some(SpringTarget::SidebarRow {
                tab,
                item: item.clone(),
            });
        }
    }
    view_spring_target(app, tab)
}

fn view_spring_target(app: &App, tab: TabKey) -> Option<SpringTarget> {
    let view = shell::shown_view(app)?;
    if (view.shows)("columns") {
        let (depth, id) = app.columns.hovered_folder_drop()?;
        return Some(SpringTarget::ViewRow { tab, depth, id });
    }
    if app.filter.pane.shows() {
        return None;
    }
    let pane = if (view.shows)("icons") {
        &app.icons.single.pane
    } else if (view.shows)("list") {
        &app.list.single.pane
    } else {
        return None;
    };
    let depth = pane.depth()?;
    match pane.collection().drop_over() {
        Some(DropOver::Row(id)) => Some(SpringTarget::ViewRow { tab, depth, id }),
        _ => None,
    }
}

pub(crate) fn poll_spring(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let now = current_spring_target(app);
    match (&app.dnd.spring.state, now) {
        (Some((target, since, fired)), Some(current)) if *target == current => {
            if !*fired && clock - *since >= SPRING_DELAY_SECS {
                let target = target.clone();
                app.dnd.spring.state = Some((target.clone(), *since, true));
                fire_spring(app, target, tx);
                return true;
            }
            if !*fired {
                let rem = (SPRING_DELAY_SECS - (clock - *since)).max(0.0);
                tx.wake_after(Duration::from_secs_f64(rem));
            }
            false
        }
        (_, Some(current)) => {
            app.dnd.spring.state = Some((current, clock, false));
            tx.wake_after(Duration::from_secs_f64(SPRING_DELAY_SECS));
            false
        }
        (_, None) => {
            app.dnd.spring.clear();
            false
        }
    }
}

fn fire_spring(app: &mut App, target: SpringTarget, tx: &mut Tx) {
    if target_tab(&target) != shell::tabs::shown_tab(app) {
        return;
    }
    match target {
        SpringTarget::SidebarRow { item, .. } => crate::sidebar::spring_open(app, &item, tx),
        SpringTarget::ViewRow { depth, id, .. } => {
            let columns = shell::shown_view(app).is_some_and(|view| (view.shows)("columns"));
            if columns {
                crate::columns::spring_open(app, depth, id, tx);
            } else {
                crate::single::spring_open(app, depth, id, tx);
            }
        }
    }
}

fn target_tab(target: &SpringTarget) -> TabKey {
    match target {
        SpringTarget::SidebarRow { tab, .. } | SpringTarget::ViewRow { tab, .. } => *tab,
    }
}
