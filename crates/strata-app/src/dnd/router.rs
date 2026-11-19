// SPDX-License-Identifier: MIT

//! Dual-mode drag-and-drop router: delegates to `strata_platform::dnd::Dnd` when attached to a
//! native window (`host::windowed`), and drives an in-window keyed `DropZone` hit-test session
//! on the headless offscreen host (`host::offscreen`) as well as for in-window sidebar row drags.

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use rune::{Drag, DragOutcome, El, Phase, Pick, Picked, Tx, el};
use strata_components::contract::{
    Dnd as Binder,
    dnd::{
        DragSource, DropData, DropHover, DropItem, DropOffer, DropZone, Dropped, ElementKey,
        URI_LIST,
    },
};
use strata_platform::dnd::{DropOffer as P1Offer, DropPoint, Error as P1Error};

use super::{
    adapter::{
        action_to_p1, drag_item, encode_uri_list, offer_of_p1, point_of_p1, set_running, target_of,
    },
    drag::{Change, Gesture, Hit, InWindowDrag, Launch, StandIn, TailProbe},
    registry::{self, Landing, PointZone, Probe, Registry, scoped_id},
};
use crate::{App, shell};

const SOURCE_SLOP: f32 = 8.0;

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

    /// A new press: a release Escape swallowed before is not this one's.
    fn arm(&self) {
        self.0.swallow_until_up.set(false);
    }

    /// Escape cancelled the drag, and the button has not come up since.
    fn swallowing(&self) -> bool {
        self.0.swallow_until_up.get()
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

    pub(crate) fn drop_target_by_point(&self, el: El, key: &str, zoned: PointZone) -> El {
        let PointZone { zone, action } = zoned;
        let rect = self
            .0
            .registry
            .borrow_mut()
            .put(key, zone.clone(), Some(action.clone()));
        if self.0.windowed.get() {
            let target = target_of(zone).action_at(
                move |app: &App, offer: &P1Offer, mods: Option<rune::Mods>, point: DropPoint| {
                    action(
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
        let offscreen = OffscreenSource {
            wiring: self.clone(),
            key: key.key().to_owned(),
            source,
            pressed: Cell::new(None),
        };
        el.key(key.key())
            .on_drag(move |app: &mut App, d: &mut Drag, tx: &mut Tx| offscreen.on_drag(app, d, tx))
    }

    /// A gesture on a source that starts its drag itself ([`Wiring::start`]). Returns whether
    /// the router took the event.
    pub(crate) fn drag_phase(&self, app: &mut App, gesture: Gesture<'_>, tx: &mut Tx) -> bool {
        let d = gesture.drag;
        if self.0.windowed.get() && self.0.p1.borrow().owns(d) {
            return true;
        }
        match d.phase {
            Phase::Down => {
                self.arm();
                self.remember(gesture);
                false
            }
            Phase::Move => self.phase_moved(gesture, tx),
            Phase::Up => {
                *self.0.phase.borrow_mut() = None;
                self.release(app, d, tx)
            }
        }
    }

    fn phase_moved(&self, gesture: Gesture<'_>, tx: &mut Tx) -> bool {
        if self.swallowing() {
            return true;
        }
        if self.is_dragging() {
            self.move_to(gesture.drag.window_point(), tx);
            return true;
        }
        self.remember(gesture);
        false
    }

    fn remember(&self, gesture: Gesture<'_>) {
        *self.0.phase.borrow_mut() = Some((*gesture.drag, gesture.source.to_owned()));
    }

    /// The button went up: a release Escape swallowed, or the drag's drop. Returns whether
    /// either took it.
    fn release(&self, app: &mut App, d: &Drag, tx: &mut Tx) -> bool {
        if self.0.swallow_until_up.replace(false) {
            return true;
        }
        if self.is_dragging() {
            self.finish_gesture(app, d, tx);
            return true;
        }
        false
    }

    pub(crate) fn start(
        &self,
        app: &mut App,
        mut launch: Launch,
        tx: &mut Tx,
    ) -> Result<(), String> {
        let Some((drag, key)) = self.0.phase.borrow_mut().take() else {
            return Err("no drag phase to start from".to_owned());
        };
        let gesture = Gesture {
            drag: &drag,
            source: &key,
        };
        if self.0.windowed.get() && self.start_p1(gesture, &mut launch, tx)? {
            return Ok(());
        }
        let in_window = self.in_window_drag(app, &key, launch);
        self.begin_in_window(in_window, &drag, tx);
        Ok(())
    }

    /// Starts the drag through P1. `Ok(false)` when no window is attached, so it runs in the
    /// window instead.
    fn start_p1(
        &self,
        gesture: Gesture<'_>,
        launch: &mut Launch,
        tx: &mut Tx,
    ) -> Result<bool, String> {
        let item = drag_item(&launch.payload, launch.icon.take());
        let end = launch.on_end.clone();
        set_running(Some(launch.payload.clone()));
        let res = self.0.p1.borrow().start_drag(
            tx,
            gesture.drag,
            gesture.source,
            item,
            move |app: &mut App, outcome, tx| {
                set_running(None);
                end(app, outcome, tx);
            },
        );
        match res {
            Ok(()) => Ok(true),
            Err(P1Error::NotAttached) => {
                set_running(None);
                Ok(false)
            }
            Err(err) => {
                set_running(None);
                Err(err.to_string())
            }
        }
    }

    pub(crate) fn start_sidebar_drag(&self, app: &App, launch: Launch) {
        let id = self.next_serial();
        let offer = DropOffer::new(Vec::<Cow<'static, str>>::new(), launch.payload.actions)
            .local(true)
            .serial(id);
        set_running(Some(launch.payload.clone()));
        *self.0.in_window.borrow_mut() = Some(InWindowDrag {
            id,
            source_key: String::new(),
            offer,
            payload: launch.payload,
            at: [0.0, 0.0],
            landing: Landing::Nowhere,
            seq: 0,
            origin_tab: shell::tabs::shown_tab(app),
            tab_tail: None,
            stand_in: StandIn::Waiting,
            on_end: Some(launch.on_end),
        });
    }

    /// A drag of `launch`'s files from the element `source_key`, not yet under way.
    fn in_window_drag(&self, app: &App, source_key: &str, launch: Launch) -> InWindowDrag {
        let id = self.next_serial();
        let offer = DropOffer::new(vec![Cow::Borrowed(URI_LIST)], launch.payload.actions)
            .local(true)
            .serial(id)
            .uris(launch.payload.uris.clone());
        InWindowDrag {
            id,
            source_key: source_key.to_owned(),
            offer,
            payload: launch.payload,
            at: [0.0, 0.0],
            landing: Landing::Nowhere,
            seq: 0,
            origin_tab: shell::tabs::shown_tab(app),
            tab_tail: None,
            stand_in: StandIn::Waiting,
            on_end: Some(launch.on_end),
        }
    }

    /// Puts `drag` under way from where the gesture `d` is now.
    fn begin_in_window(&self, mut drag: InWindowDrag, d: &Drag, tx: &mut Tx) {
        let [wx, wy] = d.window_point();
        drag.at = [wx, wy];
        let probe = TailProbe {
            id: drag.id,
            tab: drag.origin_tab,
            source_key: drag.source_key.clone(),
            at: (wx - d.dx, wy - d.dy),
        };
        set_running(Some(drag.payload.clone()));
        *self.0.in_window.borrow_mut() = Some(drag);
        self.capture_source_tail(probe, tx);
        self.move_to([wx, wy], tx);
    }

    fn capture_source_tail(&self, probe: TailProbe, tx: &mut Tx) {
        let me = self.clone();
        let (x, y) = probe.at;
        tx.element_at_in_with(
            probe.source_key.clone(),
            x,
            y,
            Pick::new().transparent(true),
            move |_: &mut App, picked: Option<Picked>, _: &mut Tx| {
                let Some(tail) = picked.and_then(|p| probe.tail_in(p.path())) else {
                    return;
                };
                let mut slot = me.0.in_window.borrow_mut();
                if let Some(active) = slot.as_mut().filter(|d| d.id == probe.id) {
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
                let path = picked.as_ref().map(Picked::path);
                me.landed(app, Hit { serial: seq, path }, tx);
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
        self.leave_landing(app, &drag, tx);
        if let Some(end) = drag.on_end {
            end(app, DragOutcome::Cancelled, tx);
        }
    }

    /// Tells the zone `drag` rests on that it left.
    fn leave_landing(&self, app: &mut App, drag: &InWindowDrag, tx: &mut Tx) {
        if let Some(zone) = drag.landing.key().and_then(|key| self.zone(key)) {
            zone.hover(app, DropHover::Left, &drag.offer, tx);
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
                let path = picked.as_ref().map(Picked::path);
                me.complete_drop(app, Hit { serial: id, path }, tx);
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
        self.0.registry.borrow().zone(key)
    }

    /// Where the drag `hit` names lands now, recorded on it. The drag stays only shared-borrowed
    /// while the zones are asked: they read whether a drag runs.
    fn reland(&self, app: &App, hit: Hit<'_>) -> Option<Change> {
        let change = {
            let slot = self.0.in_window.borrow();
            let drag = slot.as_ref().filter(|d| d.seq == hit.serial)?;
            let probe = Probe {
                offer: &drag.offer,
                at: drag.at,
                mods: shell::held_modifiers(app),
            };
            let new = self.0.registry.borrow().resolve(hit.path, app, &probe);
            Change {
                old: drag.landing.clone(),
                new,
                offer: drag.offer.clone(),
                at: drag.at,
            }
        };
        if let Some(drag) = self
            .0
            .in_window
            .borrow_mut()
            .as_mut()
            .filter(|d| d.seq == hit.serial)
        {
            drag.landing = change.new.clone();
        }
        Some(change)
    }

    fn landed(&self, app: &mut App, hit: Hit<'_>, tx: &mut Tx) {
        let Some(change) = self.reland(app, hit) else {
            return;
        };
        if change.old != change.new {
            self.announce(app, &change, tx);
        }
        // Rejecting zones also hear moves so listing/strip edge-scroll continues over same-dir panes.
        if let Some(key) = change.new.key()
            && let Some(zone) = self.zone(key)
        {
            let point = self.0.registry.borrow().point(key, change.at);
            zone.moved(app, point, tx);
        }
    }

    /// Tells the zones a drag left and entered.
    fn announce(&self, app: &mut App, change: &Change, tx: &mut Tx) {
        if change.old.key() != change.new.key()
            && let Some(old) = change.old.key().and_then(|k| self.zone(k))
        {
            old.hover(app, DropHover::Left, &change.offer, tx);
        }
        if let Some(new) = change.new.key().and_then(|k| self.zone(k)) {
            new.hover(app, change.new.hover(), &change.offer, tx);
        }
    }

    fn complete_drop(&self, app: &mut App, hit: Hit<'_>, tx: &mut Tx) {
        let Some(seq) = self
            .0
            .in_window
            .borrow()
            .as_ref()
            .filter(|d| d.id == hit.serial)
            .map(|d| d.seq)
        else {
            return;
        };
        self.landed(app, Hit { serial: seq, ..hit }, tx);
        let Some(drag) = self
            .0
            .in_window
            .borrow_mut()
            .take_if(|d| d.id == hit.serial)
        else {
            return;
        };
        set_running(None);
        self.leave_landing(app, &drag, tx);
        let on_end = drag.on_end.clone();
        let outcome = self.deliver(app, drag, tx);
        if let Some(end) = on_end {
            end(app, outcome, tx);
        }
    }

    /// Gives `drag` to the zone it landed on, and says how it ended.
    fn deliver(&self, app: &mut App, drag: InWindowDrag, tx: &mut Tx) -> DragOutcome {
        match drag.landing.clone() {
            Landing::Accepting { key, action } => {
                let Some(zone) = self.zone(&key) else {
                    return DragOutcome::Cancelled;
                };
                let dropped = self.dropped(app, &key, drag);
                zone.deliver(app, dropped, tx);
                registry::outcome_of(action)
            }
            Landing::Rejecting { key } => {
                if let Some(zone) = self.zone(&key) {
                    zone.reject(app, drag.offer, tx);
                }
                DragOutcome::Cancelled
            }
            Landing::Nowhere => DragOutcome::Cancelled,
        }
    }

    /// `drag` as the zone `key` receives it.
    fn dropped(&self, app: &App, key: &str, drag: InWindowDrag) -> Dropped {
        let mods = shell::held_modifiers(app);
        let point = Some(self.0.registry.borrow().point(key, drag.at));
        let items = if drag.offer.offers(URI_LIST) {
            vec![DropItem::new(URI_LIST, encode_uri_list(&drag.payload.uris))]
        } else {
            Vec::new()
        };
        let data = DropData::new(items).local(drag.payload);
        Dropped::new(drag.offer, data, mods).point(point)
    }

    /// Steps the drag's stand-in ([`StandIn`]) once a frame, before the view is built: due
    /// when the source has left, mounted in the next frame, which it wakes. True when the view
    /// has to be built again to mount it.
    pub(crate) fn poll_stand_in(&self, app: &App, tx: &mut Tx) -> bool {
        let shown_tab = shell::tabs::shown_tab(app);
        let mut slot = self.0.in_window.borrow_mut();
        let Some(drag) = slot.as_mut().filter(|d| d.tab_tail.is_some()) else {
            return false;
        };
        let needed = drag.origin_tab != shown_tab || !drag.source_mounted(app);
        let (next, rebuild) = drag.stand_in.step(needed);
        drag.stand_in = next;
        if next == StandIn::Due {
            // The source's subtree unmounts at once rather than fading out, so its scene entry
            // is gone when the stand-in, which Rune gives the source's id, mounts next frame.
            tx.instant();
            tx.wake_after(Duration::ZERO);
        }
        rebuild
    }

    pub(crate) fn keep_alive(&self, _app: &App) -> Option<El> {
        let slot = self.0.in_window.borrow();
        let drag = slot.as_ref().filter(|d| d.stand_in == StandIn::Mounted)?;
        let tail = drag.tab_tail.as_deref()?;
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
            Phase::Move if !self.swallowing() && self.is_dragging() => {
                self.move_to(d.window_point(), tx);
            }
            Phase::Move => {}
            Phase::Up => {
                self.release(app, d, tx);
            }
        }
    }
}

/// A drag source on the offscreen host: it starts an in-window drag once the pointer has moved
/// past the slop from where it went down.
struct OffscreenSource {
    wiring: Wiring,
    key: String,
    source: DragSource<App>,
    pressed: Cell<Option<[f32; 2]>>,
}

impl OffscreenSource {
    fn on_drag(&self, app: &mut App, d: &mut Drag, tx: &mut Tx) {
        match d.phase {
            Phase::Down => {
                self.wiring.arm();
                self.pressed.set(Some(d.window_point()));
            }
            Phase::Move => self.moved(app, d, tx),
            Phase::Up => {
                self.pressed.set(None);
                self.wiring.release(app, d, tx);
            }
        }
    }

    fn moved(&self, app: &mut App, d: &Drag, tx: &mut Tx) {
        let wiring = &self.wiring;
        if wiring.swallowing() {
            return;
        }
        if wiring.is_dragging() {
            wiring.move_to(d.window_point(), tx);
            return;
        }
        if !self.past_slop(d) {
            return;
        }
        let Some(payload) = self.source.payload(app) else {
            return;
        };
        super::on_preview_drag_start(app, tx);
        let source = self.source.clone();
        let launch = Launch::new(payload, move |app, outcome, tx| {
            super::on_drag_ended(app, tx);
            source.end(app, outcome, tx);
        });
        let drag = wiring.in_window_drag(app, &self.key, launch);
        wiring.begin_in_window(drag, d, tx);
    }

    fn past_slop(&self, d: &Drag) -> bool {
        let [wx, wy] = d.window_point();
        self.pressed
            .get()
            .is_some_and(|[sx, sy]| (wx - sx).abs().max((wy - sy).abs()) > SOURCE_SLOP)
    }
}
