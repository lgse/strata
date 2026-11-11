//! [`Dnd`]: how a component attaches its drop zones and drag sources to elements, as the host
//! chooses.

use std::cell::Cell;
use std::rc::Rc;

use rune::{DragItem, DragOperation, El};

use super::{
    DragPayload, DragSource, DropActions, DropData, DropHover, DropOffer, DropZone, Dropped,
};
use super::{URI_LIST, file_uri, uri_list_only, uri_to_path};

type DropBind<S> = Rc<dyn Fn(El, DropZone<S>) -> El>;
type DragBind<S> = Rc<dyn Fn(El, DragSource<S>) -> El>;
type KeyedDropBind<S> = Rc<dyn Fn(El, &ElementKey, DropZone<S>) -> El>;
type KeyedDragBind<S> = Rc<dyn Fn(El, &ElementKey, DragSource<S>) -> El>;

/// The key a component gave an element it attaches a zone or a source to, and, when that key
/// is unique only inside another keyed element, that element's key (its scope).
///
/// A binder that finds its targets by key needs one it can tell apart from every other in
/// the window: Rune lets no one read an element's key (Rune draft U-P1-1), so the component
/// says it. Most keys are unique in the window on their own, given the host's keys for each
/// component (a crumb, a tab, a sidebar row, a collection's background). A collection's rows
/// are keyed by entry id, unique only inside the collection, so they carry the collection's
/// key as their scope: two collections showing the same entries tell their rows apart by it.
///
/// Fields may be added: build one with [`ElementKey::new`].
///
/// Used by: C1e, C2, C6, C7, C8, X2.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ElementKey {
    key: String,
    scope: Option<String>,
}

impl ElementKey {
    /// The element keyed `key`, unique in its window.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            scope: None,
        }
    }

    /// The same key, unique only inside the element keyed `scope` (an ancestor of it).
    pub fn within(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// The element's own key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The key of the ancestor the element's key is unique inside, if it isn't unique in the
    /// window on its own.
    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }
}

impl From<&str> for ElementKey {
    fn from(key: &str) -> Self {
        Self::new(key)
    }
}

impl From<String> for ElementKey {
    fn from(key: String) -> Self {
        Self::new(key)
    }
}

/// Attaches [`DropZone`]s and [`DragSource`]s to elements. The host passes one to each
/// component that has drop targets or drag sources, and the component calls it for each.
///
/// Components never depend on the system drag-and-drop shim (work README, rule 1). This is
/// how its handlers reach their elements anyway: X2 passes a [`Dnd::custom_keyed`] binder that
/// wires the shim.
///
/// - [`Dnd::rune`], the default, wires Rune's file drops (`on_drop_files` and its hover, move
///   and reject hooks) and `drag_out`. Rune takes file drags on X11 and macOS, and in headless
///   runs, where scripts drop with `drag_files` and `drop_files`; winit reports none on
///   Wayland. Its `drag_out` starts system drags on macOS only, and elsewhere leaves the element
///   as it was.
/// - [`Dnd::none`] attaches nothing, for a read-only collection (C14b's archive preview).
/// - [`Dnd::custom`] and [`Dnd::custom_keyed`] are the host's own: the first sees only the
///   element and the zone or source, the second also each element's [`ElementKey`].
///
/// Components attach with [`Dnd::drop_zone_keyed`] and [`Dnd::drag_source_keyed`] where they
/// key the element, which every binder takes; [`Dnd::drop_zone`] and [`Dnd::drag_source`]
/// pass no key, so a keyed binder attaches nothing through them.
///
/// Used by: C1e, C2, C6, C7, C8, C9, C14b, X2.
pub struct Dnd<S> {
    drop: DropBind<S>,
    drag: DragBind<S>,
    keyed_drop: KeyedDropBind<S>,
    keyed_drag: KeyedDragBind<S>,
}

impl<S> Clone for Dnd<S> {
    fn clone(&self) -> Self {
        Self {
            drop: self.drop.clone(),
            drag: self.drag.clone(),
            keyed_drop: self.keyed_drop.clone(),
            keyed_drag: self.keyed_drag.clone(),
        }
    }
}

impl<S: 'static> Default for Dnd<S> {
    fn default() -> Self {
        Self::rune()
    }
}

impl<S: 'static> Dnd<S> {
    /// Rune's own file drops and drag-out.
    ///
    /// A file drop offers `text/uri-list`, comes from another process, and allows copy and
    /// move: Rune doesn't say what the source allows. The zone gets the files as one uri-list
    /// item, with `tx.mods()` and `tx.drop_point()`.
    ///
    /// - Rune's hover handlers hear no paths, so offers carry [`DropOffer::uris`] only on the
    ///   drop.
    /// - Rune reports no drag starts, so a drag gets a new [`DropOffer::serial`] each time it
    ///   reaches a zone. Its drop shares the serial of the hover before it.
    /// - Rune routes drags with no state, so [`DropZone::action`] is asked only at the drop. A
    ///   refusal there goes to the zone's `on_reject`, or nowhere.
    pub fn rune() -> Self {
        Self::custom(rune_drop_zone, rune_drag_source)
    }

    /// Attaches nothing: elements keep only their own handlers.
    pub fn none() -> Self {
        Self::custom(|el, _| el, |el, _| el)
    }

    /// A binder of the host's own: `drop` attaches a zone to an element, `drag` a source. It
    /// hears keyed elements too, without their keys.
    pub fn custom(
        drop: impl Fn(El, DropZone<S>) -> El + 'static,
        drag: impl Fn(El, DragSource<S>) -> El + 'static,
    ) -> Self {
        let drop: DropBind<S> = Rc::new(drop);
        let drag: DragBind<S> = Rc::new(drag);
        let (d, g) = (drop.clone(), drag.clone());
        Self {
            drop,
            drag,
            keyed_drop: Rc::new(move |el, _, zone| d(el, zone)),
            keyed_drag: Rc::new(move |el, _, source| g(el, source)),
        }
    }

    /// A binder of the host's own that finds its targets by key (X2's, through the system
    /// shim): `drop` attaches a zone to an element keyed as the [`ElementKey`] says, `drag` a
    /// source. The element arrives keyed already. Elements attached without a key
    /// ([`Dnd::drop_zone`], [`Dnd::drag_source`]) keep only their own handlers.
    pub fn custom_keyed(
        drop: impl Fn(El, &ElementKey, DropZone<S>) -> El + 'static,
        drag: impl Fn(El, &ElementKey, DragSource<S>) -> El + 'static,
    ) -> Self {
        Self {
            drop: Rc::new(|el, _| el),
            drag: Rc::new(|el, _| el),
            keyed_drop: Rc::new(drop),
            keyed_drag: Rc::new(drag),
        }
    }

    /// `el`, as a drop target for `zone`, with no key: a [`Dnd::custom_keyed`] binder attaches
    /// nothing. Prefer [`Dnd::drop_zone_keyed`].
    pub fn drop_zone(&self, el: El, zone: DropZone<S>) -> El {
        (self.drop)(el, zone)
    }

    /// `el`, as a drag source for `source`, with no key: a [`Dnd::custom_keyed`] binder
    /// attaches nothing. Prefer [`Dnd::drag_source_keyed`].
    pub fn drag_source(&self, el: El, source: DragSource<S>) -> El {
        (self.drag)(el, source)
    }

    /// `el`, keyed `key`, as a drop target for `zone`. Pass the key the element has (it is set
    /// again, so the two can't differ), unique in the window or inside its scope.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn drop_zone_keyed(&self, el: El, key: impl Into<ElementKey>, zone: DropZone<S>) -> El {
        let key = key.into();
        let el = el.key(key.key());
        (self.keyed_drop)(el, &key, zone)
    }

    /// `el`, keyed `key`, as a drag source for `source`. Pass the key the element has (it is
    /// set again, so the two can't differ), unique in the window or inside its scope.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn drag_source_keyed(
        &self,
        el: El,
        key: impl Into<ElementKey>,
        source: DragSource<S>,
    ) -> El {
        let key = key.into();
        let el = el.key(key.key());
        (self.keyed_drag)(el, &key, source)
    }
}

thread_local! {
    /// The serial of the Rune file drag last heard, and whether it is still live (no drop yet).
    /// Shared by every zone the Rune binder wires on this thread, which is the UI thread.
    static RUNE_DRAG: Cell<(u64, bool)> = const { Cell::new((0, false)) };
}

/// Rune reports no drag starts, so a drag's serial is taken from what zones hear. A zone
/// reached (`on_drop_hover(true)`, or its rejected twin) starts a new serial: one drag moving
/// across zones gets several, but two drags never share one. A leave keeps it, so the drop that
/// follows a hover shares the hover's serial.
fn hover_serial(over: bool) -> u64 {
    RUNE_DRAG.with(|d| {
        let (serial, _) = d.get();
        if over {
            d.set((serial + 1, true));
            serial + 1
        } else {
            serial
        }
    })
}

/// The serial of a drop: the live drag's, else a new one (a drop no hover came before, which
/// only headless runs send). The drag ends with it.
fn drop_serial() -> u64 {
    RUNE_DRAG.with(|d| {
        let (serial, live) = d.get();
        let serial = if live { serial } else { serial + 1 };
        d.set((serial, false));
        serial
    })
}

/// What a Rune file drag offers: files, as copy or move, since Rune doesn't say what the
/// source allows.
fn file_offer(serial: u64) -> DropOffer {
    DropOffer::new(uri_list_only(), DropActions::COPY | DropActions::MOVE).serial(serial)
}

fn rune_drop_zone<S: 'static>(el: El, zone: DropZone<S>) -> El {
    // Rune's drags all offer `text/uri-list` and come from another process, so reading it is
    // the whole acceptance test, with no offer to build per element.
    if zone.reads(URI_LIST) {
        let z = zone.clone();
        let mut el = el.on_drop_files(move |s: &mut S, files, tx| {
            let uris = files.iter().map(|p| file_uri(p)).collect();
            let offer = file_offer(drop_serial()).uris(uris);
            if z.choose(s, &offer, tx.mods()).is_none() {
                // Refused by its `action`: Rune has already routed the drop here, so it can't
                // pass it on. Only `on_reject` hears it.
                z.reject(s, offer, tx);
                return;
            }
            let dropped =
                Dropped::new(offer, DropData::files(files), tx.mods()).point(tx.drop_point());
            z.deliver(s, dropped, tx);
        });
        // Wired even for a zone that doesn't listen, so every drag reaching a zone gets a serial.
        let z = zone.clone();
        el = el.on_drop_hover(move |s: &mut S, over, tx| {
            let offer = file_offer(hover_serial(over));
            z.hover(
                s,
                if over {
                    DropHover::Accepting
                } else {
                    DropHover::Left
                },
                &offer,
                tx,
            );
        });
        if zone.follows_moves() {
            el = el.on_drop_move(move |s: &mut S, point, tx| zone.moved(s, point, tx));
        }
        el
    } else if zone.rejects() {
        // A target that takes none of the files, and stops the drag rather than passing it on.
        let z = zone.clone();
        el.on_drop_files(|_: &mut S, _, _| {})
            .accepts_files(|_| false)
            .on_drop_rejected(move |s: &mut S, files, tx| {
                let uris = files.iter().map(|p| file_uri(p)).collect();
                z.reject(s, file_offer(drop_serial()).uris(uris), tx);
            })
            .on_drop_rejected_hover(move |s: &mut S, over, tx| {
                let offer = file_offer(hover_serial(over));
                let hover = if over {
                    DropHover::Rejecting
                } else {
                    DropHover::Left
                };
                zone.hover(s, hover, &offer, tx);
            })
    } else {
        el
    }
}

fn rune_drag_source<S: 'static>(el: El, source: DragSource<S>) -> El {
    let src = source.clone();
    let el = el.drag_out(move |s: &S| src.payload(s).and_then(|p| drag_item(&p)));
    if source.hears_end() {
        el.on_drag_end(move |s: &mut S, outcome, tx| source.end(s, outcome, tx))
    } else {
        el
    }
}

/// Rune's drag item for `payload`: its local files, any other URIs as text, and its actions.
/// `None` when it allows nothing or carries nothing Rune can offer.
pub(super) fn drag_item(payload: &DragPayload) -> Option<DragItem> {
    let ops = [
        (DropActions::COPY, DragOperation::Copy),
        (DropActions::MOVE, DragOperation::Move),
        (DropActions::LINK, DragOperation::Link),
    ];
    let mut allowed = ops
        .iter()
        .filter(|(a, _)| payload.actions.contains(*a))
        .map(|&(_, op)| op);
    let mut item = DragItem::new().operation(allowed.next()?);
    for op in allowed {
        item = item.allow(op);
    }
    let mut others = Vec::new();
    for uri in &payload.uris {
        match uri_to_path(uri) {
            Some(path) => item = item.file(path),
            None => others.push(uri.as_str()),
        }
    }
    if !others.is_empty() {
        item = item.text(others.join("\n"));
    }
    (!item.is_empty()).then_some(item)
}
