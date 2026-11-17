// SPDX-License-Identifier: MIT

//! The browser window's floating destination chooser on the windowed host
//! (`crate::chooser::floating`): a Rune window of its own over a chooser `App`, opened when
//! the browser asks for one and closed once it answers.
//!
//! The chooser answers through the browser's inbox, which only the browser's own loop reads:
//! the answer therefore also wakes the browser ([`AppHandle::update_if`]).

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
    /// The open chooser's app, which a GLib wake-up settles too.
    static OPEN: RefCell<Option<AppHandle<App>>> = const { RefCell::new(None) };
}

/// Lets the browser window open floating choosers.
pub(crate) fn bind(ui: UiThread, browser: AppHandle<App>) {
    BROWSER.with(|cell| {
        let _ = cell.set((ui, browser));
    });
}

/// Opens the floating chooser the browser asked for, if it asked.
pub(crate) fn open_requested(app: &mut App) {
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
        if let Some(handle) = open.borrow().as_ref() {
            handle.update_if(shell::settle);
        }
    });
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
    let (width, height) = chooser_initial_dimensions(None, None);
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
    rune::open_window(WindowSpec::from_app(&token, app).state(state));
    OPEN.with(|open| open.replace(Some(handle)));
}
