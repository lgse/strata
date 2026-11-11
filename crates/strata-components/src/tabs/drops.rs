//! The strip's W1 drop zones: one per tab, the New tab button's, and the strip's own for the
//! gaps between tabs. Each is attached through the host's `Dnd` binder, on an element keyed
//! for X2's ([`super::tab_key`], [`super::new_tab_key`], the strip's key).
//!
//! - A **file or content** drag over a tab tints it and arms its spring (unless it is the
//!   active tab); a drop there is [`TabsIntent::DropInto`].
//! - A drag the host says **opens tabs** ([`super::BrowserTabs::opens_tabs`]) neither tints
//!   nor springs a tab: its moves place rune-kit's incoming gap
//!   ([`rune_kit::shell::TabDragState::set_incoming`]) wherever it is over the strip, and its
//!   drop is [`TabsIntent::OpenTabs`] at the gap. On the New tab button it opens at the end.

use std::rc::Rc;

use rune::{Mods, Tx};

use super::BrowserTabsState;
use super::data::{Accepts, TabId};
use super::intent::{DropSite, TabsIntent};
use crate::contract::dnd::{
    ActionInput, CrossDevice, DropAction, DropHover, DropOffer, DropZone, URI_LIST, VolumeRelation,
    choose_action,
};

pub(super) type Emit<S> = Rc<dyn Fn(&mut S, TabsIntent, &mut Tx)>;
pub(super) type ActionFn<S> = Rc<dyn Fn(&S, TabId, &DropOffer, Mods) -> Option<DropAction>>;
pub(super) type OpensFn<S> = Rc<dyn Fn(&S, &DropOffer) -> bool>;
pub(super) type Access<S> = fn(&mut S) -> &mut BrowserTabsState;

/// What every zone of one strip shares.
pub(super) struct Ctx<S> {
    pub access: Access<S>,
    pub emit: Emit<S>,
    pub action: Option<ActionFn<S>>,
    pub opens: Option<OpensFn<S>>,
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
            content: self.content,
            spring: self.spring,
            len: self.len,
        }
    }
}

impl<S: 'static> Ctx<S> {
    fn opens(&self, s: &S, offer: &DropOffer) -> bool {
        self.opens.as_ref().is_some_and(|f| f(s, offer))
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

/// Tab `id`'s zone, at `index`: `None` when it takes nothing (Trash, Recent), so a drag passes
/// through it to the strip's zone, which takes only drags that open tabs.
pub(super) fn tab<S: 'static>(
    cx: &Ctx<S>,
    id: TabId,
    index: usize,
    accepts: Accepts,
    active: bool,
) -> Option<DropZone<S>> {
    let mut mimes: Vec<&'static str> = Vec::new();
    if accepts.files {
        mimes.push(URI_LIST);
    }
    if accepts.content {
        for &m in cx.content {
            if !mimes.contains(&m) {
                mimes.push(m);
            }
        }
    }
    if mimes.is_empty() {
        return None;
    }
    let site = DropSite::Tab(id);
    let drop_cx = cx.clone();
    let hover_cx = cx.clone();
    let action_cx = cx.clone();
    let access = cx.access;
    let zone = DropZone::new(move |s: &mut S, dropped, tx: &mut Tx| {
        let cx = &drop_cx;
        let opens = cx.opens(s, &dropped.offer);
        let landing = (cx.access)(s).landing();
        (cx.access)(s).drag_ended();
        let intent = if opens {
            // Rune's binder reads a drag's paths only at the drop (U3), so a drag it couldn't
            // tell opened tabs while hovering placed no gap: it opens after this tab.
            let at = landing.unwrap_or(index + 1);
            TabsIntent::OpenTabs { at, dropped }
        } else {
            TabsIntent::DropInto { id, dropped }
        };
        (cx.emit)(s, intent, tx);
    })
    .mime_types(&mimes)
    .on_hover(move |s: &mut S, hover, offer: &DropOffer, tx: &mut Tx| {
        let cx = &hover_cx;
        let over = hover != DropHover::Left;
        if cx.opens(s, offer) {
            (cx.access)(s).follow_open(site, over, tx);
        } else {
            let st = (cx.access)(s);
            st.set_drop_hover(site, hover);
            let arm = cx.spring.filter(|_| !active);
            if let (true, Some(delay)) = (st.hover_tab(id, hover, arm, tx.clock()), arm) {
                tx.wake_at(tx.clock() + delay);
            }
        }
        let offer = offer.clone();
        (cx.emit)(s, TabsIntent::DropHover { site, hover, offer }, tx);
    })
    .on_reject(move |s: &mut S, _, _: &mut Tx| access(s).drag_ended())
    .action(move |s: &S, offer, mods| {
        let cx = &action_cx;
        if cx.opens(s, offer) {
            return Some(DropAction::Copy);
        }
        match &cx.action {
            Some(f) => f(s, id, offer, mods),
            None => default_action(offer, mods),
        }
    });
    Some(follow_moves(cx, zone, site))
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
/// ([`DropSite::Strip`]): it takes only drags that open tabs, opening them at the end or at
/// the gap. Any other drag is refused there.
pub(super) fn opener<S: 'static>(cx: &Ctx<S>, site: DropSite) -> DropZone<S> {
    let drop_cx = cx.clone();
    let hover_cx = cx.clone();
    let action_cx = cx.clone();
    let access = cx.access;
    let zone = DropZone::new(move |s: &mut S, dropped, tx: &mut Tx| {
        let cx = &drop_cx;
        let landing = (cx.access)(s).landing();
        (cx.access)(s).drag_ended();
        if !cx.opens(s, &dropped.offer) {
            return;
        }
        let at = match site {
            DropSite::Strip => landing.unwrap_or(cx.len),
            _ => cx.len,
        };
        (cx.emit)(s, TabsIntent::OpenTabs { at, dropped }, tx);
    })
    .on_hover(move |s: &mut S, hover, offer: &DropOffer, tx: &mut Tx| {
        let cx = &hover_cx;
        let over = hover != DropHover::Left;
        let opens = cx.opens(s, offer);
        let st = (cx.access)(s);
        match site {
            DropSite::Strip if opens => st.follow_open(site, over, tx),
            // The button lights for a drag that opens tabs (and for any while the host says one
            // runs: `set_open_drag`).
            DropSite::NewTab if opens || !over => st.set_drop_hover(site, hover),
            _ => {}
        }
        let offer = offer.clone();
        (cx.emit)(s, TabsIntent::DropHover { site, hover, offer }, tx);
    })
    .on_reject(move |s: &mut S, _, _: &mut Tx| access(s).drag_ended())
    .action(move |s: &S, offer, _| action_cx.opens(s, offer).then_some(DropAction::Copy));
    // New tab opens at the end, wherever the drag is over it.
    match site {
        DropSite::Strip => follow_moves(cx, zone, site),
        _ => zone,
    }
}
