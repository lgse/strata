// SPDX-License-Identifier: MIT

//! The browser window's floating destination chooser on the windowed host
//! (`crate::chooser::floating`): a Rune window of its own over a chooser `App`, opened when
//! the browser asks for one and closed once it answers.
//!
//! The chooser answers through the browser's inbox, which only the browser's own loop reads:
//! the answer therefore also wakes the browser ([`AppHandle::update_if`]).
//!
//! Rune makes no window transient for another of the app's own (Rune packet entry 23), so
//! the host does by hand what GTK's transient chooser gets from its parent: a second Copy to,
//! Move to or Extract to brings the open chooser forward (`DestinationLease::acquire`'s
//! `present`), and closing the browser closes its chooser (`bind_host_parent`'s
//! `destroy_with_parent`), which cancels it.
//!
//! A process has a browser window for each launch ([`super::windowed`], "Windows"), so the
//! choosers open are kept by the browser window that asked for each.

use std::cell::{OnceCell, RefCell};

use rune::{AppHandle, Tx, WindowHandle, WindowSpec};
use strata_core::model::Location;
use strata_platform::csd::{self, Backend, Csd, Policy};

use super::chooser_window::{ChooserWindow, chooser_app};
use crate::{
    App,
    chooser::{floating::FloatingRequest, geometry::chooser_initial_dimensions},
    folder_views,
    portal::Completion,
    shell,
    startup::{self, UiThread},
};

/// A chooser open over a browser window.
struct Open {
    /// The browser window that asked for it, and the chooser's own (`shell::window_id`).
    browser: u64,
    chooser: u64,
    /// The window to bring forward or close, and the app for a GLib wake-up to settle too.
    window: WindowHandle,
    handle: AppHandle<App>,
}

thread_local! {
    /// The interface thread, from before the first browser window's first frame.
    static UI: OnceCell<UiThread> = const { OnceCell::new() };
    /// The choosers open, one at most for each browser window.
    static OPEN: RefCell<Vec<Open>> = const { RefCell::new(Vec::new()) };
}

/// Lets the browser windows open floating choosers.
pub(crate) fn bind(ui: UiThread) {
    UI.with(|cell| {
        let _ = cell.set(ui);
    });
}

/// Opens the floating chooser the browser asked for, if it asked, or brings the open one
/// forward when the browser asked again.
pub(crate) fn open_requested(app: &mut App) {
    let id = shell::window_id(app);
    if crate::chooser::floating::take_raise(app) {
        raise_open(id);
    }
    let Some(request) = crate::chooser::floating::take(app) else {
        return;
    };
    let ui = UI.with(|cell| cell.get().copied());
    match ui.zip(super::windowed::handle_of(id)) {
        Some((ui, browser)) => open(ui, (id, browser), request),
        None => (request.completion)(Err(crate::portal::PortalError::Cancelled(
            "no window to open the chooser over".into(),
        ))),
    }
}

/// Applies what the open choosers' services queued, as the browser's GLib wake-up does for it.
pub(super) fn settle_open() {
    OPEN.with(|open| {
        for open in open.borrow().iter() {
            open.handle.update_if(shell::settle);
        }
    });
}

/// Brings the chooser open over browser window `browser` forward and gives it the keyboard.
fn raise_open(browser: u64) {
    OPEN.with(|open| {
        for open in open.borrow().iter().filter(|open| open.browser == browser) {
            tracing::debug!("the open floating chooser was asked for again: brought forward");
            open.window.show();
            open.window.focus();
        }
    });
}

/// Closes the chooser open over browser window `browser`, whose close cancels it: the browser
/// it answers is closing.
pub(super) fn close_with_browser(browser: u64) {
    let closing: Vec<Open> = OPEN.with(|open| {
        let (closing, kept) = open
            .borrow_mut()
            .drain(..)
            .partition(|open| open.browser == browser);
        *open.borrow_mut() = kept;
        closing
    });
    for open in closing {
        tracing::debug!("the browser window closed: its floating chooser closes with it");
        open.window.close();
    }
}

fn chooser_poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    super::windowed::pump(app, tx) | shell::poll(app, clock, tx)
}

fn close_chooser(app: &mut App, tx: &mut Tx) {
    crate::chooser::cancel(app, tx);
    folder_views::flush_at_exit(app);
    let chooser = shell::window_id(app);
    OPEN.with(|open| open.borrow_mut().retain(|open| open.chooser != chooser));
}

/// The chooser's answer, then a wake-up for the browser, whose inbox holds it.
fn waking(completion: Completion, browser: AppHandle<App>) -> Completion {
    Box::new(move |result| {
        completion(result);
        browser.update_if(shell::settle);
    })
}

fn open(ui: UiThread, (id, browser): (u64, AppHandle<App>), request: FloatingRequest) {
    let FloatingRequest {
        request,
        source,
        destination,
        completion,
    } = request;
    let frame = Policy::Auto.frame();
    let (width, height) = chooser_initial_dimensions(None, request.parent_size_hint);
    #[expect(clippy::cast_precision_loss, reason = "window dimensions fit f32")]
    let size = (width as f32, height as f32);
    let token = request.token.clone();
    let initial = Location::local(request.initial_directory.clone());
    let state = App::new_chooser(
        &ui,
        Csd::new(frame, Backend::from_env()),
        startup::load_settings(),
        (
            request,
            source,
            Some(destination),
            waking(completion, browser),
        ),
    );
    state.open(initial);
    let chooser = shell::window_id(&state);
    let app = chooser_app(ChooserWindow {
        size,
        frame,
        visible: true,
        poll: chooser_poll,
        on_close: close_chooser,
    });
    let handle = app.handle();
    let attach = handle.clone();
    let app = app.on_window(move |window: &WindowHandle| {
        csd::attach(window, &attach, shell::csd_of);
    });
    let window = rune::open_window(WindowSpec::from_app(&token, app).state(state));
    tracing::debug!("a floating chooser opened");
    OPEN.with(|open| {
        open.borrow_mut().push(Open {
            browser: id,
            chooser,
            window,
            handle,
        });
    });
}
