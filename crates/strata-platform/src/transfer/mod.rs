// SPDX-License-Identifier: MIT

//! Internal transfer substrate shared by `dnd`, `clipboard`, and `foreign-parent`.
//!
//! Uses `WindowHandle::with_raw_display_handle` to share winit's Wayland display; Rune still
//! lacks an every-turn dispatch hook for a custom Wayland queue ([anomalyco/rune#522]).
//!
//! Invariants:
//! - On winit's Wayland display, every request and dispatch must run on the UI thread (the
//!   reader thread never dispatches).
//! - Call [`Transfer::shutdown`] on the UI thread before the event loop tears down the display
//!   (from `App::on_display_close`, `App::on_quit`, or the last window's `on_close`) so proxies
//!   bound on winit's display are destroyed while the display is still alive; dropping them after
//!   winit frees the display segfaults.

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

pub type Turn = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

/// Where data control goes on Wayland.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DataControl {
    /// No data control.
    Off,
    /// On winit's display only.
    Shared,
    /// On a connection of our own, which needs no shared display.
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
    pub x11_fallback: bool,
    #[doc(hidden)]
    pub as_if_no_shared_display: bool,
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
            as_if_no_shared_display: false,
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

    pub fn subscribe(&self, notify: impl Fn() + Send + Sync + 'static) -> Subscription {
        self.0.subs.add(Arc::new(notify))
    }

    pub fn dispatch(&self) -> Result<bool, Error> {
        match &self.0.backend {
            Backend::Wayland {
                shared: Some(c), ..
            } => c.dispatch(),
            _ => Ok(false),
        }
    }

    #[doc(hidden)]
    pub fn provoke_protocol_error_on_own_connection(&self) -> bool {
        match &self.0.backend {
            Backend::Wayland { own: Some(c), .. } => c.provoke_protocol_error(),
            _ => false,
        }
    }

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

    pub fn write_was_dropped(&self, source: SourceId) -> bool {
        self.shared().is_some_and(|c| c.write_was_dropped(source))
    }

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

    pub fn hand_off_clipboard(&self, timeout: std::time::Duration) -> HandOff {
        match self.selections_x11() {
            Some(x) => x.hand_off(timeout),
            None => HandOff::NoManager,
        }
    }

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

#[cfg(any(feature = "dnd", feature = "foreign-parent"))]
impl Transfer {
    /// The Wayland connection on winit's display, if bound.
    pub(crate) fn shared_conn(&self) -> Option<&Arc<wayland::Conn>> {
        self.shared()
    }
}

#[cfg(feature = "foreign-parent")]
impl Transfer {
    pub(crate) fn detach(&self, key: &str) -> bool {
        self.0.windows.detach(key)
    }
}

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
            wants.as_if_no_shared_display,
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
