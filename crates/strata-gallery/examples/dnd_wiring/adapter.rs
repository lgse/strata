//! The adapter between W1's drag-and-drop intents and P1's system drag and drop: written to be
//! copied into an app as it is (Strata's), with nothing of this demo in it.
//!
//! - [`Wiring::binder`] is a keyed W1 binder (`Dnd::custom_keyed`) that turns each component's
//!   [`DropZone`] into a P1 [`DropTarget`] registered by the element's key (and its scope, for
//!   rows keyed inside their list: P1's `drop_target_in`), so P1 finds it from a hit test's
//!   path. Hover, move, reject and the zone's action map one to one; the zone's MIME types are
//!   read at the drop, all of them, in its order; `text/uri-list` is read while the drag
//!   hovers, for hosts that check a drag's items before the drop (C8 pins only folders).
//! - A collection's drags start from C1b's `Intent::BeginDrag`, not from a row (C1e owns the
//!   container's `on_drag`): [`Wiring::drag_phase`] goes in `Collection::on_drag_phase` and
//!   routes every phase through P1's `owns`, keeping the phase the intent comes in, and
//!   [`Wiring::start`] starts P1's drag from it with [`drag_item`]'s formats.
//! - Every drag from this process carries its [`DragPayload`] as P1's typed payload, so a drop
//!   in any of the process's windows takes the entries with nothing read (the fast path), and a
//!   zone hears the drag's URIs while it hovers.
//! - [`Wiring::sentinel`] marks a window's root, so a host can tell that a drag from another
//!   app ended (C22's `hold_rest`): see its docs.
//!
//! W1's `DropAction::Ask` is offered to the source as a copy (W1's rule): the drag shows a
//! copy, and the host asks after the drop. Link is X11 only (P1).

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

/// GNOME's file-list type: `copy` (or `cut`), then one URI per line, with no trailing newline.
/// Nautilus, Nemo, Caja and Thunar read files from it.
pub const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
/// KDE's cut flag beside `text/uri-list`: `0` for a copy. Dolphin reads it.
pub const KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";

/// `text/uri-list`: one URI a line, each ending in CRLF (RFC 2483, as GTK writes it).
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

/// What a drag of `payload` offers: its URIs in the three file formats, each made on P1's
/// worker the first time a target asks (a selection of 100,000 entries serialises off the UI
/// thread), the payload itself for drops in this process, and its actions.
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

/// W1's actions as P1's.
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

/// P1's actions as W1's: ask isn't one W1 offers.
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

/// W1's action as the one P1 negotiates: ask goes to the source as a copy (W1's rule).
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
    /// The drag this process started and hasn't ended: its payload, so a zone in any of the
    /// process's windows hears its URIs while it hovers. Windows share the UI thread.
    static OUT: RefCell<Option<DragPayload>> = const { RefCell::new(None) };
}

/// The payload of this process's running drag, if any.
pub fn running() -> Option<DragPayload> {
    OUT.with(|o| o.borrow().clone())
}

/// W1's view of P1's offer. Its URIs are the payload's for a drag from this process, else the
/// `text/uri-list` read while it hovers, once it has arrived.
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

/// W1's drop of P1's: the items read, in the zone's order, and the payload of a drag from this
/// process.
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

/// P1's target for W1's `zone`, telling `hovered` about each hover change.
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

/// What the window knows of the drag over it: whether a target has it, and whether one left
/// since the last [`Wiring::tick`] with none reached after (a drag that ended, dropped or left
/// the window: see [`Wiring::sentinel`]).
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

/// One window's wiring: its P1 `Dnd`, and the phase a collection's drag is starting from.
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

    /// The window's P1 `Dnd` (to attach it, and to cancel a drag).
    pub fn p1(&self) -> &strata_platform::dnd::Dnd<S> {
        &self.p1
    }

    /// The W1 binder to pass every component: zones become P1 targets by key, and sources P1
    /// drag sources on their element (C9's preview handle, the viewers).
    pub fn binder(&self) -> Binder<S> {
        let (drops, drags) = (self.clone(), self.clone());
        Binder::custom_keyed(
            move |el, key, zone| drops.drop_target(el, key, zone),
            move |el, key, source| drags.drag_source(el, key, source),
        )
    }

    /// `el`, keyed as `key` says, as a P1 target for `zone` (what the binder does for each
    /// component's zone).
    pub fn drop_target(&self, el: El, key: &ElementKey, zone: DropZone<S>) -> El {
        let target = target_of(zone, self.hovered.clone());
        match key.scope() {
            Some(scope) => self.p1.drop_target_in(el, scope, key.key(), target),
            None => self.p1.drop_target(el, key.key(), target),
        }
    }

    /// [`Wiring::drop_target`] for a zone whose action depends on where the drag is over it
    /// (one that routes a drop by point): `action` is asked with the drag's point, in the
    /// element's points and the window's (P1's `DropTarget::action_at`), instead of the zone's.
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

    /// `el` as a P1 drag source for W1's `source`, which owns its `on_drag`.
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

    /// For `Collection::on_drag_phase`: every phase of the container's drag goes through P1's
    /// `owns` (true: the system drag's, the collection ignores it), and the phase a drag may
    /// start from is kept for [`Wiring::start`].
    pub fn drag_phase(&self, d: &Drag, key: &str) -> bool {
        if self.p1.owns(d) {
            return true;
        }
        *self.phase.borrow_mut() = Some((*d, key.to_owned()));
        false
    }

    /// Starts P1's drag of `payload` from the phase C1b's `Intent::BeginDrag` came in: call it
    /// from the intent's handler. `on_end` hears how it ended. Keep the collection the drag
    /// started from mounted at its place until then (P1's rule: its container is the source).
    pub fn start(
        &self,
        tx: &mut Tx,
        payload: DragPayload,
        icon: Option<DragIcon>,
        on_end: impl Fn(&mut S, DragOutcome, &mut Tx) + 'static,
    ) -> Result<(), Error> {
        let Some((drag, key)) = self.phase.borrow_mut().take() else {
            return Err(Error::Refused("no drag phase to start from"));
        };
        let item = drag_item(&payload, icon);
        OUT.with(|o| *o.borrow_mut() = Some(payload));
        let r = self
            .p1
            .start_drag(tx, &drag, &key, item, move |s: &mut S, outcome, tx| {
                OUT.with(|o| *o.borrow_mut() = None);
                on_end(s, outcome, tx);
            });
        if r.is_err() {
            OUT.with(|o| *o.borrow_mut() = None);
        }
        r
    }

    /// `root` (the window's root, keyed `key`) as the sentinel: a target that takes any drag
    /// of `mimes` (and any from this process) and refuses it, so over the window outside every
    /// component's zone a drag is still over a target (rejecting, as nowhere is to its source).
    /// A drag then leaves every target only when it drops, ends, or leaves the window, which
    /// [`Wiring::tick`] reports to `ended` once no target has it at the next poll: what C22's
    /// `hold_rest` and `PeekState::close` need for drags that didn't start here.
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

    /// From the window's poll: reports a drag that left every target and reached none since
    /// to the sentinel's `ended`. True when it did.
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
