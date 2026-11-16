// SPDX-License-Identifier: MIT

//! Drops on the tab strip and the single-tab header's New tab button (`tabs/strip.rs` and
//! `tabs/strip/drop.rs` in `src/ui/window/composition/`, with `src/ui/browser/clipboard.rs` and
//! `transfer/drop.rs`):
//!
//! - **Accepting tabs.** A tab showing a local folder takes files and content ([`Accepts::ALL`]);
//!   a remote folder takes files only ([`Accepts::FILES`]); Trash, Recent and a tab whose folder
//!   has not opened yet take neither ([`Accepts::NONE`]).
//! - **File drops (`DropInto`).** Same-parent and self/descendant drops are refused without
//!   switching tabs ([`transferable_drop_sources`]). Hovering with readable URIs looks up the
//!   destination and source volumes ([`lookup_drop_volumes`]) and caches the result; dropping
//!   selects the tab and transfers into its active directory via [`transfer::start_drop_transfer`],
//!   waiting on the volume lookup when a plain drop under `always-ask` needs it. Cross-volume
//!   confirmation (`DropAction::Ask`) and foreign content saving wait on A19.
//! - **Opening tabs (`OpenTabs`).** Sidebar location drags ([`TabOpen::AtGap`]) place an incoming
//!   gap across tabs and open there; folder-only view drags ([`TabOpen::AtEnd`]) transfer on
//!   accepting tabs, stop on Trash/Recent, and open at the end on New tab and unused strip space.
//! - **Modal guard.** While any modal layer holds the keyboard, no spring arms, no drop transfers,
//!   and no tab opens.

use rune::{Mods, Tx};
use strata_components::{
    contract::dnd::{
        ActionInput, CrossDevice, Dnd, DropAction, DropData, DropHover, DropOffer, Dropped,
        VolumeRelation, choose_action,
    },
    tabs::{Accepts, DropSite, TabId, TabOpen},
};
use strata_core::model::Location;
use strata_fs::{
    adapters::{DropVolumeQuery, DropVolumes, location_for_file, lookup_drop_volumes},
    services::{CrossVolumeDropStrategy, WebLink, has_web_scheme, transferable_drop_sources},
};

use crate::{
    App, clipboard,
    shell::{self, tabs as tabs_of},
    slots::{Later, Setup, TabKey},
    transfer,
};

pub(super) struct Drops {
    pub(super) dnd: Option<Dnd<App>>,
    pub(super) open_drag: Option<TabOpen>,
    later: Later<App>,
    volumes: Option<HoverVolumes>,
}

struct HoverVolumes {
    serial: u64,
    query: DropVolumeQuery,
    state: DropVolumes,
}

struct TabDrop {
    id: TabId,
    dest: Location,
    sources: Vec<Location>,
}

impl Drops {
    pub(super) fn new(setup: &Setup<'_>) -> Self {
        Self {
            dnd: None,
            open_drag: None,
            later: setup.later.clone(),
            volumes: None,
        }
    }

    pub(super) fn clear_hover(&mut self) {
        self.volumes = None;
    }

    fn cached_relation(&self, serial: u64, query: &DropVolumeQuery) -> VolumeRelation {
        let Some(cached) = self.volumes.as_ref() else {
            return VolumeRelation::Unknown;
        };
        if cached.serial == serial && &cached.query == query {
            dnd_volume(cached.state.relation())
        } else {
            VolumeRelation::Unknown
        }
    }

    fn take_matching(&mut self, query: &DropVolumeQuery) -> Option<HoverVolumes> {
        let matches = self.volumes.as_ref().is_some_and(|v| &v.query == query);
        matches.then(|| self.volumes.take()).flatten()
    }

    fn ensure_lookup(&mut self, serial: u64, query: DropVolumeQuery) {
        let covered = self
            .volumes
            .as_ref()
            .is_some_and(|v| v.serial == serial && v.query == query);
        if covered {
            return;
        }
        let later = self.later.clone();
        let expected = query.clone();
        let state = lookup_drop_volumes(&query, move |lookup| {
            later.then(move |app, _| {
                let slot = app.tab_strip.drops.volumes.as_mut();
                if let Some(cached) = slot.filter(|v| v.serial == serial && v.query == expected) {
                    cached.state = DropVolumes::Ready(lookup);
                    return true;
                }
                false
            });
        });
        self.volumes = Some(HoverVolumes {
            serial,
            query,
            state,
        });
    }
}

/// Whether a modal layer is open in the window (`TabWindow::blocked`).
pub(super) fn blocked(app: &App) -> bool {
    shell::modal_open(app)
}

fn destination(app: &App, key: TabKey) -> Option<Location> {
    tabs_of::tab_browser(app, key)?
        .active_location()
        .filter(|loc| !clipboard::paths::is_trash(loc) && !loc.is_recent_location())
}

pub(super) fn accepts(app: &App, key: TabKey) -> Accepts {
    let Some(loc) = destination(app, key) else {
        return Accepts::NONE;
    };
    if loc.native_path().is_some() {
        Accepts::ALL
    } else {
        Accepts::FILES
    }
}

pub(super) fn tab_open(app: &App, offer: &DropOffer) -> Option<TabOpen> {
    (!blocked(app) && offer.local)
        .then_some(app.tab_strip.drops.open_drag)
        .flatten()
}

pub(super) fn drop_action(
    app: &App,
    id: TabId,
    offer: &DropOffer,
    mods: Mods,
) -> Option<DropAction> {
    if blocked(app) {
        return None;
    }
    let dest = destination(app, super::key(id))?;
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
        let valid_links = dest.native_path().is_some()
            && !uris.is_empty()
            && uris.iter().all(|uri| WebLink::parse(uri).is_some());
        return valid_links.then_some(DropAction::Copy);
    }
    let sources = locations_from_uris(uris)?;
    let transferable = transferable_drop_sources(&dest, &sources);
    if transferable.is_empty() {
        return None;
    }
    let query = DropVolumeQuery::new(&dest, &transferable);
    let relation = app.tab_strip.drops.cached_relation(offer.serial, &query);
    let action = choose_action(ActionInput::new(mods, offer.actions, relation, cross))?;
    can_commit_action(action, &transferable).then_some(action)
}

pub(super) fn hover(app: &mut App, site: DropSite, hover: DropHover, offer: DropOffer) -> bool {
    let (DropSite::Tab(id), DropHover::Accepting, Some(uris)) =
        (site, hover, offer.uris.as_deref())
    else {
        return false;
    };
    if blocked(app) {
        return false;
    }
    let Some(dest) = destination(app, super::key(id)) else {
        return false;
    };
    let Some(sources) = locations_from_uris(uris) else {
        return false;
    };
    let transferable = transferable_drop_sources(&dest, &sources);
    if !transferable.is_empty() {
        let query = DropVolumeQuery::new(&dest, &transferable);
        app.tab_strip.drops.ensure_lookup(offer.serial, query);
    }
    false
}

pub(super) fn drop_into(app: &mut App, id: TabId, dropped: Dropped, tx: &mut Tx) -> bool {
    if blocked(app) {
        return false;
    }
    let Some(dest) = destination(app, super::key(id)) else {
        return false;
    };
    let Some(sources) = dropped_locations(&dropped.data) else {
        return false;
    };
    let sources = transferable_drop_sources(&dest, &sources);
    if sources.is_empty() {
        return false;
    }
    let query = DropVolumeQuery::new(&dest, &sources);
    let cross = cross_device(app);
    let cached = app.tab_strip.drops.take_matching(&query);
    if let Some(HoverVolumes {
        state: DropVolumes::Ready(lookup),
        ..
    }) = &cached
    {
        let input = ActionInput::new(
            dropped.mods,
            dropped.offer.actions,
            dnd_volume(lookup.relation),
            cross,
        );
        return commit_resolved(
            app,
            TabDrop { id, dest, sources },
            (input, lookup.relation),
            tx,
        );
    }
    let same = ActionInput::new(
        dropped.mods,
        dropped.offer.actions,
        VolumeRelation::Same,
        cross,
    );
    let unknown = ActionInput::new(
        dropped.mods,
        dropped.offer.actions,
        VolumeRelation::Unknown,
        cross,
    );
    if choose_action(same) == choose_action(unknown) {
        return commit_resolved(
            app,
            TabDrop { id, dest, sources },
            (unknown, strata_fs::services::VolumeRelation::Unknown),
            tx,
        );
    }
    commit_after_lookup(
        app,
        TabDrop { id, dest, sources },
        (query, cached, same),
        tx,
    )
}

fn commit_resolved(
    app: &mut App,
    drop: TabDrop,
    (input, relation): (ActionInput, strata_fs::services::VolumeRelation),
    tx: &mut Tx,
) -> bool {
    let Some(action) = choose_action(input).filter(|&a| can_commit_action(a, &drop.sources)) else {
        return false;
    };
    super::select(app, super::key(drop.id), tx);
    crate::dnd::apply_drop_action(app, drop.dest, drop.sources, (action, relation));
    shell::settle(app, tx);
    true
}

fn commit_after_lookup(
    app: &mut App,
    drop: TabDrop,
    (query, cached, base): (DropVolumeQuery, Option<HoverVolumes>, ActionInput),
    tx: &mut Tx,
) -> bool {
    let unknown = ActionInput::new(
        base.mods,
        base.offered,
        VolumeRelation::Unknown,
        base.cross_device,
    );
    let possible = [base, unknown]
        .into_iter()
        .any(|input| choose_action(input).is_some_and(|a| can_commit_action(a, &drop.sources)));
    if !possible {
        return false;
    }
    super::select(app, super::key(drop.id), tx);
    let volumes = match cached {
        Some(HoverVolumes { state, .. }) => state,
        None => lookup_drop_volumes(&query, |_| {}),
    };
    match volumes {
        DropVolumes::Ready(lookup) => {
            apply_lookup_result(app, (drop.dest, drop.sources), base, lookup.relation);
        }
        DropVolumes::Pending(pending) => {
            let later = transfer::later(app);
            pending.wait(move |relation| {
                later.then(move |app, _| {
                    apply_lookup_result(app, (drop.dest, drop.sources), base, relation);
                    true
                });
            });
        }
    }
    shell::settle(app, tx);
    true
}

fn apply_lookup_result(
    app: &mut App,
    (dest, sources): (Location, Vec<Location>),
    base: ActionInput,
    relation: strata_fs::services::VolumeRelation,
) {
    let input = ActionInput::new(
        base.mods,
        base.offered,
        dnd_volume(relation),
        base.cross_device,
    );
    if let Some(action) = choose_action(input).filter(|&a| can_commit_action(a, &sources)) {
        crate::dnd::apply_drop_action(app, dest, sources, (action, relation));
    }
}

pub(super) fn open_tabs(app: &mut App, at: usize, dropped: Dropped, tx: &mut Tx) -> bool {
    if blocked(app) {
        return false;
    }
    let Some(locations) = dropped_locations(&dropped.data) else {
        return false;
    };
    for (offset, location) in locations.into_iter().enumerate() {
        super::open_at(app, Some(location), at + offset, tx);
    }
    shell::settle(app, tx);
    true
}

fn can_commit_action(action: DropAction, sources: &[Location]) -> bool {
    match action {
        DropAction::Move => sources.iter().all(clipboard::paths::can_remove),
        DropAction::Copy | DropAction::Ask => true,
        DropAction::Link => false,
    }
}

fn cross_device(app: &App) -> CrossDevice {
    match CrossVolumeDropStrategy::parse(&shell::preferences(app).cross_volume_drop_strategy) {
        CrossVolumeDropStrategy::Copy => CrossDevice::Copy,
        CrossVolumeDropStrategy::Move => CrossDevice::Move,
        CrossVolumeDropStrategy::Ask => CrossDevice::Ask,
    }
}

fn dnd_volume(relation: strata_fs::services::VolumeRelation) -> VolumeRelation {
    match relation {
        strata_fs::services::VolumeRelation::Same => VolumeRelation::Same,
        strata_fs::services::VolumeRelation::Different => VolumeRelation::Different,
        strata_fs::services::VolumeRelation::Unknown => VolumeRelation::Unknown,
    }
}

fn dropped_locations(data: &DropData) -> Option<Vec<Location>> {
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

fn locations_from_uris(uris: &[String]) -> Option<Vec<Location>> {
    if uris.is_empty() || uris.iter().any(|uri| has_web_scheme(uri)) {
        return None;
    }
    let locations: Vec<Location> = uris
        .iter()
        .filter_map(|uri| location_for_file(&gio::File::for_uri(uri)))
        .collect();
    (!locations.is_empty()).then_some(locations)
}
