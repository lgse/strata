// SPDX-License-Identifier: MIT

use std::cell::{OnceCell, RefCell};

use rune::{AppHandle, Tx, WindowHandle, WindowSpec};
use strata_core::model::Location;

use super::chooser_window::{self, ChooserWindow, chooser_app};
use crate::{
    App,
    chooser::{floating::FloatingRequest, geometry::chooser_initial_dimensions},
    folder_views,
    portal::Completion,
    settings::Saved,
    shell,
    startup::UiThread,
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
        Some((ui, browser)) => open(ui, (id, browser), shell::share_settings(app), request),
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

fn open(
    ui: UiThread,
    (id, browser): (u64, AppHandle<App>),
    settings: Saved,
    request: FloatingRequest,
) {
    let FloatingRequest {
        request,
        source,
        destination,
        completion,
    } = request;
    let (width, height) = chooser_initial_dimensions(None, request.parent_size_hint);
    #[expect(clippy::cast_precision_loss, reason = "window dimensions fit f32")]
    let size = (width as f32, height as f32);
    let token = request.token.clone();
    let initial = Location::local(request.initial_directory.clone());
    let mut state = App::new_chooser(
        &ui,
        settings,
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
        visible: true,
        poll: chooser_poll,
        on_close: close_chooser,
    });
    let handle = app.handle();
    let wake = handle.clone();
    state
        .window
        .set_image_cache(std::sync::Arc::new(rune::ImageCache::new(
            std::sync::Arc::new(move || wake.wake()),
        )));
    super::windowed::follow_queue(&state, handle.clone());
    let attach = handle.clone();
    let app = app.on_window(move |window: &WindowHandle| {
        chooser_window::attach_window(window, &attach);
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
