//! The transfer substrate: protocols bound beside winit, multi-MIME offers and sources, and
//! X11 selections. Internal to `dnd`, `clipboard` and `foreign-parent`, which build their own
//! APIs on it; reachable (hidden) only for strata-platform's own session tests.
//!
//! Rune lacks: a display handle on `WindowHandle`, a hook before winit closes the display, and
//! an every-turn hook to dispatch a queue of one's own (all three: draft U9, not filed; see
//! `docs/decisions/S5-upstream.md#u9`). Its consumers' asks are drafts U1, U2 and U5.
//! Rune issue: none filed yet.
//!
//! Work package: P0, `docs/work/phase-1-leaves/P0-transfer-substrate.md`. The approach is
//! S2's (`docs/decisions/S2-transfer.md`, "P0 binding approach").
//!
//! # Routes
//!
//! | Route | Where | Needs |
//! | --- | --- | --- |
//! | [`Route::DataDevice`] | `wl_data_device` on winit's display | libwayland ≥ 1.23; focus to hear, a fresh input serial to write |
//! | [`Route::DataControl`] | ext or wlr data control, on winit's display or a connection of our own | a compositor with data control (not mutter) |
//! | [`Route::X11`] | `CLIPBOARD` and `PRIMARY` on a second X connection | an X11 window, or a Wayland window with no Wayland route (below) |
//!
//! A Wayland window with neither winit's display (libwayland < 1.23) nor data control on a
//! connection of its own (mutter has none) has no Wayland route at all: Ubuntu 24.04's GNOME
//! session, an unsupported environment (libwayland 1.23 is the minimum, S3 decision 1). There
//! the selections go through Xwayland instead ([`Status::x11`]): X11
//! selections on a connection of our own, which the compositor bridges to Wayland clients,
//! as arboard falls back (the owner's choice, option a of S2's first question).
//!
//! Drags into a Wayland window arrive as [`Event::DragEnter`] and the events after it, on
//! winit's display. X11 drag and drop, and every drag out, are P1's.
//!
//! # One substrate per process
//!
//! [`Transfer::attach`] binds once per process: winit has one display, and each keyboard
//! bound on it makes the compositor resend every keyboard's keymap. Later calls add the window
//! and return the same substrate. Results and events reach the app off the UI thread: a
//! subscriber's notify and a read's `done` hand them over with `AppHandle::update`.
//! A [`Subscription`] dropped unsubscribes.
//!
//! Each window keeps the [`Turn`] it attached with, so pass a real one on every attach, built
//! from that window's app: the reader's turns go to every attached window with none
//! outstanding, so they keep running when the first window to attach closes, as long as any
//! attached window lives (`windows`). A window attached again replaces its turn.
//!
//! # Threads
//!
//! On winit's display, everything that sends a request or dispatches runs on the UI thread,
//! and is refused on any other: [`Transfer::dispatch`], [`Transfer::set_selection`], and every
//! request on an offer. Events naming winit's objects may only be parsed there (the reader
//! never dispatches; see `wayland::reader`), and a request's brief hold on the connection must
//! not outlive the UI thread's shutdown. On X11 and on a Wayland connection of our own, any
//! thread may call.
//!
//! # Protocols other modules bind
//!
//! On winit's display P0 binds `wl_seat` (with a pointer and keyboard for serials),
//! `wl_data_device_manager` and its device, data control if asked for, and `wl_compositor`
//! and `wl_shm` for P1's drag icons. It keeps the registry's globals for the rest:
//! `wayland::Conn::with_globals` hands them and the queue to another module (P4's
//! `zxdg_importer_v2`), which implements `Dispatch` for P0's `State` for its own objects and
//! keeps its own state in `State::extension`.
//!
//! # Teardown is the app's job
//!
//! Call [`Transfer::shutdown`] on the UI thread before the event loop ends: before
//! `Tx::quit` or `AppHandle::quit`, and in `App::on_close` of the last window when closing it
//! ends the app. It stops the reader and destroys everything bound on winit's display while
//! the display is alive, the wayland-backend on it included. Rune runs no hook before winit
//! closes it (U9), and after a quit no further `on_close` runs. After a shutdown, handles and
//! offers the app still holds are safe to drop at any time, after `run` too: they make no
//! libwayland call. Without the shutdown, the backend's late drop destroys its proxies on the
//! freed display (SIGSEGV in every S2 run, and in P0's quit test with the backend's drop
//! removed). A substrate never shut down is never dropped (it lives in a static), but its
//! reader then stays parked on a display winit has freed, which is unsound if the process
//! lives on.

mod data;
mod error;
mod event;
mod gate;
pub mod mime;
pub(crate) mod pipe;
pub(crate) mod wayland;
mod windows;
pub(crate) mod x11;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rune::WindowHandle;
use rune::raw_window_handle::RawWindowHandle;

pub use data::{ReadLimits, SourceSpec, Writer};
pub use error::{Error, ReadError, Unsupported};
pub use event::{Action, Event, Route, Selection, SelectionOffer, SourceId, Subscription};
pub use wayland::{DragOffer, Prefer};
pub use x11::HandOff;

use event::Subscribers;
use windows::Windows;

/// Runs a closure on the UI thread soon. Build it from the attaching window's app handle:
/// `Arc::new(move |f| handle.update_if(move |_, _| { f(); false }))`. Returning false keeps
/// the turn from redrawing, whose frame callback would wake the reader again. A turn of a
/// window whose app has closed never runs (see the module docs).
pub type Turn = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

/// Where data control goes on Wayland.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DataControl {
    /// No data control.
    Off,
    /// On winit's display only.
    Shared,
    /// On a connection of our own, which needs neither winit's display nor libwayland 1.23.
    Own,
    /// On winit's display, else on a connection of our own.
    #[default]
    SharedElseOwn,
}

/// What to bind at attach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wants {
    /// Wayland: bind on winit's display (data device, serials, drags in).
    pub shared: bool,
    pub data_control: DataControl,
    pub prefer: Prefer,
    /// Wayland: when no Wayland route is left, take X11 selections through Xwayland (where
    /// `DISPLAY` names one). Off by default: only the clipboard has a use for them, so
    /// drag and drop alone opens no X11 connection and stays unsupported there.
    pub x11_fallback: bool,
    /// A test switch: behave as on libwayland < 1.23, whose missing `wl_proxy_get_display`
    /// puts winit's display out of reach, so the fallback runs on any host. Only
    /// strata-platform's own tests set it.
    #[doc(hidden)]
    pub as_if_libwayland_below_1_23: bool,
    /// A test switch: behave as on a compositor with no data control (mutter): its globals
    /// are left unbound on every connection, so a connection of our own has nothing to bind.
    /// Only strata-platform's own tests set it.
    #[doc(hidden)]
    pub as_if_no_data_control: bool,
}

impl Default for Wants {
    fn default() -> Self {
        Wants {
            shared: true,
            data_control: DataControl::default(),
            prefer: Prefer::default(),
            x11_fallback: false,
            as_if_libwayland_below_1_23: false,
            as_if_no_data_control: false,
        }
    }
}

/// What attach bound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// `wayland` or `x11`.
    pub backend: &'static str,
    /// Bound on winit's display.
    pub shared: bool,
    /// A Wayland connection of our own, for data control.
    pub own: bool,
    /// `ext` or `wlr`, where data control is bound.
    pub data_control: Option<&'static str>,
    /// Why winit's display couldn't be used, when it was wanted.
    pub unsupported: Option<Unsupported>,
    /// Wayland with no Wayland route: X11 selections through Xwayland ([`Route::X11`]).
    pub x11: bool,
}

enum Backend {
    Wayland {
        shared: Option<Arc<wayland::Conn>>,
        own: Option<Arc<wayland::Conn>>,
        /// X11 selections through Xwayland, when neither of the others is bound.
        x11: Option<x11::X11>,
    },
    X11(x11::X11),
}

/// Runs at [`Transfer::shutdown`], on the UI thread, before anything is torn down.
#[cfg(any(feature = "dnd", feature = "clipboard", feature = "foreign-parent"))]
pub(crate) type BeforeShutdown = Box<dyn FnOnce(&Transfer) + Send>;

struct Hub {
    /// The thread that attached: the UI thread.
    ui: std::thread::ThreadId,
    status: Status,
    /// Every attached window, with its turn.
    windows: Arc<Windows>,
    subs: Subscribers,
    backend: Backend,
    /// What modules built on the substrate (P1, P2) must do first: tear down their own objects
    /// on winit's display, undo properties they set on the app's windows, hand a selection to
    /// a clipboard manager.
    #[cfg(any(feature = "dnd", feature = "clipboard", feature = "foreign-parent"))]
    before_shutdown: Mutex<Vec<BeforeShutdown>>,
}

/// The process's substrate. Never dropped: [`Transfer::shutdown`] takes it out.
static HUB: Mutex<Option<Arc<Hub>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A handle to the process's substrate.
#[derive(Clone)]
pub struct Transfer(Arc<Hub>);

impl Transfer {
    /// Binds the substrate for the process, or adds `window` and its `turn` to the one
    /// already bound; true when this call bound it. Call it on the UI thread, from
    /// `App::on_window` (which runs again after Rune recreates a window), with a turn built
    /// from `window`'s own app ([`Turn`]).
    pub fn attach(
        window: &WindowHandle,
        turn: Turn,
        wants: Wants,
    ) -> Result<(Transfer, bool), Error> {
        let mut hub = lock(&HUB);
        if let Some(h) = hub.as_ref() {
            h.windows.attach(window.clone(), turn);
            // The reader may be waiting on turns posted only to windows that closed since.
            if let Backend::Wayland {
                shared: Some(c), ..
            } = &h.backend
            {
                c.wake_reader();
            }
            return Ok((Transfer(h.clone()), false));
        }
        let kind = window
            .with_raw_handle(|h| match h.as_raw() {
                RawWindowHandle::Wayland(_) => Ok("wayland"),
                RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_) => Ok("x11"),
                other => Err(format!("{other:?}")),
            })
            .map_err(Error::WindowClosed)?
            .map_err(|k| Error::Unsupported(Unsupported::NotLinux(k)))?;
        let windows = Arc::new(Windows::default());
        windows.attach(window.clone(), turn);
        let subs = Subscribers::default();
        let (backend, status) = match kind {
            "wayland" => wayland_backend(window, &windows, &subs, wants)?,
            _ => (
                Backend::X11(x11::X11::start(subs.clone())?),
                Status {
                    backend: "x11",
                    shared: false,
                    own: false,
                    data_control: None,
                    unsupported: None,
                    x11: false,
                },
            ),
        };
        let h = Arc::new(Hub {
            ui: std::thread::current().id(),
            status,
            windows,
            subs,
            backend,
            #[cfg(any(feature = "dnd", feature = "clipboard", feature = "foreign-parent"))]
            before_shutdown: Mutex::new(Vec::new()),
        });
        *hub = Some(h.clone());
        Ok((Transfer(h), true))
    }

    /// The substrate, if one is bound.
    pub fn current() -> Option<Transfer> {
        lock(&HUB).clone().map(Transfer)
    }

    pub fn status(&self) -> &Status {
        &self.0.status
    }

    /// Hears every event from now on. `notify` runs (on whatever thread produced them) after
    /// events are queued; drain them from the app, on the UI thread.
    pub fn subscribe(&self, notify: impl Fn() + Send + Sync + 'static) -> Subscription {
        self.0.subs.add(Arc::new(notify))
    }

    /// Wayland on winit's display: runs the handlers of events already read, now. Call it on
    /// the UI thread (it refuses any other) before reading a serial, so the key or click that
    /// triggered a data-device write is in. True if there were events.
    pub fn dispatch(&self) -> Result<bool, Error> {
        match &self.0.backend {
            Backend::Wayland {
                shared: Some(c), ..
            } => c.dispatch(),
            _ => Ok(false),
        }
    }

    /// A test switch for strata-platform's own tests: makes the compositor end the Wayland
    /// connection of our own with a protocol error, to check how a dead connection is
    /// reported. False without one.
    #[doc(hidden)]
    pub fn provoke_protocol_error_on_own_connection(&self) -> bool {
        match &self.0.backend {
            Backend::Wayland { own: Some(c), .. } => c.provoke_protocol_error(),
            _ => false,
        }
    }

    /// A test switch for strata-platform's own tests: makes the X server end the X11
    /// connection, to check how the X11 side's failure is reported. False without one.
    #[doc(hidden)]
    pub fn provoke_x11_connection_failure(&self) -> bool {
        match self.selections_x11() {
            Some(x) => x.provoke_connection_failure(),
            None => false,
        }
    }

    /// A test hook for strata-platform's own tests: how many windows the substrate tracks.
    #[doc(hidden)]
    pub fn attached_windows(&self) -> usize {
        self.0.windows.len()
    }

    /// The newest key or button serial on winit's display.
    pub fn latest_serial(&self) -> Option<u32> {
        self.shared().and_then(|c| c.serials().0)
    }

    /// The newest left-button press serial on winit's display.
    pub fn press_serial(&self) -> Option<u32> {
        self.shared().and_then(|c| c.serials().1)
    }

    /// The X11 selections: on X11, or through Xwayland.
    fn selections_x11(&self) -> Option<&x11::X11> {
        match &self.0.backend {
            Backend::X11(x) => Some(x),
            Backend::Wayland { x11, .. } => x11.as_ref(),
        }
    }

    fn shared(&self) -> Option<&Arc<wayland::Conn>> {
        match &self.0.backend {
            Backend::Wayland { shared, .. } => shared.as_ref(),
            Backend::X11(_) => None,
        }
    }

    /// The connection data control is bound on.
    fn control(&self) -> Option<&Arc<wayland::Conn>> {
        match &self.0.backend {
            Backend::Wayland { shared, own, .. } => own
                .as_ref()
                .or(shared.as_ref())
                .filter(|c| c.data_control().is_some()),
            Backend::X11(_) => None,
        }
    }

    /// Offers `spec` as `selection` on `route`. Each writer runs only when a reader asks for
    /// its type, once. Replaces this process's earlier source there, which is cancelled.
    pub fn set_selection(
        &self,
        selection: Selection,
        route: Route,
        spec: SourceSpec,
    ) -> Result<SourceId, Error> {
        match (&self.0.backend, route) {
            (_, Route::X11) => self
                .selections_x11()
                .ok_or(Error::NoRoute("X11"))?
                .set_selection(selection, spec),
            (Backend::Wayland { .. }, Route::DataDevice) => self
                .shared()
                .filter(|c| c.has_device())
                .ok_or(Error::NoRoute("data device"))?
                .set_selection(selection, route, spec),
            (Backend::Wayland { .. }, Route::DataControl) => self
                .control()
                .ok_or(Error::NoRoute("data control"))?
                .set_selection(selection, route, spec),
            (Backend::X11(_), _) => Err(Error::NoRoute("Wayland")),
        }
    }

    /// The current offer on a route, as last heard.
    pub fn selection(&self, selection: Selection, route: Route) -> Option<SelectionOffer> {
        match (&self.0.backend, route) {
            (_, Route::X11) => self.selections_x11()?.selection(selection),
            (Backend::Wayland { .. }, Route::DataDevice) => {
                self.shared()?.selection(selection, route)
            }
            (Backend::Wayland { .. }, Route::DataControl) => {
                self.control()?.selection(selection, route)
            }
            _ => None,
        }
    }

    /// Whether `source` still holds its selection: no one has taken it since.
    pub fn owns(&self, source: SourceId) -> bool {
        match &self.0.backend {
            Backend::X11(x) => x.owns(source),
            Backend::Wayland { shared, own, x11 } => {
                [shared, own].into_iter().flatten().any(|c| c.owns(source))
                    || x11.as_ref().is_some_and(|x| x.owns(source))
            }
        }
    }

    /// Whether `source`'s cancel was the compositor dropping its data-device write (P0 heard
    /// no echo before its `wl_display.sync`), not another client taking the selection.
    pub fn write_was_dropped(&self, source: SourceId) -> bool {
        self.shared().is_some_and(|c| c.write_was_dropped(source))
    }

    /// Gives up the selection `source` holds, if it still holds it: nobody holds that
    /// selection after. Subscribers hear [`Event::SourceCancelled`] for it. False if it held
    /// nothing. On winit's display, call it on the UI thread.
    pub fn clear_selection(&self, source: SourceId) -> Result<bool, Error> {
        match &self.0.backend {
            Backend::X11(x) => x.clear_selection(source),
            Backend::Wayland { shared, own, x11 } => {
                for c in [shared, own].into_iter().flatten() {
                    if c.owns(source) {
                        return c.clear_selection(source);
                    }
                }
                match x11 {
                    Some(x) => x.clear_selection(source),
                    None => Ok(false),
                }
            }
        }
    }

    /// Hands the clipboard this process holds on X11 to a running clipboard manager, every
    /// type of it, so it outlives the process: call it just before [`Transfer::shutdown`].
    /// Blocks the caller until the manager answers, at most about `timeout`. On Wayland
    /// there is no such protocol ([`HandOff::NoManager`]), except through Xwayland
    /// ([`Status::x11`]).
    pub fn hand_off_clipboard(&self, timeout: std::time::Duration) -> HandOff {
        match self.selections_x11() {
            Some(x) => x.hand_off(timeout),
            None => HandOff::NoManager,
        }
    }

    /// Stops and destroys the process's substrate: on the UI thread, before the event loop
    /// ends (see the module docs). True if one was bound. Handles kept elsewhere stay valid
    /// to drop, and their calls fail or do nothing. Refused off the UI thread (the one that
    /// attached), and the substrate stays: its teardown drops winit's display binding, which
    /// must happen there.
    pub fn shutdown() -> Result<bool, Error> {
        let mut slot = lock(&HUB);
        let Some(hub) = slot.as_ref() else {
            return Ok(false);
        };
        if hub.ui != std::thread::current().id() {
            return Err(Error::Refused(
                "shut down on the UI thread, the one that attached",
            ));
        }
        let Some(hub) = slot.take() else {
            return Ok(false);
        };
        drop(slot);
        #[cfg(any(feature = "dnd", feature = "clipboard", feature = "foreign-parent"))]
        {
            let hooks = std::mem::take(&mut *lock(&hub.before_shutdown));
            let t = Transfer(hub.clone());
            for hook in hooks {
                hook(&t);
            }
        }
        match &hub.backend {
            Backend::Wayland { shared, own, x11 } => {
                for c in [shared, own].into_iter().flatten() {
                    c.shutdown();
                }
                if let Some(x) = x11 {
                    x.shutdown();
                }
            }
            Backend::X11(x) => x.shutdown(),
        }
        Ok(true)
    }
}

/// What P1 (`dnd`) and P4 (`foreign-parent`) build on: the connection on winit's display.
#[cfg(any(feature = "dnd", feature = "foreign-parent"))]
impl Transfer {
    /// The Wayland connection on winit's display, if bound.
    pub(crate) fn shared_conn(&self) -> Option<&Arc<wayland::Conn>> {
        self.shared()
    }
}

/// What P4 (`foreign-parent`) builds on: dropping a window as it closes.
#[cfg(feature = "foreign-parent")]
impl Transfer {
    /// Forgets the window keyed `key` and its turn: call it as the window closes, from its
    /// `on_close`. True if it was attached. A window that closes without it is set aside
    /// later, and dropped once a few more pile up (see `windows`).
    pub(crate) fn detach(&self, key: &str) -> bool {
        self.0.windows.detach(key)
    }
}

/// What P1 (`dnd`) builds on: the X11 actor.
#[cfg(feature = "dnd")]
impl Transfer {
    /// The X11 side, on X11.
    pub(crate) fn x11(&self) -> Option<&x11::X11> {
        match &self.0.backend {
            Backend::X11(x) => Some(x),
            Backend::Wayland { .. } => None,
        }
    }
}

/// What P1 (`dnd`), P2 (`clipboard`) and P4 (`foreign-parent`) build on: a hook before the
/// teardown.
#[cfg(any(feature = "dnd", feature = "clipboard", feature = "foreign-parent"))]
impl Transfer {
    /// Runs `f` at [`Transfer::shutdown`], on the UI thread, before the substrate tears down.
    pub(crate) fn before_shutdown(&self, f: BeforeShutdown) {
        lock(&self.0.before_shutdown).push(f);
    }
}

fn wayland_backend(
    window: &WindowHandle,
    windows: &Arc<Windows>,
    subs: &Subscribers,
    wants: Wants,
) -> Result<(Backend, Status), Error> {
    let on_shared = matches!(
        wants.data_control,
        DataControl::Shared | DataControl::SharedElseOwn
    );
    let mut unsupported = None;
    let shared = if wants.shared {
        let control = on_shared
            .then_some(wants.prefer)
            .filter(|_| !wants.as_if_no_data_control);
        match wayland::Conn::shared(
            window,
            windows.clone(),
            subs.clone(),
            control,
            wants.as_if_libwayland_below_1_23,
        ) {
            Ok(c) => Some(c),
            Err(Error::Unsupported(u)) => {
                unsupported = Some(u);
                None
            }
            Err(e) => return Err(e),
        }
    } else {
        None
    };
    let shared_control = shared.as_ref().and_then(|c| c.data_control());
    let want_own = match wants.data_control {
        DataControl::Own => true,
        DataControl::SharedElseOwn => shared_control.is_none(),
        DataControl::Off | DataControl::Shared => false,
    };
    let own = if want_own {
        match wayland::Conn::own(subs.clone(), wants.prefer, wants.as_if_no_data_control) {
            Ok(c) => Some(c),
            // A compositor without data control (mutter). With winit's display out of reach
            // too, the X11 fallback below may still serve.
            Err(Error::Unsupported(u)) => {
                unsupported.get_or_insert(u);
                None
            }
            Err(e) if shared.is_none() => return Err(e),
            Err(_) => None,
        }
    } else {
        None
    };
    if shared.is_none() && own.is_none() {
        let why = unsupported.unwrap_or(Unsupported::NothingWanted);
        let display = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
        if !(wants.x11_fallback && display) || why == Unsupported::NothingWanted {
            return Err(Error::Unsupported(why));
        }
        // No Xwayland to reach is the same answer as no fallback: no route here.
        let Ok(x11) = x11::X11::start(subs.clone()) else {
            return Err(Error::Unsupported(why));
        };
        let status = Status {
            backend: "wayland",
            shared: false,
            own: false,
            data_control: None,
            unsupported: Some(why),
            x11: true,
        };
        return Ok((
            Backend::Wayland {
                shared: None,
                own: None,
                x11: Some(x11),
            },
            status,
        ));
    }
    let status = Status {
        backend: "wayland",
        shared: shared.is_some(),
        own: own.is_some(),
        data_control: own
            .as_ref()
            .and_then(|c| c.data_control())
            .or(shared_control),
        unsupported,
        x11: false,
    };
    Ok((
        Backend::Wayland {
            shared,
            own,
            x11: None,
        },
        status,
    ))
}
