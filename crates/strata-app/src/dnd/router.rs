// SPDX-License-Identifier: MIT

//! Dual-mode drag-and-drop router: delegates to `strata_platform::dnd::Dnd` when attached to a
//! native window (`host::windowed`), and drives an in-window keyed `DropZone` hit-test session
//! on the headless offscreen host (`host::offscreen`) as well as for in-window sidebar row drags.

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use rune::{Drag, DragOutcome, El, Live, Mods, Phase, Pick, Picked, PointerEvent, Tx, el};
use strata_components::contract::{
    Dnd as Binder,
    dnd::{
        DragPayload, DragSource, DropAction, DropActions, DropData, DropHover, DropItem, DropOffer,
        DropZone, Dropped, ElementKey, URI_LIST,
    },
};
use strata_platform::dnd::{
    DragIcon, DropOffer as P1Offer, DropPoint, Error as P1Error,
};

use super::adapter::{
    action_to_p1, drag_item, encode_uri_list, offer_of_p1, point_of_p1, set_running, target_of,
};
use crate::{App, shell, slots::TabKey};

const REGISTRY_CAP: usize = 16_384;
const SOURCE_SLOP: f32 = 8.0;

type PointActionFn = Rc<dyn Fn(&App, &DropOffer, Mods, PointerEvent) -> Option<DropAction>>;
type EndFn = Rc<dyn Fn(&mut App, DragOutcome, &mut Tx)>;

struct TargetEntry {
    zone: DropZone<App>,
    by_point: Option<PointActionFn>,
    rect: Live<[f32; 4]>,
    stamp: u64,
}

impl TargetEntry {
    fn point_of(&self, at: [f32; 2]) -> PointerEvent {
        let [rx, ry, ..] = self.rect.get();
        let mut event = PointerEvent::default();
        event.x = at[0] - rx;
        event.y = at[1] - ry;
        event.window_x = at[0];
        event.window_y = at[1];
        event
    }
}

#[derive(Default)]
struct Registry {
    entries: HashMap<String, TargetEntry>,
    next: u64,
}

fn scoped_id(scope: &str, key: &str) -> String {
    format!("{scope}\u{1f}{key}")
}

fn keyed_segments(path: &str) -> impl DoubleEndedIterator<Item = &str> {
    path.split('/')
        .filter(|seg| !seg.is_empty() && !seg.starts_with('#'))
}

impl Registry {
    fn put(
        &mut self,
        key: &str,
        zone: DropZone<App>,
        by_point: Option<PointActionFn>,
    ) -> Live<[f32; 4]> {
        self.next += 1;
        let stamp = self.next;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.zone = zone;
            entry.by_point = by_point;
            entry.stamp = stamp;
            return entry.rect.clone();
        }
        let rect = Live::new([0.0; 4]);
        self.entries.insert(
            key.to_owned(),
            TargetEntry {
                zone,
                by_point,
                rect: rect.clone(),
                stamp,
            },
        );
        if self.entries.len() > REGISTRY_CAP {
            self.evict();
        }
        rect
    }

    fn evict(&mut self) {
        let mut stamps: Vec<u64> = self.entries.values().map(|e| e.stamp).collect();
        stamps.sort_unstable();
        let cut = stamps[stamps.len() / 8];
        self.entries.retain(|_, e| e.stamp > cut);
    }

    fn get(&self, key: &str) -> Option<&TargetEntry> {
        self.entries.get(key)
    }

    fn along(&self, path: &str) -> Vec<String> {
        let segs: Vec<&str> = keyed_segments(path).collect();
        let mut out = Vec::new();
        for (idx, seg) in segs.iter().enumerate().rev() {
            if self.entries.contains_key(*seg) {
                out.push((*seg).to_owned());
                continue;
            }
            if let Some(id) = segs[..idx]
                .iter()
                .rev()
                .map(|scope| scoped_id(scope, seg))
                .find(|id| self.entries.contains_key(id))
            {
                out.push(id);
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Landing {
    Nowhere,
    Accepting { key: String, action: DropAction },
    Rejecting { key: String },
}

impl Landing {
    fn key(&self) -> Option<&str> {
        match self {
            Self::Nowhere => None,
            Self::Accepting { key, .. } | Self::Rejecting { key } => Some(key),
        }
    }

    fn hover(&self) -> DropHover {
        match self {
            Self::Nowhere => DropHover::Left,
            Self::Accepting { .. } => DropHover::Accepting,
            Self::Rejecting { .. } => DropHover::Rejecting,
        }
    }
}

struct InWindowDrag {
    id: u64,
    source_key: String,
    offer: DropOffer,
    payload: DragPayload,
    at: [f32; 2],
    landing: Landing,
    seq: u64,
    origin_tab: TabKey,
    tab_tail: Option<Vec<String>>,
    on_end: Option<EndFn>,
}

#[derive(Clone)]
pub(crate) struct Wiring(Rc<Core>);

struct Core {
    p1: RefCell<strata_platform::dnd::Dnd<App>>,
    windowed: Cell<bool>,
    registry: RefCell<Registry>,
    phase: RefCell<Option<(Drag, String)>>,
    in_window: RefCell<Option<InWindowDrag>>,
    serial: Cell<u64>,
    swallow_until_up: Cell<bool>,
}

impl Default for Wiring {
    fn default() -> Self {
        Self::new()
    }
}

impl Wiring {
    pub(crate) fn new() -> Self {
        Self(Rc::new(Core {
            p1: RefCell::new(strata_platform::dnd::Dnd::new()),
            windowed: Cell::new(false),
            registry: RefCell::new(Registry::default()),
            phase: RefCell::new(None),
            in_window: RefCell::new(None),
            serial: Cell::new(0),
            swallow_until_up: Cell::new(false),
        }))
    }

    pub(crate) fn bind_windowed(&self, p1: strata_platform::dnd::Dnd<App>) {
        *self.0.p1.borrow_mut() = p1;
        self.0.windowed.set(true);
    }

    pub(crate) fn is_dragging(&self) -> bool {
        self.0.in_window.borrow().is_some()
    }

    pub(crate) fn binder(&self) -> Binder<App> {
        let (drops, drags) = (self.clone(), self.clone());
        Binder::custom_keyed(
            move |el, key, zone| drops.drop_target(el, key, zone),
            move |el, key, source| drags.drag_source(el, key, source),
        )
    }

    pub(crate) fn drop_target(&self, el: El, key: &ElementKey, zone: DropZone<App>) -> El {
        let id = match key.scope() {
            Some(scope) => scoped_id(scope, key.key()),
            None => key.key().to_owned(),
        };
        let rect = self.0.registry.borrow_mut().put(&id, zone.clone(), None);
        if self.0.windowed.get() {
            let target = target_of(zone);
            let p1 = self.0.p1.borrow();
            return match key.scope() {
                Some(scope) => p1.drop_target_in(el, scope, key.key(), target),
                None => p1.drop_target(el, key.key(), target),
            };
        }
        let el = el.key(key.key()).drawn_rect_into(&rect);
        Binder::rune().drop_zone(el, zone)
    }

    pub(crate) fn drop_target_by_point(
        &self,
        el: El,
        key: &str,
        zone: DropZone<App>,
        action: impl Fn(&App, &DropOffer, Mods, PointerEvent) -> Option<DropAction> + 'static,
    ) -> El {
        let by_point: PointActionFn = Rc::new(action);
        let rect = self
            .0
            .registry
            .borrow_mut()
            .put(key, zone.clone(), Some(by_point.clone()));
        if self.0.windowed.get() {
            let target = target_of(zone).action_at(
                move |app: &App, offer: &P1Offer, mods: Option<Mods>, point: DropPoint| {
                    by_point(
                        app,
                        &offer_of_p1(offer),
                        mods.unwrap_or_default(),
                        point_of_p1(point),
                    )
                    .map(action_to_p1)
                },
            );
            return self.0.p1.borrow().drop_target(el, key, target);
        }
        el.key(key).drawn_rect_into(&rect)
    }

    pub(crate) fn drag_source(&self, el: El, key: &ElementKey, source: DragSource<App>) -> El {
        if self.0.windowed.get() {
            let (s1, s2) = (source.clone(), source);
            let p1_source = strata_platform::dnd::DragSource::new(move |app: &App, _| {
                s1.payload(app).map(|payload| {
                    set_running(Some(payload.clone()));
                    drag_item(&payload, None)
                })
            })
            .on_start(super::on_preview_drag_start)
            .on_end(move |app: &mut App, outcome, tx: &mut Tx| {
                set_running(None);
                super::on_drag_ended(app, tx);
                s2.end(app, outcome, tx);
            });
            return self.0.p1.borrow().drag_source(el, key.key(), p1_source);
        }
        self.offscreen_drag_source(el, key.key(), source)
    }

    fn offscreen_drag_source(&self, el: El, key: &str, source: DragSource<App>) -> El {
        let me = self.clone();
        let source_key = key.to_owned();
        let pressed: Rc<Cell<Option<[f32; 2]>>> = Rc::new(Cell::new(None));
        el.key(key).on_drag(move |app: &mut App, d: &mut Drag, tx: &mut Tx| {
            match d.phase {
                Phase::Down => {
                    me.0.swallow_until_up.set(false);
                    pressed.set(Some(d.window_point()));
                }
                Phase::Move if me.0.swallow_until_up.get() => {}
                Phase::Move if me.is_dragging() => {
                    me.move_to(d.window_point(), tx);
                }
                Phase::Move => {
                    let Some([sx, sy]) = pressed.get() else {
                        return;
                    };
                    let [wx, wy] = d.window_point();
                    if (wx - sx).abs().max((wy - sy).abs()) <= SOURCE_SLOP {
                        return;
                    }
                    let Some(payload) = source.payload(app) else {
                        return;
                    };
                    super::on_preview_drag_start(app, tx);
                    let src = source.clone();
                    me.start_in_window(
                        app,
                        (d, &source_key),
                        payload,
                        false,
                        move |app, outcome, tx| {
                            super::on_drag_ended(app, tx);
                            src.end(app, outcome, tx);
                        },
                        tx,
                    );
                }
                Phase::Up => {
                    pressed.set(None);
                    if me.0.swallow_until_up.replace(false) {
                        return;
                    }
                    if me.is_dragging() {
                        me.finish_gesture(app, d, tx);
                    }
                }
            }
        })
    }

    pub(crate) fn drag_phase(&self, app: &mut App, d: &Drag, key: &str, tx: &mut Tx) -> bool {
        if self.0.windowed.get() && self.0.p1.borrow().owns(d) {
            return true;
        }
        match d.phase {
            Phase::Down => {
                self.0.swallow_until_up.set(false);
                *self.0.phase.borrow_mut() = Some((*d, key.to_owned()));
                false
            }
            Phase::Move if self.0.swallow_until_up.get() => true,
            Phase::Move if self.is_dragging() => {
                self.move_to(d.window_point(), tx);
                true
            }
            Phase::Move => {
                *self.0.phase.borrow_mut() = Some((*d, key.to_owned()));
                false
            }
            Phase::Up => {
                *self.0.phase.borrow_mut() = None;
                if self.0.swallow_until_up.replace(false) {
                    return true;
                }
                if self.is_dragging() {
                    self.finish_gesture(app, d, tx);
                    return true;
                }
                false
            }
        }
    }

    pub(crate) fn start(
        &self,
        app: &mut App,
        tx: &mut Tx,
        payload: DragPayload,
        icon: Option<DragIcon>,
        on_end: impl Fn(&mut App, DragOutcome, &mut Tx) + 'static,
    ) -> Result<(), String> {
        let Some((drag, key)) = self.0.phase.borrow_mut().take() else {
            return Err("no drag phase to start from".to_owned());
        };
        let on_end: EndFn = Rc::new(on_end);
        if self.0.windowed.get() {
            let item = drag_item(&payload, icon);
            let p1_end = on_end.clone();
            set_running(Some(payload.clone()));
            let res = self.0.p1.borrow().start_drag(
                tx,
                &drag,
                &key,
                item,
                move |app: &mut App, outcome, tx| {
                    set_running(None);
                    p1_end(app, outcome, tx);
                },
            );
            match res {
                Ok(()) => return Ok(()),
                Err(P1Error::NotAttached) => set_running(None),
                Err(err) => {
                    set_running(None);
                    return Err(err.to_string());
                }
            }
        }
        self.start_in_window(
            app,
            (&drag, &key),
            payload,
            false,
            move |app, outcome, tx| on_end(app, outcome, tx),
            tx,
        );
        Ok(())
    }

    pub(crate) fn start_sidebar_drag(
        &self,
        app: &App,
        payload: DragPayload,
        on_end: impl Fn(&mut App, DragOutcome, &mut Tx) + 'static,
    ) {
        let id = self.next_serial();
        let offer = DropOffer::new(Vec::<Cow<'static, str>>::new(), payload.actions)
            .local(true)
            .serial(id);
        set_running(Some(payload.clone()));
        *self.0.in_window.borrow_mut() = Some(InWindowDrag {
            id,
            source_key: String::new(),
            offer,
            payload,
            at: [0.0, 0.0],
            landing: Landing::Nowhere,
            seq: 0,
            origin_tab: shell::tabs::shown_tab(app),
            tab_tail: None,
            on_end: Some(Rc::new(on_end)),
        });
    }

    fn start_in_window(
        &self,
        app: &App,
        (drag, source_key): (&Drag, &str),
        payload: DragPayload,
        sidebar_only: bool,
        on_end: impl Fn(&mut App, DragOutcome, &mut Tx) + 'static,
        tx: &mut Tx,
    ) {
        let id = self.next_serial();
        let mimes: Vec<Cow<'static, str>> = if sidebar_only {
            Vec::new()
        } else {
            vec![Cow::Borrowed(URI_LIST)]
        };
        let mut offer = DropOffer::new(mimes, payload.actions)
            .local(true)
            .serial(id);
        if !sidebar_only {
            offer = offer.uris(payload.uris.clone());
        }
        let [wx, wy] = drag.window_point();
        let start_at = (wx - drag.dx, wy - drag.dy);
        let origin_tab = shell::tabs::shown_tab(app);
        set_running(Some(payload.clone()));
        *self.0.in_window.borrow_mut() = Some(InWindowDrag {
            id,
            source_key: source_key.to_owned(),
            offer,
            payload,
            at: [wx, wy],
            landing: Landing::Nowhere,
            seq: 0,
            origin_tab,
            tab_tail: None,
            on_end: Some(Rc::new(on_end)),
        });
        self.capture_source_tail(id, origin_tab, source_key, start_at, tx);
        self.move_to([wx, wy], tx);
    }

    fn capture_source_tail(
        &self,
        id: u64,
        origin_tab: TabKey,
        source_key: &str,
        at: (f32, f32),
        tx: &mut Tx,
    ) {
        let me = self.clone();
        let wanted = source_key.to_owned();
        let tab_seg = format!("tab-{}", origin_tab.0);
        tx.element_at_in_with(
            source_key.to_owned(),
            at.0,
            at.1,
            Pick::new().transparent(true),
            move |_: &mut App, picked: Option<Picked>, _: &mut Tx| {
                let Some(picked) = picked else {
                    return;
                };
                let segs: Vec<&str> = picked.path().split('/').collect();
                let Some(start) = segs.iter().position(|s| *s == tab_seg) else {
                    return;
                };
                let Some(end) = segs[start..].iter().position(|s| *s == wanted) else {
                    return;
                };
                let tail = segs[start..=start + end]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect();
                if let Some(active) = me.0.in_window.borrow_mut().as_mut().filter(|d| d.id == id) {
                    active.tab_tail = Some(tail);
                }
            },
        );
    }

    fn next_serial(&self) -> u64 {
        let next = self.0.serial.get() + 1;
        self.0.serial.set(next);
        next
    }

    pub(crate) fn move_to(&self, at: [f32; 2], tx: &mut Tx) {
        let seq = {
            let mut slot = self.0.in_window.borrow_mut();
            let Some(drag) = slot.as_mut() else {
                return;
            };
            drag.at = at;
            drag.seq += 1;
            drag.seq
        };
        let me = self.clone();
        tx.element_at_with(
            at[0],
            at[1],
            Pick::new().transparent(true),
            move |app: &mut App, picked: Option<Picked>, tx: &mut Tx| {
                me.landed(seq, picked.as_ref().map(Picked::path), app, tx);
            },
        );
    }

    pub(crate) fn repick(&self, tx: &mut Tx) {
        let at = self.0.in_window.borrow().as_ref().map(|d| d.at);
        if let Some(at) = at {
            self.move_to(at, tx);
        }
    }

    pub(crate) fn leave_hover(&self, app: &mut App, tx: &mut Tx) {
        let left = {
            let mut slot = self.0.in_window.borrow_mut();
            let Some(drag) = slot.as_mut() else {
                return;
            };
            drag.seq += 1;
            let old = std::mem::replace(&mut drag.landing, Landing::Nowhere);
            old.key().map(|k| (k.to_owned(), drag.offer.clone()))
        };
        if let Some((key, offer)) = left
            && let Some(zone) = self.zone(&key)
        {
            zone.hover(app, DropHover::Left, &offer, tx);
        }
    }

    pub(crate) fn cancel(&self, app: &mut App, tx: &mut Tx, swallow_release: bool) {
        if self.0.windowed.get() {
            self.0.p1.borrow().cancel_drag();
        }
        if swallow_release {
            self.0.swallow_until_up.set(true);
        }
        let Some(drag) = self.0.in_window.borrow_mut().take() else {
            return;
        };
        set_running(None);
        if let Some(key) = drag.landing.key()
            && let Some(zone) = self.zone(key)
        {
            zone.hover(app, DropHover::Left, &drag.offer, tx);
        }
        if let Some(end) = drag.on_end {
            end(app, DragOutcome::Cancelled, tx);
        }
    }

    pub(crate) fn drop_at(&self, at: [f32; 2], tx: &mut Tx) {
        let id = {
            let mut slot = self.0.in_window.borrow_mut();
            let Some(drag) = slot.as_mut() else {
                return;
            };
            drag.at = at;
            drag.seq += 1;
            drag.id
        };
        let me = self.clone();
        tx.element_at_with(
            at[0],
            at[1],
            Pick::new().transparent(true),
            move |app: &mut App, picked: Option<Picked>, tx: &mut Tx| {
                me.complete_drop(id, picked.as_ref().map(Picked::path), app, tx);
            },
        );
    }

    fn finish_gesture(&self, app: &mut App, d: &Drag, tx: &mut Tx) {
        if d.cancelled() {
            self.cancel(app, tx, false);
        } else {
            self.drop_at(d.window_point(), tx);
        }
    }

    fn zone(&self, key: &str) -> Option<DropZone<App>> {
        self.0.registry.borrow().get(key).map(|e| e.zone.clone())
    }

    fn point(&self, key: &str, at: [f32; 2]) -> PointerEvent {
        self.0
            .registry
            .borrow()
            .get(key)
            .map_or_else(PointerEvent::default, |e| e.point_of(at))
    }

    fn resolve_landing(&self, path: Option<&str>, app: &App, offer: &DropOffer, at: [f32; 2]) -> Landing {
        let Some(path) = path else {
            return Landing::Nowhere;
        };
        let mods = shell::held_modifiers(app);
        let registry = self.0.registry.borrow();
        for key in registry.along(path) {
            let Some(entry) = registry.get(&key) else {
                continue;
            };
            let stops = entry.zone.rejects();
            if !entry.zone.accepts(offer) {
                if stops {
                    return Landing::Rejecting { key };
                }
                continue;
            }
            let chosen = match &entry.by_point {
                Some(by_point) => by_point(app, offer, mods, entry.point_of(at)),
                None => entry.zone.choose(app, offer, mods),
            };
            match negotiate_local(chosen, offer.actions) {
                Some(action) => return Landing::Accepting { key, action },
                None if stops => return Landing::Rejecting { key },
                None => continue,
            }
        }
        Landing::Nowhere
    }

    fn landed(&self, seq: u64, path: Option<&str>, app: &mut App, tx: &mut Tx) {
        let (landing, old, offer, at) = {
            let slot = self.0.in_window.borrow();
            let Some(drag) = slot.as_ref().filter(|d| d.seq == seq) else {
                return;
            };
            let landing = self.resolve_landing(path, app, &drag.offer, drag.at);
            (landing, drag.landing.clone(), drag.offer.clone(), drag.at)
        };
        if old != landing {
            if let Some(drag) = self.0.in_window.borrow_mut().as_mut().filter(|d| d.seq == seq) {
                drag.landing = landing.clone();
            }
            if old.key() != landing.key()
                && let Some(old_zone) = old.key().and_then(|k| self.zone(k))
            {
                old_zone.hover(app, DropHover::Left, &offer, tx);
            }
            if let Some(new_zone) = landing.key().and_then(|k| self.zone(k)) {
                new_zone.hover(app, landing.hover(), &offer, tx);
            }
        }
        // Rejecting zones also hear moves so listing/strip edge-scroll continues over same-dir panes.
        if let Landing::Accepting { key, .. } | Landing::Rejecting { key } = &landing
            && let Some(zone) = self.zone(key)
        {
            zone.moved(app, self.point(key, at), tx);
        }
    }

    fn complete_drop(&self, id: u64, path: Option<&str>, app: &mut App, tx: &mut Tx) {
        let seq = match self.0.in_window.borrow().as_ref().filter(|d| d.id == id) {
            Some(d) => d.seq,
            None => return,
        };
        self.landed(seq, path, app, tx);
        let Some(drag) = self.0.in_window.borrow_mut().take_if(|d| d.id == id) else {
            return;
        };
        set_running(None);
        if let Some(key) = drag.landing.key()
            && let Some(zone) = self.zone(key)
        {
            zone.hover(app, DropHover::Left, &drag.offer, tx);
        }
        let outcome = match &drag.landing {
            Landing::Accepting { key, action } => match self.zone(key) {
                Some(zone) => {
                    let mods = shell::held_modifiers(app);
                    let point = Some(self.point(key, drag.at));
                    let items = if drag.offer.offers(URI_LIST) {
                        vec![DropItem::new(URI_LIST, encode_uri_list(&drag.payload.uris))]
                    } else {
                        Vec::new()
                    };
                    let data = DropData::new(items).local(drag.payload);
                    let dropped = Dropped::new(drag.offer, data, mods).point(point);
                    zone.deliver(app, dropped, tx);
                    outcome_of(*action)
                }
                None => DragOutcome::Cancelled,
            },
            Landing::Rejecting { key } => {
                if let Some(zone) = self.zone(key) {
                    zone.reject(app, drag.offer, tx);
                }
                DragOutcome::Cancelled
            }
            Landing::Nowhere => DragOutcome::Cancelled,
        };
        if let Some(end) = drag.on_end {
            end(app, outcome, tx);
        }
    }

    pub(crate) fn keep_alive(&self, app: &App) -> Option<El> {
        let shown_tab = shell::tabs::shown_tab(app);
        let slot = self.0.in_window.borrow();
        let drag = slot.as_ref()?;
        let tail = drag.tab_tail.as_deref()?;
        if drag.origin_tab == shown_tab && source_still_mounted(app, &drag.source_key, tail) {
            return None;
        }
        Some(
            el().key(tail.join("/"))
                .size(0.0, 0.0)
                .opacity(0.0)
                .focusable(false)
                .a11y_hidden()
                .on_drag(super::keep_alive_drag),
        )
    }

    pub(crate) fn keep_alive_drag(&self, app: &mut App, d: &mut Drag, tx: &mut Tx) {
        match d.phase {
            Phase::Down => {}
            Phase::Move if !self.0.swallow_until_up.get() && self.is_dragging() => {
                self.move_to(d.window_point(), tx);
            }
            Phase::Move => {}
            Phase::Up => {
                if self.0.swallow_until_up.replace(false) {
                    return;
                }
                if self.is_dragging() {
                    self.finish_gesture(app, d, tx);
                }
            }
        }
    }
}

fn negotiate_local(chosen: Option<DropAction>, offered: DropActions) -> Option<DropAction> {
    let action = chosen?;
    let allowed = match action {
        DropAction::Copy | DropAction::Ask => {
            offered.is_empty() || offered.contains(DropActions::COPY)
        }
        DropAction::Move => offered.contains(DropActions::MOVE),
        DropAction::Link => offered.contains(DropActions::LINK),
    };
    allowed.then_some(action)
}

fn outcome_of(action: DropAction) -> DragOutcome {
    match action {
        DropAction::Copy | DropAction::Ask => DragOutcome::Copied,
        DropAction::Move => DragOutcome::Moved,
        DropAction::Link => DragOutcome::Linked,
    }
}

fn source_still_mounted(app: &App, source_key: &str, tail: &[String]) -> bool {
    if source_key == "preview-handle" || source_key.ends_with("-picture") {
        return crate::preview::current_entry(app).is_some();
    }
    let Some(view) = shell::shown_view(app) else {
        return false;
    };
    if source_key.starts_with("columns-col-") {
        return (view.shows)("columns") && app.columns.has_column_key(source_key);
    }
    if source_key == "icons" || source_key == "list" {
        let from_results = tail.iter().any(|seg| seg == "filter-results");
        return (view.shows)(source_key) && app.filter.pane.shows() == from_results;
    }
    true
}
