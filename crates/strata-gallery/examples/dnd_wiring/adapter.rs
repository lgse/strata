// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use rune::{Drag, DragOutcome, El, Mods, PointerEvent, Tx};
use strata_components::contract::Dnd as Binder;
use strata_components::contract::dnd::{
    DragPayload, DragSource, DropAction, DropActions, DropData, DropHover, DropItem, DropOffer,
    DropZone, Dropped, ElementKey, URI_LIST, parse_uri_list,
};
use strata_platform::dnd::{
    Actions, DragAction, DragIcon, DragItem, DropData as P1Data, DropOffer as P1Offer, DropPoint,
    DropTarget, Error, Hover,
};

pub const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
/// KDE's cut flag beside `text/uri-list`: `0` for a copy. Dolphin reads it.
pub const KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";

pub fn encode_uri_list(uris: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for uri in uris {
        out.extend_from_slice(uri.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// `x-special/gnome-copied-files` for a copy: `copy`, then a newline before each URI.
pub fn encode_gnome_copied_files(uris: &[String]) -> Vec<u8> {
    let mut out = b"copy".to_vec();
    for uri in uris {
        out.push(b'\n');
        out.extend_from_slice(uri.as_bytes());
    }
    out
}

pub fn drag_item(payload: &DragPayload, icon: Option<DragIcon>) -> DragItem {
    let uris: Arc<[String]> = payload.uris.clone().into();
    let (a, b) = (uris.clone(), uris);
    let mut item = DragItem::new()
        .lazy(URI_LIST, move || encode_uri_list(&a))
        .lazy(GNOME_COPIED_FILES, move || encode_gnome_copied_files(&b))
        .data(KDE_CUT_SELECTION, b"0".to_vec())
        .actions(actions_to_p1(payload.actions))
        .local(payload.clone());
    if let Some(icon) = icon {
        item = item.icon(icon);
    }
    item
}

pub fn actions_to_p1(a: DropActions) -> Actions {
    let mut out = Actions::NONE;
    for (w1, p1) in [
        (DropActions::COPY, Actions::COPY),
        (DropActions::MOVE, Actions::MOVE),
        (DropActions::LINK, Actions::LINK),
    ] {
        if a.contains(w1) {
            out = out | p1;
        }
    }
    out
}

pub fn actions_of_p1(a: Actions) -> DropActions {
    let mut out = DropActions::NONE;
    for (p1, w1) in [
        (DragAction::Copy, DropActions::COPY),
        (DragAction::Move, DropActions::MOVE),
        (DragAction::Link, DropActions::LINK),
    ] {
        if a.has(p1) {
            out = out | w1;
        }
    }
    out
}

pub fn action_to_p1(a: DropAction) -> DragAction {
    match a {
        DropAction::Copy | DropAction::Ask => DragAction::Copy,
        DropAction::Move => DragAction::Move,
        DropAction::Link => DragAction::Link,
    }
}

pub fn hover_of_p1(h: Hover) -> DropHover {
    match h {
        Hover::Left => DropHover::Left,
        Hover::Accepting(_) => DropHover::Accepting,
        Hover::Rejecting => DropHover::Rejecting,
    }
}

thread_local! {
    static OUT: RefCell<Option<DragPayload>> = const { RefCell::new(None) };
}

/// The payload of this process's running drag, if any.
pub fn running() -> Option<DragPayload> {
    OUT.with(|o| o.borrow().clone())
}

pub fn offer_of_p1(o: &P1Offer) -> DropOffer {
    let mimes: Vec<std::borrow::Cow<'static, str>> =
        o.mimes.iter().map(|m| m.clone().into()).collect();
    let mut offer = DropOffer::new(mimes, actions_of_p1(o.actions))
        .local(o.local)
        .serial(o.serial);
    let uris = match running().filter(|_| o.local) {
        Some(p) => Some(p.uris),
        None => o.peeked(URI_LIST).map(parse_uri_list),
    };
    if let Some(uris) = uris {
        offer = offer.uris(uris);
    }
    offer
}

fn point_of_p1(p: DropPoint) -> PointerEvent {
    // Non-exhaustive in Rune: built from its default.
    let mut e = PointerEvent::default();
    e.x = p.x;
    e.y = p.y;
    e.window_x = p.window_x;
    e.window_y = p.window_y;
    e
}

pub fn dropped_of_p1(d: &P1Data) -> Dropped {
    let mut offer = offer_of_p1(&d.offer);
    let items: Vec<DropItem> = d
        .items
        .iter()
        .map(|(m, b)| DropItem::new(m.clone(), b.clone()))
        .collect();
    let mut data = DropData::new(items);
    if let Some(p) = d.local::<DragPayload>() {
        offer = offer.uris(p.uris.clone());
        data = data.local(p.clone());
    } else if offer.uris.is_none() {
        let uris = data.uris();
        if !uris.is_empty() {
            offer = offer.uris(uris);
        }
    }
    Dropped::new(offer, data, d.offer.mods.unwrap_or_default()).point(d.point.map(point_of_p1))
}

fn target_of<S: 'static>(zone: DropZone<S>, hovered: Hovered) -> DropTarget<S> {
    let z = zone.clone();
    let mut t = DropTarget::new(move |s: &mut S, d: P1Data, tx: &mut Tx| {
        z.deliver(s, dropped_of_p1(&d), tx);
    })
    .mime_types(zone.preferred_mime_types())
    .read_all();
    if zone.accepts_local() {
        t = t.local::<DragPayload>();
    }
    if zone.hears_hover() {
        if zone.reads(URI_LIST) {
            t = t.peek(&[URI_LIST]);
        }
        let z = zone.clone();
        let seen = hovered.clone();
        t = t.on_hover(move |s: &mut S, h, o: &P1Offer, tx: &mut Tx| {
            note_hover(&seen, h);
            z.hover(s, hover_of_p1(h), &offer_of_p1(o), tx);
        });
    } else {
        t = t.on_hover(move |_: &mut S, h, _: &P1Offer, _: &mut Tx| note_hover(&hovered, h));
    }
    if zone.follows_moves() {
        let z = zone.clone();
        t = t.on_move(move |s: &mut S, p, tx: &mut Tx| z.moved(s, point_of_p1(p), tx));
    }
    if zone.rejects() {
        let z = zone.clone();
        t = t.on_reject(move |s: &mut S, o: &P1Offer, tx: &mut Tx| z.reject(s, offer_of_p1(o), tx));
    }
    t.action(move |s: &S, o: &P1Offer, mods: Option<Mods>| {
        zone.choose(s, &offer_of_p1(o), mods.unwrap_or_default())
            .map(action_to_p1)
    })
}

#[derive(Default)]
struct Seen {
    over: Cell<bool>,
    left: Cell<bool>,
}

type Hovered = Rc<Seen>;

fn note_hover(seen: &Hovered, h: Hover) {
    match h {
        Hover::Left => {
            seen.over.set(false);
            seen.left.set(true);
        }
        Hover::Accepting(_) | Hover::Rejecting => seen.over.set(true),
    }
}

type EndFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;

pub struct Wiring<S: 'static> {
    p1: strata_platform::dnd::Dnd<S>,
    phase: Rc<RefCell<Option<(Drag, String)>>>,
    ended: Rc<RefCell<Option<EndFn<S>>>>,
    hovered: Hovered,
}

impl<S: 'static> Clone for Wiring<S> {
    fn clone(&self) -> Self {
        Self {
            p1: self.p1.clone(),
            phase: self.phase.clone(),
            ended: self.ended.clone(),
            hovered: self.hovered.clone(),
        }
    }
}

impl<S: 'static> Wiring<S> {
    pub fn new(p1: strata_platform::dnd::Dnd<S>) -> Self {
        Self {
            p1,
            phase: Rc::default(),
            ended: Rc::default(),
            hovered: Rc::default(),
        }
    }

    pub fn p1(&self) -> &strata_platform::dnd::Dnd<S> {
        &self.p1
    }

    pub fn binder(&self) -> Binder<S> {
        let (drops, drags) = (self.clone(), self.clone());
        Binder::custom_keyed(
            move |el, key, zone| drops.drop_target(el, key, zone),
            move |el, key, source| drags.drag_source(el, key, source),
        )
    }

    pub fn drop_target(&self, el: El, key: &ElementKey, zone: DropZone<S>) -> El {
        let target = target_of(zone, self.hovered.clone());
        match key.scope() {
            Some(scope) => self.p1.drop_target_in(el, scope, key.key(), target),
            None => self.p1.drop_target(el, key.key(), target),
        }
    }

    pub fn drop_target_by_point(
        &self,
        el: El,
        key: &str,
        zone: DropZone<S>,
        action: impl Fn(&S, &DropOffer, Mods, PointerEvent) -> Option<DropAction> + 'static,
    ) -> El {
        let target = target_of(zone, self.hovered.clone()).action_at(
            move |s: &S, o: &P1Offer, mods: Option<Mods>, p: DropPoint| {
                action(s, &offer_of_p1(o), mods.unwrap_or_default(), point_of_p1(p))
                    .map(action_to_p1)
            },
        );
        self.p1.drop_target(el, key, target)
    }

    pub fn drag_source(&self, el: El, key: &ElementKey, source: DragSource<S>) -> El {
        let (s1, s2) = (source.clone(), source);
        let p1 = strata_platform::dnd::DragSource::new(move |s: &S, _| {
            s1.payload(s).map(|p| {
                OUT.with(|o| *o.borrow_mut() = Some(p.clone()));
                drag_item(&p, None)
            })
        })
        .on_end(move |s: &mut S, outcome, tx: &mut Tx| {
            OUT.with(|o| *o.borrow_mut() = None);
            s2.end(s, outcome, tx);
        });
        self.p1.drag_source(el, key.key(), p1)
    }

    pub fn drag_phase(&self, d: &Drag, key: &str) -> bool {
        if self.p1.owns(d) {
            return true;
        }
        *self.phase.borrow_mut() = Some((*d, key.to_owned()));
        false
    }

    pub fn start(
        &self,
        tx: &mut Tx,
        payload: DragPayload,
        icon: Option<DragIcon>,
        on_end: impl Fn(&mut S, DragOutcome, &mut Tx) + 'static,
    ) -> Result<(), Error> {
        let Some((_drag, _key)) = self.phase.borrow_mut().take() else {
            return Err(Error::Refused("no drag phase to start from"));
        };
        let item = drag_item(&payload, icon);
        OUT.with(|o| *o.borrow_mut() = Some(payload));
        let r = self.p1.start_drag(tx, item, move |s: &mut S, outcome, tx| {
            OUT.with(|o| *o.borrow_mut() = None);
            on_end(s, outcome, tx);
        });
        if r.is_err() {
            OUT.with(|o| *o.borrow_mut() = None);
        }
        r
    }

    pub fn sentinel(
        &self,
        root: El,
        key: &str,
        mimes: &[&str],
        ended: impl Fn(&mut S, &mut Tx) + 'static,
    ) -> El {
        *self.ended.borrow_mut() = Some(Rc::new(ended));
        let seen = self.hovered.clone();
        let t = DropTarget::new(|_: &mut S, _, _: &mut Tx| {})
            .mime_types(mimes)
            .local::<DragPayload>()
            .on_hover(move |_: &mut S, h, _: &P1Offer, _: &mut Tx| note_hover(&seen, h))
            .on_reject(|_: &mut S, _: &P1Offer, _: &mut Tx| {})
            .action(|_: &S, _: &P1Offer, _| None);
        self.p1.drop_target(root, key, t)
    }

    pub fn tick(&self, s: &mut S, tx: &mut Tx) -> bool {
        let (over, left) = (self.hovered.over.get(), self.hovered.left.get());
        if over || !left {
            return false;
        }
        self.hovered.left.set(false);
        let f = self.ended.borrow().clone();
        match f {
            Some(f) => {
                f(s, tx);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests;
