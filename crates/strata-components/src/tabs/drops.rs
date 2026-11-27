// SPDX-License-Identifier: MIT

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
                tx.wake_after(crate::contract::dnd::spring_wake(tx.clock(), delay));
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

fn follow_moves<S: 'static>(cx: &Ctx<S>, zone: DropZone<S>, site: DropSite) -> DropZone<S> {
    if cx.opens.is_none() {
        return zone;
    }
    let access = cx.access;
    zone.on_move(move |s: &mut S, point, _: &mut Tx| {
        access(s).open_moved(site, [point.window_x, point.window_y]);
    })
}

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
