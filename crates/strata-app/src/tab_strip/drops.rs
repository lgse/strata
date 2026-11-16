// SPDX-License-Identifier: MIT

//! Drops on the tab strip and the single-tab header's New tab button (`tabs/strip.rs` and
//! `tabs/strip/drop.rs` in `src/ui/window/composition/`, with `src/ui/browser/clipboard.rs` and
//! `transfer/drop.rs`):
//!
//! - **Accepting tabs.** A tab showing a local folder takes files and content ([`Accepts::ALL`]);
//!   a remote folder takes files only ([`Accepts::FILES`]); Trash, Recent and a tab whose folder
//!   has not opened yet take neither ([`Accepts::NONE`]).
//! - **File drops (`DropInto`).** The same policy as every other drop zone (`dnd::policy`):
//!   same-parent and self/descendant drops are refused without switching tabs, the volumes
//!   are looked up as the drop's action is asked, and a drop that commits selects the tab
//!   first, then transfers into its active directory or asks "Copy or move?".
//! - **Opening tabs (`OpenTabs`).** Sidebar location drags ([`TabOpen::AtGap`]) place an incoming
//!   gap across tabs and open there; folder-only view drags ([`TabOpen::AtEnd`]) transfer on
//!   accepting tabs, stop on Trash/Recent, and open at the end on New tab and unused strip space.
//! - **Modal guard.** While any modal layer holds the keyboard, no spring arms, no drop transfers,
//!   and no tab opens.

use rune::{Mods, Tx};
use strata_components::{
    contract::dnd::{Dnd, DropAction, DropOffer, Dropped},
    tabs::{Accepts, TabId, TabOpen},
};
use strata_core::model::Location;

use crate::{
    App, clipboard,
    dnd::FileDrop,
    shell::{self, tabs as tabs_of},
    slots::{Setup, TabKey},
};

pub(super) struct Drops {
    pub(super) dnd: Option<Dnd<App>>,
    pub(super) open_drag: Option<TabOpen>,
}

impl Drops {
    pub(super) fn new(_: &Setup<'_>) -> Self {
        Self {
            dnd: None,
            open_drag: None,
        }
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
    let dest = destination(app, super::key(id))?;
    crate::dnd::drop_action(app, &dest, offer, mods)
}

/// Files dropped on tab `id` go into its folder, the tab selected first as GTK's drop does.
pub(super) fn drop_into(app: &mut App, id: TabId, dropped: Dropped, tx: &mut Tx) -> bool {
    let Some(dest) = destination(app, super::key(id)) else {
        return false;
    };
    let drop = FileDrop::to(dest).before(move |app: &mut App, tx: &mut Tx| {
        super::select(app, super::key(id), tx);
    });
    crate::dnd::commit(app, drop, &dropped, tx)
}

pub(super) fn open_tabs(app: &mut App, at: usize, dropped: Dropped, tx: &mut Tx) -> bool {
    if blocked(app) {
        return false;
    }
    let Some(locations) = crate::dnd::dropped_locations(&dropped.data) else {
        return false;
    };
    for (offset, location) in locations.into_iter().enumerate() {
        super::open_at(app, Some(location), at + offset, tx);
    }
    shell::settle(app, tx);
    true
}
