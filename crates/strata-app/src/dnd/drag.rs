// SPDX-License-Identifier: MIT

//! What an in-window drag is made of: the gesture that starts it, what it carries
//! ([`Launch`]), the drag under way ([`InWindowDrag`]), and the hit-tests the router reads
//! back about it.

use std::rc::Rc;

use rune::{Drag, DragOutcome, Tx};
use strata_components::contract::dnd::{DragPayload, DropOffer};
use strata_platform::dnd::DragIcon;

use super::registry::Landing;
use crate::{App, shell, slots::TabKey};

pub(super) type EndFn = Rc<dyn Fn(&mut App, DragOutcome, &mut Tx)>;

/// A pointer gesture on a drag source: the drag, and the source element's key.
#[derive(Clone, Copy)]
pub(crate) struct Gesture<'a> {
    pub(crate) drag: &'a Drag,
    pub(crate) source: &'a str,
}

/// What a drag carries, how it looks, and what runs when it ends.
pub(crate) struct Launch {
    pub(super) payload: DragPayload,
    pub(super) icon: Option<DragIcon>,
    pub(super) on_end: EndFn,
}

impl Launch {
    pub(crate) fn new(
        payload: DragPayload,
        on_end: impl Fn(&mut App, DragOutcome, &mut Tx) + 'static,
    ) -> Self {
        Self {
            payload,
            icon: None,
            on_end: Rc::new(on_end),
        }
    }

    pub(crate) fn icon(mut self, icon: DragIcon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// An element hit-test's answer for the drag numbered `serial` (its `seq` or its `id`).
#[derive(Clone, Copy)]
pub(super) struct Hit<'a> {
    pub(super) serial: u64,
    pub(super) path: Option<&'a str>,
}

/// A drag's landing before and after a move.
pub(super) struct Change {
    pub(super) old: Landing,
    pub(super) new: Landing,
    pub(super) offer: DropOffer,
    pub(super) at: [f32; 2],
}

pub(super) struct InWindowDrag {
    pub(super) id: u64,
    pub(super) source_key: String,
    pub(super) offer: DropOffer,
    pub(super) payload: DragPayload,
    pub(super) at: [f32; 2],
    pub(super) landing: Landing,
    pub(super) seq: u64,
    pub(super) origin_tab: TabKey,
    pub(super) tab_tail: Option<Vec<String>>,
    /// Where the stand-in that keeps Rune's held press alive is ([`StandIn`]).
    pub(super) stand_in: StandIn,
    pub(super) on_end: Option<EndFn>,
}

/// The hidden stand-in for a drag's source while the source is not drawn (its tab hidden, its
/// column closed): an element at the source's path, which Rune's held press then reaches.
///
/// It is mounted one commit after the source leaves, never in the commit that removes it, and
/// that commit unmounts the source at once. Rune gives a node at a path it has seen the id it
/// had there (`path_ids` in its commit), and its scene is keyed by id: a stand-in mounted in
/// the source's place keeps the source's scene entry under the source's old ancestors, and
/// loses it when they go. `Runtime::node` then counts it unmounted, and the held press's moves
/// and release reach no handler (A4, `tabs/hovered-tab-takes-drop`; Rune packet entry 109,
/// `docs/rune/upstream/rune/109-remount-at-a-seen-path.md`).
///
/// For the one frame between, no live node holds the press's path: a move in that frame is
/// lost, which the next move makes up, and a release in exactly that frame loses the drop.
/// That is accepted; the frame after is woken at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum StandIn {
    /// The source is drawn; no stand-in.
    #[default]
    Waiting,
    /// The source left in this frame's commit: the stand-in comes in the next.
    Due,
    /// The stand-in is mounted.
    Mounted,
}

impl StandIn {
    /// The next state, given whether the drag still needs a stand-in; and whether the view has
    /// to be built again to mount it.
    pub(super) fn step(self, needed: bool) -> (Self, bool) {
        match (self, needed) {
            (_, false) => (Self::Waiting, false),
            (Self::Waiting, true) => (Self::Due, false),
            (Self::Due | Self::Mounted, true) => (Self::Mounted, self == Self::Due),
        }
    }
}

/// Where to look for the drag source's path in the tree, to keep it mounted while the drag runs.
pub(super) struct TailProbe {
    pub(super) id: u64,
    pub(super) tab: TabKey,
    pub(super) source_key: String,
    pub(super) at: (f32, f32),
}

impl TailProbe {
    /// The keyed segments of `path` from the drag's tab down to its source element.
    pub(super) fn tail_in(&self, path: &str) -> Option<Vec<String>> {
        let tab_seg = format!("tab-{}", self.tab.0);
        let segs: Vec<&str> = path.split('/').collect();
        let start = segs.iter().position(|s| *s == tab_seg)?;
        let end = segs[start..].iter().position(|s| *s == self.source_key)?;
        Some(
            segs[start..=start + end]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        )
    }
}

impl InWindowDrag {
    fn started_in_preview(&self) -> bool {
        self.source_key == "preview-handle" || self.source_key.ends_with("-picture")
    }

    fn started_in_single_pane(&self) -> bool {
        self.source_key == "icons" || self.source_key == "list"
    }

    /// Whether the element the drag started from is still drawn.
    pub(super) fn source_mounted(&self, app: &App) -> bool {
        if self.started_in_preview() {
            return crate::preview::current_entry(app).is_some();
        }
        self.view_source_mounted(app)
    }

    /// Whether the shown view still draws the view row the drag started from.
    fn view_source_mounted(&self, app: &App) -> bool {
        let Some(view) = shell::shown_view(app) else {
            return false;
        };
        let key = self.source_key.as_str();
        if key.starts_with("columns-col-") {
            return (view.shows)("columns") && app.columns.has_column_key(key);
        }
        if self.started_in_single_pane() {
            let from_results = self
                .tab_tail
                .iter()
                .flatten()
                .any(|seg| seg == "filter-results");
            return (view.shows)(key) && app.filter.pane.shows() == from_results;
        }
        true
    }
}
