//! Wayland: protocols bound on winit's own display (shared), or on a connection of our own.
//!
//! **Shared** binds a second `wl_seat` (with its own `wl_pointer` and `wl_keyboard`, for input
//! serials only, never input), `wl_data_device`, and data control if asked, in a queue of our
//! own on winit's display. Rune exposes no display handle (draft U9, not filed), so it is
//! recovered from the window's surface ([`display`]). The reader thread ([`reader`]) asks the
//! UI thread for a turn, which runs [`Conn::dispatch`]: handlers run on the UI thread, where
//! `WindowHandle::with_raw_handle` may be called. Any protocol error on these objects ends
//! winit's connection with ours, so requests the protocols forbid are refused before they are
//! sent.
//!
//! **Own** connects to `WAYLAND_DISPLAY` and binds data control alone: no surface, serial or
//! focus needed, and it works on libwayland 1.22. Its thread dispatches by itself.

mod control;
mod display;
mod echo;
mod offer;
mod reader;

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use rune::WindowHandle;
use rune::raw_window_handle::RawWindowHandle;
use wayland_backend::client::ObjectId;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child,
};

use super::data::SourceSpec;
use super::error::{Error, Unsupported};
use super::event::{Action, Event, Route, Selection, SelectionOffer, SourceId, Subscribers};
use super::gate::Gate;
use super::pipe;
use super::windows::Windows;
use control::Control;
use echo::Echoes;
pub use offer::DragOffer;
use offer::{AnyOffer, Home, WlSelection};
use reader::Reader;

/// Which data-control protocol to bind when the compositor has both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Prefer {
    /// `ext_data_control_v1`, the standard one.
    #[default]
    Ext,
    /// `zwlr_data_control_v1`, which older wlroots compositors have alone.
    Wlr,
}

const BTN_LEFT: u32 = 0x110;

const ON_UI: &str = "winit's display: call on the UI thread";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Shared,
    Own,
}

/// The current selection offer on one route.
struct Current {
    offer: Arc<WlSelection>,
}

/// A drag in progress over this client.
struct Drag {
    offer: DragOffer,
    /// The app window it is over (its key), or None for a surface that is no window of the
    /// app's (winit's decoration subsurfaces): those get no events.
    window: Option<String>,
}

enum SourceProxy {
    Device(WlDataSource),
    Control(control::Source),
}

impl SourceProxy {
    fn destroy(&self) {
        match self {
            SourceProxy::Device(s) => s.destroy(),
            SourceProxy::Control(s) => s.destroy(),
        }
    }
}

struct Source {
    spec: SourceSpec,
    proxy: SourceProxy,
}

/// What the queue's handlers update. Handlers of a shared connection run on the UI thread.
pub(crate) struct State {
    kind: Kind,
    home: Home,
    windows: Arc<Windows>,
    seat: Option<WlSeat>,
    /// For P1's drag icons, bound on winit's display only.
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    ddm: Option<WlDataDeviceManager>,
    device: Option<WlDataDevice>,
    control: Option<Control>,
    /// The newest key or button serial; a data-device write needs a recent one.
    latest_serial: Option<u32>,
    /// The newest left-button press serial, for a drag (P1).
    press_serial: Option<u32>,
    /// This client has keyboard focus (our own `wl_keyboard` is entered).
    focused: bool,
    /// Data-device writes waiting to learn whether they took.
    echoes: Echoes,
    /// Offers being announced: their MIME types arrive before the event that uses them.
    announced: HashMap<ObjectId, Vec<String>>,
    /// A drag offer's source actions, which also arrive before its `enter`.
    source_actions: HashMap<ObjectId, u32>,
    selections: HashMap<(Selection, Route), Current>,
    drag: Option<Drag>,
    /// Dropped drag offers the app hasn't finished, closed at shutdown.
    dropped: Vec<DragOffer>,
    sources: HashMap<u64, Source>,
    events: Vec<Event>,
    /// Room for the state of protocols other modules bind on this queue (P4's
    /// `zxdg_importer_v2`): one value per type, which their own `Dispatch<_, _> for State`
    /// impls reach through [`State::extension`].
    extensions: HashMap<TypeId, Box<dyn Any + Send>>,
}

impl State {
    fn new(kind: Kind, home: Home, windows: Arc<Windows>) -> State {
        State {
            kind,
            home,
            windows,
            seat: None,
            compositor: None,
            shm: None,
            pointer: None,
            keyboard: None,
            ddm: None,
            device: None,
            control: None,
            latest_serial: None,
            press_serial: None,
            focused: false,
            echoes: Echoes::default(),
            announced: HashMap::new(),
            source_actions: HashMap::new(),
            selections: HashMap::new(),
            drag: None,
            dropped: Vec::new(),
            sources: HashMap::new(),
            events: Vec::new(),
            extensions: HashMap::new(),
        }
    }

    /// The extension state of type `T`, made with `Default` on first use. For other modules'
    /// handlers on this queue; see [`Conn::with_globals`].
    #[cfg_attr(
        not(any(feature = "dnd", feature = "foreign-parent")),
        expect(
            dead_code,
            reason = "the extension point for P1 (dnd) and P4 (foreign-parent)"
        )
    )]
    pub(crate) fn extension<T: Any + Send + Default>(&mut self) -> &mut T {
        self.extensions
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(T::default()))
            .downcast_mut::<T>()
            .unwrap_or_else(|| unreachable!("keyed by its own type"))
    }

    /// Replaces the current offer of a route, destroying the old one.
    fn set_selection_offer(
        &mut self,
        selection: Selection,
        route: Route,
        offer: Option<AnyOffer>,
        id: Option<ObjectId>,
    ) {
        if let Some(old) = self.selections.remove(&(selection, route)) {
            old.offer.close();
        }
        let offer = offer.map(|proxy| {
            let mimes = id
                .and_then(|id| self.announced.remove(&id))
                .unwrap_or_default();
            let offer = WlSelection::new(Some(proxy), mimes, self.home.clone());
            self.selections.insert(
                (selection, route),
                Current {
                    offer: offer.clone(),
                },
            );
            SelectionOffer(offer)
        });
        self.events.push(Event::SelectionChanged {
            selection,
            route,
            offer,
        });
    }

    /// A reader asked one of our sources for `mime`: its writer runs on a worker.
    fn source_send(&mut self, id: u64, mime: String, fd: std::os::fd::OwnedFd) {
        if let Some(writer) = self.sources.get(&id).and_then(|s| s.spec.writer(&mime)) {
            pipe::spawn_write(fd, writer.clone());
        }
        self.events.push(Event::SourceAsked {
            source: SourceId::from_raw(id),
            mime,
        });
    }

    fn source_cancelled(&mut self, id: u64) {
        if let Some(s) = self.sources.remove(&id) {
            s.proxy.destroy();
        }
        self.events.push(Event::SourceCancelled {
            source: SourceId::from_raw(id),
        });
    }

    /// The key of the app window whose surface is `surface`. Only the pointers are compared:
    /// the event's surface may be stale by now, so no request is ever sent on it.
    fn window_of(&self, surface: &WlSurface) -> Option<String> {
        let ptr = surface.id().as_ptr().cast::<c_void>();
        self.windows.find(|w| {
            w.with_raw_handle(
                |h| matches!(h.as_raw(), RawWindowHandle::Wayland(s) if s.surface.as_ptr() == ptr),
            )
            .unwrap_or(false)
        })
    }

    /// The connection is dead: nothing is held or heard on it. owns() turns false for every
    /// source, and no offer is left to read or finish: the selections, the drag over a window
    /// and the dropped offers not yet finished all close (P0's review, S3), so a handle the
    /// app still holds answers `Gone` instead of sending on a dead connection.
    fn forget_all(&mut self) {
        self.sources.clear();
        for (_, current) in self.selections.drain() {
            current.offer.close();
        }
        if let Some(drag) = self.drag.take() {
            drag.offer.close();
        }
        for offer in self.dropped.drain(..) {
            offer.close();
        }
    }

    /// A drag that left without a drop is over: its offer goes.
    fn end_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            if drag.offer.with_state(|s| s.dropped()) {
                self.dropped.retain(DragOffer::is_open);
                self.dropped.push(drag.offer);
            } else {
                drag.offer.close();
            }
        }
    }
}

/// One Wayland connection with our queue on it.
pub(crate) struct Conn {
    /// With the reader's clone, the only strong references to the backend: [`Conn::shutdown`]
    /// drops it, on the UI thread, so a `Conn` dropped later (after `run` returns) makes no
    /// libwayland call. The backend's own drop destroys its queue, which needs the display.
    queue: Mutex<Option<EventQueue<State>>>,
    /// The connection, weakly, and the thread that may use it (the UI thread, when shared).
    home: Home,
    state: Mutex<State>,
    qh: QueueHandle<State>,
    /// Kept for the protocols other modules bind later ([`Conn::with_globals`]).
    globals: Mutex<Option<GlobalList>>,
    gate: Arc<Gate>,
    reader: Mutex<Option<Reader>>,
    subs: Subscribers,
    /// The attached windows (winit's display): the reader's turns go through them.
    windows: Arc<Windows>,
    /// A dispatch failed: the connection is dead, and that was reported once.
    failed: AtomicBool,
    /// A Turn ran dispatch off the UI thread, and that was reported once.
    misrouted: AtomicBool,
}

impl Conn {
    /// Binds on winit's display, found from `window`'s surface. Call it on the UI thread.
    /// The reader's turns go to `windows`' turns.
    pub(crate) fn shared(
        window: &WindowHandle,
        windows: Arc<Windows>,
        subs: Subscribers,
        data_control: Option<Prefer>,
        as_if_old: bool,
    ) -> Result<Arc<Conn>, Error> {
        let display = display::winit_display(window, as_if_old)?;
        let backend = display::foreign_backend(display);
        let conn = Connection::from_backend(backend);
        Conn::bind(Kind::Shared, conn, windows, subs, data_control)
    }

    /// Connects to the compositor on a connection of our own, for data control alone.
    /// `as_if_none`: a test switch that leaves the data-control globals unbound, as on a
    /// compositor without them.
    pub(crate) fn own(
        subs: Subscribers,
        prefer: Prefer,
        as_if_none: bool,
    ) -> Result<Arc<Conn>, Error> {
        let conn = Connection::connect_to_env().map_err(|e| Error::Connection(e.to_string()))?;
        let control = (!as_if_none).then_some(prefer);
        Conn::bind(Kind::Own, conn, Arc::default(), subs, control)
    }

    fn bind(
        kind: Kind,
        conn: Connection,
        windows: Arc<Windows>,
        subs: Subscribers,
        data_control: Option<Prefer>,
    ) -> Result<Arc<Conn>, Error> {
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).map_err(|e| Error::Connection(e.to_string()))?;
        let qh = queue.handle();
        let home = Home {
            backend: conn.backend().downgrade(),
            ui: (kind == Kind::Shared).then(|| std::thread::current().id()),
        };
        let mut state = State::new(kind, home.clone(), windows.clone());
        state.seat = globals.bind::<WlSeat, _, _>(&qh, 1..=8, ()).ok();
        if kind == Kind::Shared {
            state.compositor = globals.bind::<WlCompositor, _, _>(&qh, 1..=6, ()).ok();
            state.shm = globals.bind::<WlShm, _, _>(&qh, 1..=2, ()).ok();
            state.ddm = globals
                .bind::<WlDataDeviceManager, _, _>(&qh, 1..=3, ())
                .ok();
            if let (Some(ddm), Some(seat)) = (&state.ddm, &state.seat) {
                state.device = Some(ddm.get_data_device(seat, &qh, ()));
            }
        }
        if let (Some(prefer), Some(seat)) = (data_control, &state.seat) {
            state.control = Control::bind(&globals, &qh, seat, prefer);
        }
        if kind == Kind::Own && state.control.is_none() {
            return Err(Unsupported::NoProtocol("data control").into());
        }
        // Capabilities arrive in this round trip, and the seat handler binds the pointer and
        // keyboard only once the seat has them: `get_pointer` on a seat without a pointer is a
        // protocol error.
        queue
            .roundtrip(&mut state)
            .map_err(|e| Error::Connection(e.to_string()))?;
        let gate = Arc::new(Gate::default());
        let c = Arc::new(Conn {
            queue: Mutex::new(Some(queue)),
            home,
            globals: Mutex::new(Some(globals)),
            failed: AtomicBool::new(false),
            misrouted: AtomicBool::new(false),
            state: Mutex::new(state),
            qh,
            gate: gate.clone(),
            reader: Mutex::new(None),
            subs: subs.clone(),
            windows: windows.clone(),
        });
        let weak = Arc::downgrade(&c);
        let failed = {
            let weak = weak.clone();
            move |message: String| {
                if let Some(c) = weak.upgrade() {
                    c.report_failure(message);
                }
            }
        };
        let reader = match kind {
            Kind::Shared => {
                let gate = gate.clone();
                Reader::spawn(
                    "transfer-wl",
                    conn,
                    gate.clone(),
                    move || {
                        // Ask the UI thread to dispatch, through every attached window's app
                        // with no turn outstanding (see `windows`), then wait for it with no
                        // read intent held.
                        let before = gate.turns();
                        windows.post(|| {
                            let weak: Weak<Conn> = weak.clone();
                            Box::new(move || {
                                // On the UI thread, where Turn runs it; refused anywhere else, and
                                // the reader then waits for a turn that never comes: say so, once.
                                if let Some(c) = weak.upgrade()
                                    && let Err(e) = c.dispatch()
                                {
                                    c.report_misrouted_turn(e);
                                }
                            })
                        });
                        gate.wait(before);
                    },
                    failed,
                )
            }
            Kind::Own => Reader::spawn(
                "transfer-wl-own",
                conn,
                gate,
                move || {
                    if let Some(c) = weak.upgrade() {
                        c.dispatch_here();
                    }
                },
                failed,
            ),
        }
        .map_err(|e| Error::Connection(e.to_string()))?;
        *lock(&c.reader) = Some(reader);
        Ok(c)
    }

    pub(crate) fn data_control(&self) -> Option<&'static str> {
        lock(&self.state).control.as_ref().map(Control::name)
    }

    pub(crate) fn has_device(&self) -> bool {
        lock(&self.state).device.is_some()
    }

    /// Runs the queued events' handlers and hands their events to subscribers. Shared: the
    /// UI thread only, from the reader's turn or before reading a serial.
    pub(crate) fn dispatch(&self) -> Result<bool, Error> {
        // Handlers call `WindowHandle::with_raw_handle` and parse events naming winit's
        // objects: both only on the UI thread (see `reader`).
        if !self.home.here() {
            return Err(Error::Refused(ON_UI));
        }
        let any = self.dispatch_here();
        self.gate.turned();
        self.windows.prune();
        Ok(any)
    }

    /// Releases a reader waiting for a turn, so it asks again: the windows it asked through
    /// may have closed since. A no-op when it isn't waiting.
    pub(crate) fn wake_reader(&self) {
        self.gate.turned();
    }

    fn dispatch_here(&self) -> bool {
        if self.failed.load(Ordering::Acquire) {
            return false;
        }
        let (events, dead) = {
            let mut queue = lock(&self.queue);
            let Some(queue) = queue.as_mut() else {
                return false;
            };
            let mut state = lock(&self.state);
            let result = queue.dispatch_pending(&mut state);
            let _ = queue.flush();
            // wayland-client's dispatch_pending drops the inner queue's error (a protocol error,
            // a closed socket): the backend keeps it, so ask there.
            let dead = match result {
                Err(e) => Some(e.to_string()),
                Ok(_) => self
                    .home
                    .backend
                    .upgrade()
                    .and_then(|b| b.last_error())
                    .map(|e| e.to_string()),
            };
            (std::mem::take(&mut state.events), dead)
        };
        let any = !events.is_empty();
        self.subs.emit(events);
        if let Some(message) = dead {
            self.report_failure(message);
        }
        any
    }

    /// A Turn ran off the UI thread, so nothing was dispatched and the reader waits for a turn
    /// that never comes. Reported once, as a failure, since events stop.
    fn report_misrouted_turn(&self, e: Error) {
        if !self.misrouted.swap(true, Ordering::AcqRel) {
            self.subs.emit(vec![Event::Failed {
                message: format!("the Turn ran dispatch off the UI thread, so events stop: {e}"),
            }]);
        }
    }

    /// The connection is dead: say so once, from whichever thread noticed first, and dispatch
    /// no more.
    fn report_failure(&self, message: String) {
        if !self.failed.swap(true, Ordering::AcqRel) {
            lock(&self.state).forget_all();
            self.subs.emit(vec![Event::Failed { message }]);
        }
    }

    /// Runs `f` with the registry's globals and the queue, to bind a protocol P0 doesn't (P4's
    /// `zxdg_importer_v2`). The module binding it implements `Dispatch<I, U> for State` for its
    /// objects and keeps its state in [`State::extension`]. None after shutdown, or off the
    /// UI thread on winit's display.
    #[cfg_attr(
        not(any(feature = "dnd", feature = "foreign-parent")),
        expect(
            dead_code,
            reason = "the extension point for P1 (dnd) and P4 (foreign-parent)"
        )
    )]
    pub(crate) fn with_globals<R>(
        &self,
        f: impl FnOnce(&GlobalList, &QueueHandle<State>, &mut State) -> R,
    ) -> Option<R> {
        if !self.home.here() {
            return None;
        }
        let globals = lock(&self.globals);
        let mut state = lock(&self.state);
        Some(f(globals.as_ref()?, &self.qh, &mut state))
    }

    /// A test switch: makes the compositor end this connection with a protocol error. Only
    /// for a connection of our own (on winit's display it would end winit's too).
    pub(crate) fn provoke_protocol_error(&self) -> bool {
        if self.home.ui.is_some() {
            return false;
        }
        let state = lock(&self.state);
        let Some(control) = state.control.as_ref() else {
            return false;
        };
        control.provoke_protocol_error(&self.qh);
        drop(state);
        self.home.flush();
        true
    }

    pub(crate) fn serials(&self) -> (Option<u32>, Option<u32>) {
        let state = lock(&self.state);
        (state.latest_serial, state.press_serial)
    }

    pub(crate) fn selection(&self, selection: Selection, route: Route) -> Option<SelectionOffer> {
        lock(&self.state)
            .selections
            .get(&(selection, route))
            .map(|c| SelectionOffer(c.offer.clone()))
    }

    pub(crate) fn owns(&self, source: SourceId) -> bool {
        lock(&self.state).sources.contains_key(&source.get())
    }

    /// Whether `source` was cancelled because the compositor dropped its data-device write
    /// (see `echo`), rather than lost to another client.
    pub(crate) fn write_was_dropped(&self, source: SourceId) -> bool {
        lock(&self.state).echoes.was_dropped(source.get())
    }

    /// Offers `spec` as `selection` on `route`. A data-device write uses the newest input
    /// serial: call [`Conn::dispatch`] first so the event that triggered the copy is in.
    pub(crate) fn set_selection(
        &self,
        selection: Selection,
        route: Route,
        spec: SourceSpec,
    ) -> Result<SourceId, Error> {
        if spec.is_empty() {
            return Err(Error::Refused("a source with no types"));
        }
        if !self.home.here() {
            return Err(Error::Refused(ON_UI));
        }
        if self.failed.load(Ordering::Acquire) {
            return Err(Error::Connection("the connection failed".into()));
        }
        let id = SourceId::next();
        let mut state = lock(&self.state);
        let proxy = match route {
            Route::DataDevice => {
                if selection != Selection::Clipboard {
                    return Err(Error::Refused("the data device has no primary selection"));
                }
                let (Some(ddm), Some(device)) = (&state.ddm, &state.device) else {
                    return Err(Error::NoRoute("data device"));
                };
                let serial = state.latest_serial.ok_or(Error::NoSerial)?;
                let source = ddm.create_data_source(&self.qh, id.get());
                for mime in spec.mimes() {
                    source.offer(mime.to_owned());
                }
                // No set_actions: on a selection source it is a protocol error.
                device.set_selection(Some(&source), serial);
                // A write the compositor drops is told by nothing but a missing echo (`echo`).
                if state.focused
                    && let Some(backend) = self.home.backend.upgrade()
                {
                    let _ = Connection::from_backend(backend)
                        .display()
                        .sync(&self.qh, id.get());
                    state.echoes.arm(id.get());
                }
                SourceProxy::Device(source)
            }
            Route::DataControl => {
                let control = state
                    .control
                    .as_ref()
                    .ok_or(Error::NoRoute("data control"))?;
                SourceProxy::Control(control.set_selection(&self.qh, id.get(), selection, &spec)?)
            }
            Route::X11 => return Err(Error::NoRoute("X11")),
        };
        state.sources.insert(id.get(), Source { spec, proxy });
        drop(state);
        self.home.flush();
        Ok(id)
    }

    /// Gives up `source`'s selection, if it still holds it: the source is destroyed, which
    /// clears the selection if the compositor still has it as the selection.
    /// Subscribers hear [`Event::SourceCancelled`] at once. False if `source` no longer held
    /// anything. A data-device clear uses the newest input serial, as a write does.
    pub(crate) fn clear_selection(&self, source: SourceId) -> Result<bool, Error> {
        if !self.home.here() {
            return Err(Error::Refused(ON_UI));
        }
        if self.failed.load(Ordering::Acquire) {
            return Ok(false);
        }
        let mut state = lock(&self.state);
        let Some(held) = state.sources.remove(&source.get()) else {
            return Ok(false);
        };
        // Destroying the source clears the selection only while it still holds it. A data
        // control set_selection(None) carries no serial, so it would also wipe a copy another
        // app made since, which we haven't heard of yet. The data device's carries the newest
        // serial, which the compositor refuses when another copy is newer.
        if let (SourceProxy::Device(_), Some(device), Some(serial)) =
            (&held.proxy, &state.device, state.latest_serial)
        {
            device.set_selection(None, serial);
        }
        held.proxy.destroy();
        drop(state);
        self.home.flush();
        self.subs.emit(vec![Event::SourceCancelled { source }]);
        Ok(true)
    }

    /// Stops the reader, then destroys every object, on the UI thread for a shared
    /// connection, before winit closes the display (S2, step 6).
    pub(crate) fn shutdown(&self) {
        if let Some(reader) = lock(&self.reader).take() {
            reader.stop();
        }
        let mut state = lock(&self.state);
        for (_, current) in state.selections.drain() {
            current.offer.close();
        }
        if let Some(drag) = state.drag.take() {
            drag.offer.close();
        }
        for offer in state.dropped.drain(..) {
            offer.close();
        }
        for (_, source) in state.sources.drain() {
            source.proxy.destroy();
        }
        if let Some(control) = state.control.take() {
            control.destroy();
        }
        if let Some(device) = state.device.take()
            && device.version() >= 2
        {
            device.release();
        }
        if let Some(p) = state.pointer.take()
            && p.version() >= 3
        {
            p.release();
        }
        if let Some(k) = state.keyboard.take()
            && k.version() >= 3
        {
            k.release();
        }
        if let Some(seat) = state.seat.take()
            && seat.version() >= 5
        {
            seat.release();
        }
        if let Some(shm) = state.shm.take()
            && shm.version() >= 2
        {
            shm.release();
        }
        // wl_compositor has no destructor.
        state.compositor = None;
        state.extensions.clear();
        drop(state);
        drop(lock(&self.globals).take());
        self.home.flush();
        // The last strong references to the backend go now, while the display lives.
        drop(lock(&self.queue).take());
    }
}

/// What P1 (`dnd`) reads of the queue's state, from its own handlers on this queue and from
/// [`Conn::with_globals`]. Read-only: P0's handlers keep these fields.
#[cfg(feature = "dnd")]
impl State {
    pub(crate) fn data_device_manager(&self) -> Option<&WlDataDeviceManager> {
        self.ddm.as_ref()
    }

    pub(crate) fn data_device(&self) -> Option<&WlDataDevice> {
        self.device.as_ref()
    }

    pub(crate) fn compositor(&self) -> Option<&WlCompositor> {
        self.compositor.as_ref()
    }

    pub(crate) fn shm(&self) -> Option<&WlShm> {
        self.shm.as_ref()
    }

    /// The newest left-button press serial, for `start_drag`.
    pub(crate) fn press_serial(&self) -> Option<u32> {
        self.press_serial
    }
}

#[cfg(any(feature = "dnd", feature = "foreign-parent"))]
impl Conn {
    /// Sends what was queued. Requests P1 and P4 make through [`Conn::with_globals`] need it.
    pub(crate) fn flush(&self) {
        self.home.flush();
    }

    /// The connection failed: no request should be made on it.
    pub(crate) fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// Runs `f` with a proxy for `window`'s `wl_surface` on this connection, for that call
    /// only. Winit's display only, on the UI thread.
    pub(crate) fn with_window_surface<R>(
        &self,
        window: &WindowHandle,
        f: impl FnOnce(&WlSurface) -> R,
    ) -> Result<R, Error> {
        if self.home.ui.is_none() {
            return Err(Error::Refused("a connection of our own has no app windows"));
        }
        if !self.home.here() {
            return Err(Error::Refused(ON_UI));
        }
        let backend = self.home.backend.upgrade().ok_or(Error::Shutdown)?;
        display::with_surface(&Connection::from_backend(backend), window, f)
    }
}

/// What P4 (`foreign-parent`) builds on: a round trip, to learn at once whether the compositor
/// destroyed an import.
#[cfg(feature = "foreign-parent")]
impl Conn {
    /// Sends what is queued, waits for the compositor to answer a `wl_display.sync`, and runs
    /// every handler on the queue meanwhile, P0's included: their events reach subscribers as
    /// [`Conn::dispatch`]'s do. Winit's display only, on the UI thread, never inside
    /// [`Conn::with_globals`] (both lock the state).
    ///
    /// It blocks the UI thread for one round trip, **with no limit**: a compositor that stops
    /// answering holds the UI thread until it answers or the connection ends. That is left
    /// unbounded on purpose: such a compositor stalls winit's own frames and requests on the
    /// same connection anyway, and a round trip given up midway would leave its import
    /// half made, its answer due later.
    ///
    /// Only libwayland's own error ([`Failure::Dead`]) marks the connection dead. A full
    /// socket buffer is waited out ([`BUSY_TRIES`] times, [`BUSY_WAIT`] each), then reported
    /// as [`Error::Busy`] with the connection alive. So is an event this client couldn't parse
    /// ([`Failure::Other`]), named as such.
    ///
    /// The reader thread may hold a read intent meanwhile: libwayland's read protocol lets two
    /// threads prepare and read at once, and the reader never waits on the UI thread while it
    /// holds one (see `reader`).
    pub(crate) fn roundtrip(&self) -> Result<(), Error> {
        if self.home.ui.is_none() {
            return Err(Error::Refused("a round trip is for winit's display"));
        }
        if !self.home.here() {
            return Err(Error::Refused(ON_UI));
        }
        if self.failed.load(Ordering::Acquire) {
            return Err(Error::Connection("the connection failed".into()));
        }
        let (events, dead) = {
            let mut queue = lock(&self.queue);
            let Some(queue) = queue.as_mut() else {
                return Err(Error::Shutdown);
            };
            let mut state = lock(&self.state);
            let mut tries = 0;
            let outcome = loop {
                let result = queue.roundtrip(&mut state);
                let backend = self.home.backend.upgrade();
                let last_error = backend
                    .as_ref()
                    .and_then(|b| b.last_error())
                    .map(|e| e.to_string());
                let failure = match result {
                    Ok(_) => last_error.map(Failure::Dead),
                    Err(e) => Some(classify(&e, last_error)),
                };
                match failure {
                    Some(Failure::Busy) if tries < BUSY_TRIES => {
                        tries += 1;
                        // Wait until the compositor reads, then send the sync again: the
                        // first one's answer, if it comes, is ignored.
                        if let Some(b) = &backend {
                            let _ = pipe::poll(&[(b.poll_fd(), libc::POLLOUT)], Some(BUSY_WAIT));
                        }
                    }
                    Some(Failure::Busy) => {
                        break Err(Error::Busy(
                            "the compositor isn't reading: the socket stayed full".into(),
                        ));
                    }
                    Some(Failure::Other(message)) => {
                        break Err(Error::Busy(format!(
                            "an event from the compositor didn't parse: {message}"
                        )));
                    }
                    Some(Failure::Dead(message)) => break Ok(Some(message)),
                    None => break Ok(None),
                }
            };
            (std::mem::take(&mut state.events), outcome)
        };
        self.subs.emit(events);
        self.gate.turned();
        match dead {
            Ok(Some(message)) => {
                self.report_failure(message.clone());
                Err(Error::Connection(message))
            }
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// How a failed round trip leaves the connection.
#[cfg(feature = "foreign-parent")]
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    /// libwayland has an error: the connection is dead, for good.
    Dead(String),
    /// The socket's buffer was full (`EAGAIN` on the flush): wait for the socket and try
    /// again.
    Busy,
    /// Anything else libwayland didn't keep, in practice `DispatchError::BadMessage` (an event
    /// wayland-client couldn't parse): this round trip failed, and the connection lives on.
    Other(String),
}

/// `last_error` is the backend's: wayland-backend (0.3.17) keeps every I/O error of
/// `wl_display_flush` and `wl_display_read_events` there except `EAGAIN`, which
/// `blocking_dispatch` passes on as it is. `EINTR` included: libwayland counts it fatal in
/// both, and wayland-client's `blocking_read` retries it in its own poll. So an I/O error
/// with no last error is `EAGAIN` from the flush.
#[cfg(feature = "foreign-parent")]
fn classify(e: &wayland_client::DispatchError, last_error: Option<String>) -> Failure {
    use std::io::ErrorKind;
    use wayland_backend::client::WaylandError;
    use wayland_client::DispatchError;

    if let Some(message) = last_error {
        return Failure::Dead(message);
    }
    match e {
        DispatchError::Backend(WaylandError::Io(io)) if io.kind() == ErrorKind::WouldBlock => {
            Failure::Busy
        }
        e => Failure::Other(e.to_string()),
    }
}

/// How often a round trip waits for a full socket to drain, and how long each time.
#[cfg(feature = "foreign-parent")]
const BUSY_TRIES: u32 = 4;
#[cfg(feature = "foreign-parent")]
const BUSY_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

#[cfg(test)]
mod tests;

// ---- handlers ----

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// `format` events list the pixel formats; P1's icons use ARGB8888, which every `wl_shm` has.
impl Dispatch<WlShm, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlShm,
        _: <WlShm as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: <WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        st: &mut Self,
        seat: &WlSeat,
        e: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = e
        else {
            return;
        };
        // Serials only matter on winit's display: they come from input to the app's windows.
        if st.kind != Kind::Shared {
            return;
        }
        if caps.contains(wl_seat::Capability::Pointer) && st.pointer.is_none() {
            st.pointer = Some(seat.get_pointer(qh, ()));
        }
        // One keyboard per display: each bind makes the compositor send every keyboard of
        // the client its keymap again, winit's too.
        if caps.contains(wl_seat::Capability::Keyboard) && st.keyboard.is_none() {
            st.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        st: &mut Self,
        _: &WlPointer,
        e: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            button,
            state,
            ..
        } = e
        {
            st.latest_serial = Some(serial);
            if button == BTN_LEFT && matches!(state, WEnum::Value(wl_pointer::ButtonState::Pressed))
            {
                st.press_serial = Some(serial);
            }
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        st: &mut Self,
        _: &WlKeyboard,
        e: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            // Dropping the fd closes it.
            wl_keyboard::Event::Keymap { fd, .. } => drop(fd),
            wl_keyboard::Event::Key { serial, .. } => st.latest_serial = Some(serial),
            wl_keyboard::Event::Enter { .. } => st.focused = true,
            wl_keyboard::Event::Leave { .. } => st.focused = false,
            _ => {}
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    event_created_child!(State, WlDataDevice, [wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ())]);

    fn event(
        st: &mut Self,
        _: &WlDataDevice,
        e: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_data_device::Event::DataOffer { id } => {
                st.announced.insert(id.id(), Vec::new());
            }
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id,
            } => {
                st.end_drag();
                let Some(proxy) = id else {
                    return;
                };
                let mimes = st.announced.remove(&proxy.id()).unwrap_or_default();
                let actions = st.source_actions.remove(&proxy.id());
                // Anything else announced for an earlier drag is stale now.
                st.source_actions.clear();
                let offer = DragOffer::new(proxy, mimes, serial, st.home.clone());
                if let Some(a) = actions {
                    offer.with_state(|s| s.on_source_actions(a));
                }
                let window = st.window_of(&surface);
                if let Some(window) = &window {
                    st.events.push(Event::DragEnter {
                        window: window.clone(),
                        offer: offer.clone(),
                        x,
                        y,
                    });
                    if let Some(a) = actions {
                        st.events.push(Event::DragActions {
                            source: Action::all_in(a),
                            chosen: None,
                        });
                    }
                }
                st.drag = Some(Drag { offer, window });
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if st.drag.as_ref().is_some_and(|d| d.window.is_some()) {
                    st.events.push(Event::DragMotion { x, y });
                }
            }
            wl_data_device::Event::Leave => {
                if st.drag.as_ref().is_some_and(|d| d.window.is_some()) {
                    st.events.push(Event::DragLeave);
                }
                // A dropped offer lives on until the app finishes it: the compositor sends
                // leave right after the drop.
                st.end_drag();
            }
            wl_data_device::Event::Drop => {
                if let Some(drag) = &st.drag {
                    drag.offer.with_state(|s| s.on_drop());
                    if drag.window.is_some() {
                        st.events.push(Event::Drop);
                    }
                }
            }
            wl_data_device::Event::Selection { id } => {
                st.echoes.selection();
                let oid = id.as_ref().map(Proxy::id);
                st.set_selection_offer(
                    Selection::Clipboard,
                    Route::DataDevice,
                    id.map(AnyOffer::Device),
                    oid,
                );
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        st: &mut Self,
        o: &WlDataOffer,
        e: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = o.id();
        let drag = st.drag.as_ref().filter(|d| d.offer.is(&id));
        match e {
            wl_data_offer::Event::Offer { mime_type } => {
                if let Some(mimes) = st.announced.get_mut(&id) {
                    mimes.push(mime_type);
                }
            }
            wl_data_offer::Event::SourceActions {
                source_actions: WEnum::Value(a),
            } => match drag {
                Some(d) => {
                    d.offer.with_state(|s| s.on_source_actions(a.bits()));
                    if d.window.is_some() {
                        st.events.push(Event::DragActions {
                            source: Action::all_in(a.bits()),
                            chosen: None,
                        });
                    }
                }
                // Not entered yet: told at enter.
                None => {
                    st.source_actions.insert(id, a.bits());
                }
            },
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(a),
            } => {
                if let Some(d) = drag {
                    d.offer.with_state(|s| s.on_action(a.bits()));
                    if d.window.is_some() {
                        st.events.push(Event::DragActions {
                            source: Vec::new(),
                            chosen: Action::from_bits(a.bits()),
                        });
                    }
                } else if let Some(dropped) = st.dropped.iter().find(|o| o.is(&id)) {
                    // A dropped offer outlives the `leave` that follows its drop, and the
                    // compositor may still change its action (answering a set_actions sent
                    // before the drop): `finish` must see that.
                    dropped.with_state(|s| s.on_action(a.bits()));
                }
            }
            _ => {}
        }
    }
}

/// The sync after a data-device write (see `echo`): a write with no echo before it was
/// dropped, and its source is reported cancelled, as a lost selection is.
impl Dispatch<WlCallback, u64> for State {
    fn event(
        st: &mut Self,
        _: &WlCallback,
        e: wl_callback::Event,
        id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = e
            && st.echoes.done(*id)
            && st.sources.contains_key(id)
        {
            st.source_cancelled(*id);
        }
    }
}

impl Dispatch<WlDataSource, u64> for State {
    fn event(
        st: &mut Self,
        _: &WlDataSource,
        e: wl_data_source::Event,
        id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_data_source::Event::Send { mime_type, fd } => st.source_send(*id, mime_type, fd),
            wl_data_source::Event::Cancelled => st.source_cancelled(*id),
            _ => {}
        }
    }
}
