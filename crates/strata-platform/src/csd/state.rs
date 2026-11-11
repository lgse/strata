//! [`Csd`]: what the app keeps in its state, and where each window button's request goes.

use rune::Tx;

use super::x11::Link;
use super::{Backend, Bands, Edge, Frame, Gap, Outcome, Request, Route};

/// The window's state, where it is known. `None` is unknown: Rune reports neither on Wayland
/// (draft U6, not filed), and X11 knows once the window is linked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowState {
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
}

/// Whether the app's own control for each action can work. Hide a control only on
/// `Some(false)`; `None` is unknown (the request goes out, and the window manager may ignore
/// it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Resizing from the bands.
    pub resize: Option<bool>,
    pub minimize: Option<bool>,
    pub maximize: Option<bool>,
    /// The window menu from the header.
    pub window_menu: Option<bool>,
}

/// What the X11 watcher reads: the window's `_NET_WM_STATE`, and its window manager's
/// `_NET_SUPPORTED` and `_NET_WM_ALLOWED_ACTIONS`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The X window it was read from. Once a [`Csd`] is linked to an X11 window, it takes only
    /// that window's reports, so a late one from a window since destroyed is dropped.
    pub window: Option<u32>,
    pub state: WindowState,
    pub capabilities: Capabilities,
}

/// What an observer ([`Csd::observe`]) hears.
#[derive(Debug)]
pub enum Notice<'a> {
    /// [`attach`](super::attach) linked a native window.
    Attached,
    /// A new [`Report`] from the X11 watcher.
    Report,
    /// A request's outcome.
    Outcome(&'a Outcome),
    /// Something went wrong outside a request: the X11 connection or its first read failed.
    Error(&'a str),
}

/// What [`attach`](super::attach) found.
pub(super) enum Native {
    Wayland,
    X11 {
        window: u32,
        link: Result<Box<Link>, String>,
    },
    /// No native window this module knows.
    Other,
}

/// The X11 connection, for an X11 window.
#[derive(Debug)]
enum X11 {
    /// Not attached yet.
    None,
    Linked(Box<Link>),
    Failed(String),
}

/// Client-side decorations for one window: its frame, its backend and what is known about it.
/// The app keeps one in its state per window and passes an accessor to
/// [`decorate`](super::decorate) and [`header`](super::header).
pub struct Csd {
    frame: Frame,
    backend: Backend,
    bands: Bands,
    report: Report,
    /// The X window this is linked to, whether or not its connection worked.
    window: Option<u32>,
    x11: X11,
    last: Option<Outcome>,
    revision: u64,
    observer: Option<fn(&Csd, Notice<'_>)>,
}

impl std::fmt::Debug for Csd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Csd")
            .field("frame", &self.frame)
            .field("backend", &self.backend)
            .field("report", &self.report)
            .field("window", &self.window)
            .field("x11", &self.x11)
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

impl Csd {
    /// A window that opens with `frame` on `backend` ([`Backend::from_env`] before the window
    /// exists; [`attach`](super::attach) confirms it). A headless run, a test or a gallery
    /// scene passes [`Backend::Headless`].
    pub fn new(frame: Frame, backend: Backend) -> Csd {
        Csd {
            frame,
            backend,
            bands: Bands::default(),
            report: Report {
                capabilities: defaults(frame, backend),
                ..Report::default()
            },
            window: None,
            x11: X11::None,
            last: None,
            revision: 0,
            observer: None,
        }
    }

    /// Bands of another width.
    pub fn with_bands(mut self, bands: Bands) -> Csd {
        self.bands = bands;
        self
    }

    /// Calls `f` on each [`Notice`], after it took effect: for logging, or to follow outcomes.
    pub fn observe(mut self, f: fn(&Csd, Notice<'_>)) -> Csd {
        self.observer = Some(f);
        self
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn bands(&self) -> Bands {
        self.bands
    }

    pub fn state(&self) -> WindowState {
        self.report.state
    }

    pub fn capabilities(&self) -> Capabilities {
        self.report.capabilities
    }

    /// Why the X11 connection failed, if it did.
    pub fn connection_error(&self) -> Option<&str> {
        match &self.x11 {
            X11::Failed(e) => Some(e),
            X11::None | X11::Linked(_) => None,
        }
    }

    /// The last request's outcome.
    pub fn last_outcome(&self) -> Option<&Outcome> {
        self.last.as_ref()
    }

    /// Goes up by one each time the backend, state or capabilities change.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether [`decorate`](super::decorate) mounts the resize bands: a borderless window
    /// whose bands can resize (not on Wayland, U6, nor without an X11 connection), that isn't
    /// maximised or full screen.
    pub fn shows_bands(&self) -> bool {
        let state = self.report.state;
        self.frame == Frame::Borderless
            && self.report.capabilities.resize != Some(false)
            && state.maximized != Some(true)
            && state.fullscreen != Some(true)
    }

    /// Whether [`header`](super::header) opens the window menu on a secondary click: only
    /// where the window manager says it has one.
    pub fn shows_window_menu(&self) -> bool {
        self.frame == Frame::Borderless && self.report.capabilities.window_menu == Some(true)
    }

    /// Takes the X11 watcher's report. Apps that start their own watcher call this; [`attach`]
    /// wires it for them. Once linked to an X11 window, a report from another window (one since
    /// destroyed, or `window: None`) is dropped.
    ///
    /// [`attach`]: super::attach
    pub fn apply(&mut self, mut report: Report) {
        if self.window.is_some() && report.window != self.window {
            return;
        }
        if self.frame == Frame::Native {
            // The frame resizes and has the window menu; the app's own controls for them don't.
            report.capabilities.resize = Some(false);
            report.capabilities.window_menu = Some(false);
        }
        if report != self.report {
            self.report = report;
            self.revision += 1;
            self.notify(Notice::Report);
        }
    }

    /// Links the native window [`attach`](super::attach) found, in place of any before it.
    /// Dropping an old X11 link stops its watcher.
    pub(super) fn attached(&mut self, native: Native) {
        self.report = Report::default();
        let mut error = None;
        match native {
            Native::Wayland => self.unlinked(Backend::Wayland),
            Native::Other => self.unlinked(Backend::Headless),
            Native::X11 { window, link } => {
                self.backend = Backend::X11;
                self.window = Some(window);
                self.report.capabilities = defaults(self.frame, Backend::X11);
                match link {
                    Ok(link) => {
                        match link.read() {
                            Ok(report) => self.report = report,
                            Err(e) => error = Some(format!("the window's first read failed: {e}")),
                        }
                        self.x11 = X11::Linked(link);
                    }
                    Err(e) => {
                        if self.frame == Frame::Borderless {
                            self.report.capabilities = Capabilities {
                                resize: Some(false),
                                minimize: Some(false),
                                maximize: None,
                                window_menu: Some(false),
                            };
                        }
                        error = Some(format!("no X11 connection: {e}"));
                        self.x11 = X11::Failed(e);
                    }
                }
                self.report.window = Some(window);
                if self.frame == Frame::Native {
                    self.report.capabilities.resize = Some(false);
                    self.report.capabilities.window_menu = Some(false);
                }
            }
        }
        self.revision += 1;
        self.notify(Notice::Attached);
        if let Some(e) = error {
            self.notify(Notice::Error(&e));
        }
    }

    /// A window with no X11 link.
    fn unlinked(&mut self, backend: Backend) {
        self.backend = backend;
        self.window = None;
        self.x11 = X11::None;
        self.report.capabilities = defaults(self.frame, backend);
    }

    /// The Close button.
    pub fn close(&mut self, tx: &mut Tx) -> Outcome {
        tx.close_window();
        self.done(sent(Request::Close, Route::Rune))
    }

    /// The Maximise or Restore button. Rune maximises or restores a borderless window too.
    pub fn toggle_maximized(&mut self, tx: &mut Tx) -> Outcome {
        tx.toggle_maximized();
        self.done(sent(Request::ToggleMaximize, Route::Rune))
    }

    /// The Minimise button. Rune minimises a window with a native frame and ignores a
    /// borderless one (U7), so on X11 this module asks the window manager itself.
    pub fn minimize(&mut self, tx: &mut Tx) -> Outcome {
        let request = Request::Minimize;
        let can = self.report.capabilities.minimize;
        let outcome = match (self.frame, self.backend) {
            (Frame::Native, _) => {
                tx.minimize();
                sent(request, Route::Rune)
            }
            (Frame::Borderless, Backend::Wayland) => unsupported(request, Gap::RuneMinimize),
            // Unknown goes out: WM_CHANGE_STATE is ICCCM, which any window manager may honour.
            (Frame::Borderless, Backend::X11) => {
                self.x11(request, can, None, |link| link.minimize())
            }
            (Frame::Borderless, Backend::Headless) => sent(request, Route::Headless),
        };
        self.done(outcome)
    }

    /// Starts an interactive resize from `edge`. Call it while the button is still down (a
    /// band's `on_drag` `Phase::Down`). `screen` is the pointer in desktop pixels
    /// (`Drag::screen_point`), else the X server is asked.
    pub fn begin_resize(&mut self, edge: Edge, screen: Option<(i32, i32)>) -> Outcome {
        let request = Request::Resize(edge);
        let can = self.report.capabilities.resize;
        let outcome = match (self.frame, self.backend) {
            (Frame::Native, _) => unsupported(request, Gap::NativeFrame),
            (Frame::Borderless, Backend::Wayland) => unsupported(request, Gap::RuneResize),
            (Frame::Borderless, Backend::X11) => {
                self.x11(request, can, None, |link| link.begin_resize(edge, screen))
            }
            (Frame::Borderless, Backend::Headless) => sent(request, Route::Headless),
        };
        self.done(outcome)
    }

    /// Opens the window manager's window menu at the pointer.
    pub fn window_menu(&mut self) -> Outcome {
        let request = Request::WindowMenu;
        let can = self.report.capabilities.window_menu;
        let outcome = match (self.frame, self.backend) {
            (Frame::Native, _) => unsupported(request, Gap::NativeFrame),
            (Frame::Borderless, Backend::Wayland) => unsupported(request, Gap::RuneWindowMenu),
            // `_GTK_SHOW_WINDOW_MENU` means nothing to a window manager that doesn't list it.
            (Frame::Borderless, Backend::X11) => {
                self.x11(request, can, Some(Gap::NotKnown), |link| link.window_menu())
            }
            (Frame::Borderless, Backend::Headless) => sent(request, Route::Headless),
        };
        self.done(outcome)
    }

    /// Sends an X11 request where the link and the window manager allow it ([`gate`]).
    fn x11(
        &self,
        request: Request,
        can: Option<bool>,
        unknown: Option<Gap>,
        send: impl FnOnce(&Link) -> Result<(), String>,
    ) -> Outcome {
        let link = match &self.x11 {
            X11::None => Linking::None,
            X11::Failed(_) => Linking::Failed,
            X11::Linked(_) => Linking::Linked,
        };
        if let Some(gap) = gate(link, can, unknown) {
            return unsupported(request, gap);
        }
        let X11::Linked(link) = &self.x11 else {
            return unsupported(request, Gap::NoWindow);
        };
        match send(link) {
            Ok(()) => sent(request, Route::X11),
            Err(error) => Outcome::Failed { request, error },
        }
    }

    fn done(&mut self, outcome: Outcome) -> Outcome {
        self.last = Some(outcome.clone());
        self.notify(Notice::Outcome(&outcome));
        outcome
    }

    fn notify(&self, notice: Notice<'_>) {
        if let Some(f) = self.observer {
            f(self, notice);
        }
    }
}

/// Where an X11 link stands, for [`gate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Linking {
    None,
    Linked,
    Failed,
}

/// Why an X11 request can't go out, if it can't: no window or connection first, then what the
/// window manager says. `can` is its capability, and `unknown` the gap to answer while it is
/// unknown (`None` sends anyway).
pub(super) fn gate(link: Linking, can: Option<bool>, unknown: Option<Gap>) -> Option<Gap> {
    match (link, can) {
        (Linking::None, _) => Some(Gap::NoWindow),
        (Linking::Failed, _) => Some(Gap::NoConnection),
        (Linking::Linked, Some(false)) => Some(Gap::WindowManager),
        (Linking::Linked, None) => unknown,
        (Linking::Linked, Some(true)) => None,
    }
}

/// What can work before anything is read from the window manager.
fn defaults(frame: Frame, backend: Backend) -> Capabilities {
    match (frame, backend) {
        // The native frame resizes and has its own menu; the app's buttons go through Rune.
        (Frame::Native, _) => Capabilities {
            resize: Some(false),
            window_menu: Some(false),
            ..Capabilities::default()
        },
        // U6, U7, U8.
        (Frame::Borderless, Backend::Wayland) => Capabilities {
            resize: Some(false),
            minimize: Some(false),
            maximize: None,
            window_menu: Some(false),
        },
        // The link's first read fills these in.
        (Frame::Borderless, Backend::X11 | Backend::Headless) => Capabilities::default(),
    }
}

fn sent(request: Request, route: Route) -> Outcome {
    Outcome::Sent { request, route }
}

fn unsupported(request: Request, gap: Gap) -> Outcome {
    Outcome::Unsupported { request, gap }
}
