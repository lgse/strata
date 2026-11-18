// SPDX-License-Identifier: MIT

//! The host in windows. Rune runs winit's loop, which never looks at GLib, so the default
//! GLib context is embedded in it: strata-platform's `glib_loop` waits for what the context
//! waits for on a helper thread, and its dispatch runs here, on Rune's loop.
//!
//! The helper reaches the loop through `AppHandle::update_if`. A `Waker` call never runs
//! `poll` while a Wayland window is covered, and a `wake_after` timer runs four times a
//! second then (Rune #139; `docs/rune/slice/V5-shell-and-loop.md` has the measurement).
//!
//! # Windows
//!
//! The process opens a window for each launch's argument (`crate::application`): the first
//! is the one `App::run` opens, and each later one is a `rune::open_window` of its own, with
//! its own state and the process's settings shared ([`Saved::share`]). The app ends when the
//! last of them closes, as Rune ends it.
//!
//! The GLib context is the process's, so the helper pumps it through one window: the one with
//! the keyboard, or else the first still open. A pump hands that window's view changes to its
//! browser before it dispatches ([`shell::publish`]); another window's go at its own next
//! `poll`, which pumps too. What a dispatch queued for another window's browser waits for that
//! window's `poll`.

use std::{
    cell::OnceCell,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use rune::{AppHandle, AppSpec, Tx, WindowHandle, WindowOptions, WindowSpec};
use strata_core::model::Location;
use strata_kit::fonts::{FAMILIES, FONTS};
use strata_platform::{
    csd::{self, Backend, Csd, Policy},
    glib_loop::GlibLoop,
};

use crate::{
    App, application, commands, folder_views, launch,
    settings::Saved,
    shell,
    startup::{APPLICATION_ID, UiThread},
};

/// Strata's default window size (`src/ui/window`).
const SIZE: (f32, f32) = (1200.0, 760.0);

thread_local! {
    /// The interface thread's GLib context in Rune's loop, from before the first frame.
    static GLIB: OnceCell<GlibLoop> = const { OnceCell::new() };
    /// The interface thread, for the windows opened after the first.
    static UI: OnceCell<UiThread> = const { OnceCell::new() };
}

/// The open windows' handles by window id, in the order they opened.
static WINDOWS: Mutex<Vec<(u64, AppHandle<App>)>> = Mutex::new(Vec::new());
/// The window with the keyboard, or 0.
static ACTIVE: AtomicU64 = AtomicU64::new(0);

/// `AppSpec` needs a plain function; `App::init` supplies the state, so Rune never calls it.
fn unused() -> App {
    unreachable!("the window's state comes from App::init")
}

/// Dispatches what GLib has ready, then applies what the browser's observers queued, and opens
/// the windows the dispatch asked for. Runs when the helper thread says GLib has work, and in
/// `poll`: a handler may have attached a source since, which the helper cannot know of.
pub(super) fn pump(app: &mut App, tx: &mut Tx) -> bool {
    shell::publish(app);
    GLIB.with(|glib| {
        if let Some(glib) = glib.get() {
            glib.pump(super::DISPATCH_BUDGET);
        }
    });
    let settled = shell::settle(app, tx);
    open_asked(app);
    settled
}

fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let pumped = pump(app, tx);
    let polled = shell::poll(app, clock, tx) | pumped;
    super::floating::open_requested(app);
    polled
}

/// The helper's way into the loop: the active window's, else the first open one's, and the
/// floating choosers', whose services the dispatch may have answered.
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

fn opened(id: u64, handle: AppHandle<App>) {
    let mut windows = WINDOWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    windows.push((id, handle));
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

/// Opens a window for each launch the bus handed over since the last pump.
fn open_asked(app: &App) {
    let asked = application::take();
    if asked.is_empty() {
        return;
    }
    let Some(ui) = UI.with(|ui| ui.get().copied()) else {
        return;
    };
    for opening in asked {
        let settings = shell::share_settings(app);
        let initial = launch::initial_for(opening, &settings);
        static KEYS: AtomicU64 = AtomicU64::new(2);
        let key = format!("window-{}", KEYS.fetch_add(1, Ordering::Relaxed));
        rune::open_window(WindowSpec::from_app(key, window(ui, settings, initial)));
    }
}

/// Opens the window on `initial` and exits when the last window closes, returning an error if
/// startup fails.
pub(crate) fn run(ui: UiThread, settings: Saved, initial: launch::Initial) -> Result<(), String> {
    run_initial(ui, settings, initial, |app| app)
}

/// Opens the window on `location` with the app handed to `extend` before it runs: the QA mode's
/// session host adds its frame hook and takes a handle there (`qa::session`).
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
    location: Location,
    extend: impl FnOnce(rune::App<App>) -> rune::App<App>,
) -> Result<(), String> {
    run_initial(ui, settings, launch::Initial::Explicit(location), extend)
}

/// One window's app on `initial`: its state, its hooks, and the clipboard, drag and drop and
/// frame that attach to each native window it gets.
fn window(ui: UiThread, settings: Saved, initial: launch::Initial) -> rune::App<App> {
    let frame = Policy::Auto.frame();
    let clip =
        strata_platform::clipboard::Clipboard::<App>::new().on_change(crate::clipboard::changed);
    let clip_for_init = clip.clone();
    let dnd = strata_platform::dnd::Dnd::<App>::new();
    let dnd_for_init = dnd.clone();
    let app = rune::App::new(AppSpec {
        poll: Some(poll),
        on_key: Some(commands::on_key),
        fonts: FONTS,
        families: FAMILIES,
        ..AppSpec::new("Strata", SIZE, unused, shell::view)
    })
    // The last write of the per-folder views, before the writer is flushed. A quit runs no
    // `on_close`, so both ask for it; it is written once.
    .on_close(|app: &mut App, _: &mut Tx| {
        super::floating::close_with_browser(shell::window_id(app));
        folder_views::flush_at_exit(app);
        if closed(shell::window_id(app)) {
            shut_down();
        }
    })
    .on_quit(|app: &mut App, _: &mut Tx| {
        folder_views::flush_at_exit(app);
        if closed(shell::window_id(app)) {
            shut_down();
        }
    })
    .on_modifiers(shell::modifiers)
    .on_focus(focused)
    // Rune's own flags (`--snap`, `--tree`) are not this binary's.
    .args(Vec::new())
    .window_options(csd::window_options(frame, WindowOptions::default()));
    let handle = app.handle();
    let registered = handle.clone();
    app.init(move || {
        let mut app = App::new(&ui, Csd::new(frame, Backend::from_env()), settings);
        crate::clipboard::bind(&mut app, clip_for_init);
        crate::dnd::bind(&mut app, dnd_for_init);
        opened(shell::window_id(&app), registered);
        app.start(initial);
        app
    })
    .on_window(move |window: &WindowHandle| {
        crate::clipboard::attach(&clip, window, &handle);
        crate::dnd::attach(&dnd, window, &handle);
        csd::attach(window, &handle, shell::csd_of);
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
    let app = extend(window(ui, settings, initial));
    let glib = GlibLoop::start(glib::MainContext::default(), notify)
        .map_err(|error| format!("unable to start the GLib loop: {error}"))?;
    GLIB.with(|cell| cell.set(glib))
        .map_err(|_| "the GLib loop is already running on this thread".to_owned())?;
    // Rune ends the process after the quit hooks (`rune::fast_exit`), before `try_run`
    // returns: flush any queued setting or session write in the final hook, and keep the
    // fallback flush below for an early startup error.
    rune::fast_exit(strata_core::storage::writer::flush_at_exit);
    let ran = app.try_run();
    strata_core::storage::writer::flush_at_exit();
    ran
}
