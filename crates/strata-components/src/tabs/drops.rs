// SPDX-License-Identifier: MIT

//! The strip's W1 drop zones: one per tab, the New tab button's, and the strip's own for the
//! gaps between tabs and unused space. Each is attached through the host's `Dnd` binder, on an
//! element keyed for X2's ([`super::tab_key`], [`super::new_tab_key`], the strip's key).
//!
//! - A **file or content** drag over an accepting tab tints it and arms its spring (unless it is
//!   the active tab); a drop there is [`TabsIntent::DropInto`].
//! - A drag that opens tabs [`TabOpen::AtGap`] (sidebar rows) neither tints nor springs a tab:
//!   its moves place rune-kit's incoming gap ([`rune_kit::shell::TabDragState::set_incoming`])
//!   across the strip, and its drop is [`TabsIntent::OpenTabs`] at the gap, or at the end on New
//!   tab.
//! - A drag that opens tabs [`TabOpen::AtEnd`] (folder-only view drags) transfers and springs on
//!   accepting tabs, is refused on Trash/Recent tabs, and opens at the end on New tab and unused
//!   strip space.

use std::rc::Rc;

use rune::{Mods, Tx};

use super::BrowserTabsState;
use super::data::{Accepts, TabId};
use super::intent::{DropSite, TabOpen, TabsIntent};
use crate::contract::dnd::{
    ActionInput, CrossDevice, DropAction, DropHover, DropOffer, DropZone, URI_LIST, VolumeRelation,
    choose_action,
};

pub(super) type Emit<S> = Rc<dyn Fn(&mut S, TabsIntent, &mut Tx)>;
pub(super) type ActionFn<S> = Rc<dyn Fn(&S, TabId, &DropOffer, Mods) -> Option<DropAction>>;
pub(super) type OpensFn<S> = Rc<dyn Fn(&S, &DropOffer) -> Option<TabOpen>>;
pub(super) type Access<S> = fn(&mut S) -> &mut BrowserTabsState;

/// One tab's drop-zone parameters.
#[derive(Clone, Copy)]
pub(super) struct TabSpec {
    pub id: TabId,
    pub index: usize,
    pub accepts: Accepts,
    pub active: bool,
}

/// What every zone of one strip shares.
pub(super) struct Ctx<S> {
    pub access: Access<S>,
    pub emit: Emit<S>,
    pub action: Option<ActionFn<S>>,
    pub opens: Option<OpensFn<S>>,
    /// Whether `Accepts::NONE` tabs attach a zone that accepts [`TabOpen::AtGap`] and stops
    /// [`TabOpen::AtEnd`] from falling through to the strip (`BrowserTabs::tab_open`).
    pub guard_none: bool,
    pub content: &'static [&'static str],
    pub spring: Option<f64>,
    pub len: usize,
}

impl<S> Clone for Ctx<S> {
    fn clone(&self) -> Self {
        Ctx {
            access: self.access,
            emit: self.emit.clone(),
            action: self.action.clone(),
            opens: self.opens.clone(),
            guard_none: self.guard_none,
            content: self.content,
            spring: self.spring,
            len: self.len,
        }
    }
}

impl<S: 'static> Ctx<S> {
    fn tab_open(&self, s: &S, offer: &DropOffer) -> Option<TabOpen> {
        self.opens.as_ref().and_then(|f| f(s, offer))
    }

    fn opens_at_gap(&self, s: &S, offer: &DropOffer) -> bool {
        self.tab_open(s, offer) == Some(TabOpen::AtGap)
    }
}

/// W1's default rule, for a zone the host gave no action.
fn default_action(offer: &DropOffer, mods: Mods) -> Option<DropAction> {
    choose_action(ActionInput::new(
        mods,
        offer.actions,
        VolumeRelation::Unknown,
        CrossDevice::default(),
    ))
}

fn tab_mimes(accepts: Accepts, content: &'static [&'static str]) -> Vec<&'static str> {
    let mut mimes = Vec::new();
    if accepts.files {
        mimes.push(URI_LIST);
    }
    if accepts.content {
        for &m in content {
            if !mimes.contains(&m) {
                mimes.push(m);
            }
        }
    }
    mimes
}

/// Tab `spec.id`'s zone: on a Trash or Recent tab (`Accepts::NONE`), either `None` (legacy
/// `opens_tabs`) or a zone that accepts only [`TabOpen::AtGap`] and refuses everything else.
pub(super) fn tab<S: 'static>(cx: &Ctx<S>, spec: TabSpec) -> Option<DropZone<S>> {
    let mimes = tab_mimes(spec.accepts, cx.content);
    if mimes.is_empty() {
        return cx.guard_none.then(|| gap_only_tab(cx, spec));
    }
    let site = DropSite::Tab(spec.id);
    let (drop_cx, hover_cx, action_cx) = (cx.clone(), cx.clone(), cx.clone());
    let access = cx.access;
    let zone = DropZone::new(move |s: &mut S, dropped, tx: &mut Tx| {
        let opens = drop_cx.opens_at_gap(s, &dropped.offer);
        let landing = (drop_cx.access)(s).landing();
        (drop_cx.access)(s).drag_ended();
        let intent = if opens {
            // Rune's binder reads a drag's paths only at the drop (U3), so a drag it couldn't
            // tell opened tabs while hovering placed no gap: it opens after this tab.
            let at = landing.unwrap_or(spec.index + 1);
            TabsIntent::OpenTabs { at, dropped }
        } else {
            TabsIntent::DropInto {
                id: spec.id,
                dropped,
            }
        };
        (drop_cx.emit)(s, intent, tx);
    })
    .mime_types(&mimes)
    .on_hover(move |s: &mut S, hover, offer: &DropOffer, tx: &mut Tx| {
        let over = hover != DropHover::Left;
        if hover_cx.opens_at_gap(s, offer) {
            (hover_cx.access)(s).follow_open(site, over, tx);
        } else {
            let st = (hover_cx.access)(s);
            st.set_drop_hover(site, hover);
            let arm = hover_cx.spring.filter(|_| !spec.active);
            if let (true, Some(delay)) = (st.hover_tab(spec.id, hover, arm, tx.clock()), arm) {
                tx.wake_at(tx.clock() + delay);
            }
        }
        let offer = offer.clone();
        (hover_cx.emit)(s, TabsIntent::DropHover { site, hover, offer }, tx);
    })
    .on_reject(move |s: &mut S, _, _: &mut Tx| access(s).drag_ended())
    .action(move |s: &S, offer, mods| {
        if action_cx.opens_at_gap(s, offer) {
            return Some(DropAction::Copy);
        }
        match &action_cx.action {
            Some(f) => f(s, spec.id, offer, mods),
            None => default_action(offer, mods),
        }
    });
    Some(follow_moves(cx, zone, site))
}

/// A Trash or Recent tab (`Accepts::NONE`) under [`super::BrowserTabs::tab_open`]: takes a
/// sidebar drag ([`TabOpen::AtGap`]) at its gap, and refuses file, content and folder-only view
/// drags ([`TabOpen::AtEnd`]) without letting them fall through to [`DropSite::Strip`].
fn gap_only_tab<S: 'static>(cx: &Ctx<S>, spec: TabSpec) -> DropZone<S> {
    let site = DropSite::Tab(spec.id);
    let (drop_cx, hover_cx, action_cx) = (cx.clone(), cx.clone(), cx.clone());
    let access = cx.access;
    let zone = DropZone::new(move |s: &mut S, dropped, tx: &mut Tx| {
        let landing = (drop_cx.access)(s).landing();
        (drop_cx.access)(s).drag_ended();
        if !drop_cx.opens_at_gap(s, &dropped.offer) {
            return;
        }
        let at = landing.unwrap_or(spec.index + 1);
        (drop_cx.emit)(s, TabsIntent::OpenTabs { at, dropped }, tx);
    })
    .mime_types(&[])
    .local(true)
    .on_hover(move |s: &mut S, hover, offer: &DropOffer, tx: &mut Tx| {
        let over = hover != DropHover::Left;
        if hover_cx.opens_at_gap(s, offer) || !over {
            (hover_cx.access)(s).follow_open(site, over, tx);
        }
        let offer = offer.clone();
        (hover_cx.emit)(s, TabsIntent::DropHover { site, hover, offer }, tx);
    })
    .on_reject(move |s: &mut S, _, _: &mut Tx| access(s).drag_ended())
    .action(move |s: &S, offer, _| action_cx.opens_at_gap(s, offer).then_some(DropAction::Copy));
    follow_moves(cx, zone, site)
}

/// `zone`, placing the gap as a drag that opens tabs moves over it. Only for a host with such
/// drags: a zone that follows moves asks for a frame on each motion of every drag over it.
fn follow_moves<S: 'static>(cx: &Ctx<S>, zone: DropZone<S>, site: DropSite) -> DropZone<S> {
    if cx.opens.is_none() {
        return zone;
    }
    let access = cx.access;
    zone.on_move(move |s: &mut S, point, _: &mut Tx| {
        access(s).open_moved(site, [point.window_x, point.window_y]);
    })
}

/// The zone of the New tab button (`site` [`DropSite::NewTab`]) or of the strip
/// ([`DropSite::Strip`]): it takes drags that open tabs, opening [`TabOpen::AtGap`] at the gap
/// and [`TabOpen::AtEnd`] (or New tab) at the end. Any other drag is refused there.
pub(super) fn opener<S: 'static>(cx: &Ctx<S>, site: DropSite) -> DropZone<S> {
    let (drop_cx, hover_cx, action_cx) = (cx.clone(), cx.clone(), cx.clone());
    let access = cx.access;
    let zone = DropZone::new(move |s: &mut S, dropped, tx: &mut Tx| {
        let landing = (drop_cx.access)(s).landing();
        (drop_cx.access)(s).drag_ended();
        let Some(open) = drop_cx.tab_open(s, &dropped.offer) else {
            return;
        };
        let at = match (site, open) {
            (DropSite::Strip, TabOpen::AtGap) => landing.unwrap_or(drop_cx.len),
            _ => drop_cx.len,
        };
        (drop_cx.emit)(s, TabsIntent::OpenTabs { at, dropped }, tx);
    })
    .on_hover(move |s: &mut S, hover, offer: &DropOffer, tx: &mut Tx| {
        let over = hover != DropHover::Left;
        let open = hover_cx.tab_open(s, offer);
        let st = (hover_cx.access)(s);
        match (site, open) {
            (DropSite::Strip, Some(TabOpen::AtGap)) => st.follow_open(site, over, tx),
            (DropSite::Strip, _) if !over => st.follow_open(site, false, tx),
            // The button lights for a drag that opens tabs (and for any while the host says one
            // runs: `set_open_drag`).
            (DropSite::NewTab, _) if open.is_some() || !over => st.set_drop_hover(site, hover),
            _ => {}
        }
        let offer = offer.clone();
        (hover_cx.emit)(s, TabsIntent::DropHover { site, hover, offer }, tx);
    })
    .on_reject(move |s: &mut S, _, _: &mut Tx| access(s).drag_ended())
    .action(move |s: &S, offer, _| action_cx.tab_open(s, offer).map(|_| DropAction::Copy));
    // New tab opens at the end, wherever the drag is over it.
    match site {
        DropSite::Strip => follow_moves(cx, zone, site),
        _ => zone,
    }
}
