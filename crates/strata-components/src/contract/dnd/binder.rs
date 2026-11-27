// SPDX-License-Identifier: MIT

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

/// The key a component gave an element it attaches a zone or a source to, and.
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

    /// The key of the ancestor the element's key is unique inside, if it isn't unique in the window on its own.
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

/// Attaches `DropZone`s and `DragSource`s to elements.
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
    pub fn rune() -> Self {
        Self::custom(rune_drop_zone, rune_drag_source)
    }

    /// Attaches nothing: elements keep only their own handlers.
    pub fn none() -> Self {
        Self::custom(|el, _| el, |el, _| el)
    }

    /// A binder of the host's own: `drop` attaches a zone to an element, `drag` a source.
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

    /// A binder of the host's own that finds its targets by key.
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

    /// `el`, as a drop target for `zone`, with no key: a `Dnd::custom_keyed` binder attaches nothing.
    pub fn drop_zone(&self, el: El, zone: DropZone<S>) -> El {
        (self.drop)(el, zone)
    }

    /// `el`, as a drag source for `source`, with no key: a `Dnd::custom_keyed` binder attaches nothing.
    pub fn drag_source(&self, el: El, source: DragSource<S>) -> El {
        (self.drag)(el, source)
    }

    /// `el`, keyed `key`, as a drop target for `zone`.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn drop_zone_keyed(&self, el: El, key: impl Into<ElementKey>, zone: DropZone<S>) -> El {
        let key = key.into();
        let el = el.key(key.key());
        (self.keyed_drop)(el, &key, zone)
    }

    /// `el`, keyed `key`, as a drag source for `source`.
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
    static RUNE_DRAG: Cell<(u64, bool)> = const { Cell::new((0, false)) };
}

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

fn drop_serial() -> u64 {
    RUNE_DRAG.with(|d| {
        let (serial, live) = d.get();
        let serial = if live { serial } else { serial + 1 };
        d.set((serial, false));
        serial
    })
}

fn file_offer(serial: u64) -> DropOffer {
    DropOffer::new(uri_list_only(), DropActions::COPY | DropActions::MOVE).serial(serial)
}

fn rune_drop_zone<S: 'static>(el: El, zone: DropZone<S>) -> El {
    if zone.reads(URI_LIST) {
        let z = zone.clone();
        let mut el = el.on_drop_files(move |s: &mut S, files, tx| {
            let uris = files.iter().map(|p| file_uri(p)).collect();
            let offer = file_offer(drop_serial()).uris(uris);
            if z.choose(s, &offer, tx.mods()).is_none() {
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
