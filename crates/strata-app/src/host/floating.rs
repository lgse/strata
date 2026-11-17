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
//! One browser window per process: `strata-rune` opens one, and New Window does not open a
//! second in process yet. The one `BROWSER` and `OPEN` below assume it; whoever opens a second
//! browser window in process keeps them per window.

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

thread_local! {
    /// The browser window's thread and app, from before its first frame.
    static BROWSER: OnceCell<(UiThread, AppHandle<App>)> = const { OnceCell::new() };
    /// The open chooser's window and app: the window to bring forward or close, the app for a
    /// GLib wake-up to settle too.
    static OPEN: RefCell<Option<(WindowHandle, AppHandle<App>)>> = const { RefCell::new(None) };
}

/// Lets the browser window open floating choosers.
pub(crate) fn bind(ui: UiThread, browser: AppHandle<App>) {
    BROWSER.with(|cell| {
        let _ = cell.set((ui, browser));
    });
}

/// Opens the floating chooser the browser asked for, if it asked, or brings the open one
/// forward when the browser asked again.
pub(crate) fn open_requested(app: &mut App) {
    if crate::chooser::floating::take_raise(app) {
        raise_open();
    }
    let Some(request) = crate::chooser::floating::take(app) else {
        return;
    };
    match BROWSER.with(|cell| cell.get().cloned()) {
        Some((ui, browser)) => open(ui, browser, request),
        None => (request.completion)(Err(crate::portal::PortalError::Cancelled(
            "no window to open the chooser over".into(),
        ))),
    }
}

/// Applies what the open chooser's services queued, as the browser's GLib wake-up does for it.
pub(super) fn settle_open() {
    OPEN.with(|open| {
        if let Some((_, handle)) = open.borrow().as_ref() {
            handle.update_if(shell::settle);
        }
    });
}

/// Brings the open chooser forward and gives it the keyboard.
fn raise_open() {
    OPEN.with(|open| {
        if let Some((window, _)) = open.borrow().as_ref() {
            tracing::debug!("the open floating chooser was asked for again: brought forward");
            window.show();
            window.focus();
        }
    });
}

/// Closes the open chooser, whose close cancels it: the browser it answers is closing.
pub(super) fn close_with_browser() {
    let open = OPEN.with(|open| open.borrow_mut().take());
    if let Some((window, _)) = open {
        tracing::debug!("the browser window closed: its floating chooser closes with it");
        window.close();
    }
}

fn chooser_poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    super::windowed::pump(app, tx) | shell::poll(app, clock, tx)
}

fn close_chooser(app: &mut App, tx: &mut Tx) {
    crate::chooser::cancel(app, tx);
    folder_views::flush_at_exit(app);
    OPEN.with(|open| open.borrow_mut().take());
}

/// The chooser's answer, then a wake-up for the browser, whose inbox holds it.
fn waking(completion: Completion, browser: AppHandle<App>) -> Completion {
    Box::new(move |result| {
        completion(result);
        browser.update_if(shell::settle);
    })
}

fn open(ui: UiThread, browser: AppHandle<App>, request: FloatingRequest) {
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
    OPEN.with(|open| open.replace(Some((window, handle))));
}
