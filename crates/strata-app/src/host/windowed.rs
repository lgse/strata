// SPDX-License-Identifier: MIT

//! Windowed Rune host with the default GLib context pumped on Rune's UI thread.
//!
//! GLib callbacks wake Rune via `AppHandle::update_if` because `Waker` does not run `poll`
//! under a covered Wayland window and `wake_after` throttles to 4 Hz
//! ([anomalyco/rune#139](https://github.com/anomalyco/rune/issues/139)).
//!
//! Refused closes: Rune cannot cancel a WM-initiated close, so `on_close` preserves state by
//! moving `App` into a replacement window when `shell::close_guard` refuses closing.

use std::{
    cell::{OnceCell, RefCell},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use rune::{AppHandle, AppSpec, Tx, WindowHandle, WindowOptions, WindowSpec};
use strata_kit::fonts::{FAMILIES, FONTS};
use strata_platform::{
    backdrop_blur::BackdropBlur,
    csd::{self, Backend, Csd, Policy},
    glib_loop::GlibLoop,
};

use crate::{
    App, application, commands, folder_views, launch,
    settings::Saved,
    shell,
    slots::WeakLater,
    startup::{APPLICATION_ID, UiThread},
};

mod refused;

const SIZE: (f32, f32) = (1200.0, 760.0);

thread_local! {
    /// The interface thread's GLib context in Rune's loop, from before the first frame.
    static GLIB: OnceCell<GlibLoop> = const { OnceCell::new() };
    /// The interface thread, for the windows opened after the first.
    static UI: OnceCell<UiThread> = const { OnceCell::new() };
    static QUEUES: RefCell<Vec<Queue>> = const { RefCell::new(Vec::new()) };
}

/// A window's id, its queue and its handle ([`wake_pending`]).
type Queue = (u64, WeakLater<App>, AppHandle<App>);

/// The open windows' handles by window id, in the order they opened.
static WINDOWS: Mutex<Vec<(u64, AppHandle<App>)>> = Mutex::new(Vec::new());
/// The window with the keyboard, or 0.
static ACTIVE: AtomicU64 = AtomicU64::new(0);

/// `AppSpec` needs a plain function; `App::init` supplies the state, so Rune never calls it.
fn unused() -> App {
    unreachable!("the window's state comes from App::init")
}

pub(super) fn pump(app: &mut App, tx: &mut Tx) -> bool {
    shell::publish(app);
    GLIB.with(|glib| {
        if let Some(glib) = glib.get() {
            glib.pump_until(super::DISPATCH_BUDGET, || app.take_yield_paint());
        }
    });
    let settled = shell::settle(app, tx);
    wake_pending();
    open_asked(app);
    settled
}

pub(super) fn follow_queue(app: &App, handle: AppHandle<App>) {
    let (id, later) = (shell::window_id(app), shell::window_later(app).downgrade());
    QUEUES.with(|queues| {
        let mut queues = queues.borrow_mut();
        queues.retain(|(open, ..)| *open != id);
        queues.push((id, later, handle));
    });
}

pub(crate) fn wake_pending() {
    QUEUES.with(|queues| {
        let Ok(mut queues) = queues.try_borrow_mut() else {
            return;
        };
        queues.retain(|(_, later, _)| later.is_live());
        for (_, _, handle) in queues.iter().filter(|(_, later, _)| later.is_pending()) {
            handle.update_if(shell::settle);
        }
    });
}

fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let pumped = pump(app, tx);
    let polled = shell::poll(app, clock, tx) | pumped;
    super::floating::open_requested(app);
    polled
}

fn notify() {
    let windows = WINDOWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let active = ACTIVE.load(Ordering::Relaxed);
    let handle = windows
        .iter()
        .find(|(id, _)| *id == active)
        .or_else(|| windows.first());
    if let Some((_, handle)) = handle {
        handle.update_if(|app, tx| {
            let pumped = pump(app, tx);
            super::floating::settle_open();
            pumped
        });
    }
}

/// The browser window `id`'s handle, while it is open.
pub(super) fn handle_of(id: u64) -> Option<AppHandle<App>> {
    let windows = WINDOWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let open = windows.iter().find(|(open, _)| *open == id);
    open.map(|(_, handle)| handle.clone())
}

/// Keeps window `id`'s handle, in place of one it had before.
fn opened(id: u64, handle: AppHandle<App>) {
    let mut windows = WINDOWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    windows.retain(|(open, _)| *open != id);
    windows.push((id, handle));
}

pub(crate) fn is_last_window(id: u64) -> bool {
    let windows = WINDOWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    windows.iter().all(|(open, _)| *open == id)
}

/// Forgets the window; true when it was the last.
fn closed(id: u64) -> bool {
    let mut windows = WINDOWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    windows.retain(|(open, _)| *open != id);
    let _ = ACTIVE.compare_exchange(id, 0, Ordering::Relaxed, Ordering::Relaxed);
    windows.is_empty()
}

fn focused(app: &mut App, focused: bool, _: &mut Tx) {
    let id = shell::window_id(app);
    if focused {
        ACTIVE.store(id, Ordering::Relaxed);
    } else {
        let _ = ACTIVE.compare_exchange(id, 0, Ordering::Relaxed, Ordering::Relaxed);
    }
}

/// What the process holds for all its windows goes with the last.
fn shut_down() {
    crate::dnd::shutdown();
    crate::clipboard::shutdown();
    application::release();
}

fn open_asked(app: &App) {
    let asked = application::take_asked();
    if asked.is_empty() {
        return;
    }
    let Some(ui) = UI.with(|ui| ui.get().copied()) else {
        return;
    };
    for application::Asked { opening, token } in asked {
        let settings = shell::share_settings(app);
        let initial = launch::initial_for(opening, &settings);
        let window = window(ui, Start::New(Box::new((settings, initial))));
        // A window whose app's handle has the token is created with it.
        if let Some(token) = token {
            window.handle().activate(token);
        }
        rune::open_window(WindowSpec::from_app(next_key(), window));
    }
}

/// A key no window of the process has had, for `rune::open_window`.
fn next_key() -> String {
    static KEYS: AtomicU64 = AtomicU64::new(2);
    format!("window-{}", KEYS.fetch_add(1, Ordering::Relaxed))
}

enum Start {
    New(Box<(Saved, launch::Initial)>),
    Kept(Box<App>),
}

pub(crate) fn run(ui: UiThread, settings: Saved, initial: launch::Initial) -> Result<(), String> {
    run_initial(ui, settings, initial, |app| app)
}

#[cfg_attr(
    not(feature = "qa"),
    expect(
        dead_code,
        reason = "only the QA mode's session host extends the window"
    )
)]
pub(crate) fn run_with(
    ui: UiThread,
    settings: Saved,
    initial: launch::Initial,
    extend: impl FnOnce(rune::App<App>) -> rune::App<App>,
) -> Result<(), String> {
    run_initial(ui, settings, initial, extend)
}

fn window(ui: UiThread, start: Start) -> rune::App<App> {
    let frame = Policy::Auto.frame();
    let clip =
        strata_platform::clipboard::Clipboard::<App>::new().on_change(crate::clipboard::changed);
    let clip_for_init = clip.clone();
    let dnd = strata_platform::dnd::Dnd::<App>::new();
    let dnd_for_init = dnd.clone();
    let blur = BackdropBlur::new();
    let blur_for_init = blur.clone();
    let app = rune::App::new(AppSpec {
        poll: Some(poll),
        on_key: Some(commands::on_key),
        fonts: FONTS,
        families: FAMILIES,
        ..AppSpec::new("Strata", SIZE, unused, shell::view)
    })
    .compositor(crate::fx::FxCompositor::new())
    .on_close(|app: &mut App, _: &mut Tx| {
        if refused::keep_open(app) {
            return;
        }
        shell::detach_backdrop_blur(app);
        super::floating::close_with_browser(shell::window_id(app));
        folder_views::flush_at_exit(app);
        if closed(shell::window_id(app)) {
            shut_down();
        }
    })
    .on_quit(|app: &mut App, _: &mut Tx| {
        shell::detach_backdrop_blur(app);
        folder_views::flush_at_exit(app);
        if closed(shell::window_id(app)) {
            shut_down();
        }
    })
    .on_modifiers(shell::modifiers)
    .on_focus(focused)
    .on_visibility(shell::window_shown)
    // Rune's own flags (`--snap`, `--tree`) are not this binary's.
    .args(Vec::new())
    .window_options(csd::window_options(
        frame,
        WindowOptions {
            transparent: true,
            ..WindowOptions::default()
        },
    ));
    let handle = app.handle();
    let registered = handle.clone();
    app.init(move || {
        let (mut app, initial) = match start {
            Start::New(new) => {
                let (settings, initial) = *new;
                let csd = Csd::new(frame, Backend::from_env());
                (App::new(&ui, csd, settings), Some(initial))
            }
            Start::Kept(app) => {
                let mut app = *app;
                shell::reopened(&mut app);
                (app, None)
            }
        };
        crate::clipboard::bind(&mut app, clip_for_init);
        crate::dnd::bind(&mut app, dnd_for_init);
        shell::bind_backdrop_blur(&mut app, blur_for_init);
        follow_queue(&app, registered.clone());
        opened(shell::window_id(&app), registered);
        if let Some(initial) = initial {
            app.start(initial);
        }
        app
    })
    .on_window(move |window: &WindowHandle| {
        crate::clipboard::attach(&clip, window, &handle);
        crate::dnd::attach(&dnd, window, &handle);
        csd::attach(window, &handle, shell::csd_of);
        blur.attach(window, &handle);
    })
}

fn run_initial(
    ui: UiThread,
    settings: Saved,
    initial: launch::Initial,
    extend: impl FnOnce(rune::App<App>) -> rune::App<App>,
) -> Result<(), String> {
    rune::set_app_id(APPLICATION_ID);
    UI.with(|cell| cell.set(ui))
        .map_err(|_| "the windows are already running on this thread".to_owned())?;
    super::floating::bind(ui);
    let app = extend(window(ui, Start::New(Box::new((settings, initial)))));
    let glib = GlibLoop::start(glib::MainContext::default(), notify)
        .map_err(|error| format!("unable to start the GLib loop: {error}"))?;
    GLIB.with(|cell| cell.set(glib))
        .map_err(|_| "the GLib loop is already running on this thread".to_owned())?;
    rune::fast_exit(strata_core::storage::writer::flush_at_exit);
    let ran = app.try_run();
    strata_core::storage::writer::flush_at_exit();
    ran
}
