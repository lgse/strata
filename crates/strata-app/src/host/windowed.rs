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
//! `poll`, which pumps too.
//!
//! What a dispatch queues for another window's browser, a folder's change say, and what one
//! window's change of a setting queues for the others, would wait for a frame of that window's
//! own, which may never come: Rune runs `poll` only before a frame, and a window shown without
//! the keyboard gets none until something moves in it. So every window with an update queued is
//! woken to apply it ([`wake_pending`]), after every dispatch and after every setting a window
//! commits, as each GTK window updates at once.
//!
//! # Refused closes
//!
//! GTK refuses a window's close while one of its guards says no (`shell::close_guard`), whoever
//! asks for it. Rune lets nothing refuse a close the window manager asks for (its title bar,
//! Alt+F4, the compositor): the window is off screen when `on_close` runs, and its state goes
//! with it (Rune packet entry 110, finding lgse/strata#2473). So `on_close` keeps the window by
//! hand ([`refused`]): it moves the state into a window opened in its place, which says why, and
//! leaves the closing window a stand-in state to go with. The running file operations and jobs
//! go on in the state kept.
//!
//! What moves is the `App`: every tab, its location, selection and history, the browsers with
//! their operations, undo, and the dialogs open in it. Strata's own menus and popovers are
//! `App` state too (the context menu's `ContextMenuState`, the plain popovers' open flags), so
//! one open at the close comes back open in the new window, at the point it had in the old
//! one. What Rune keeps per native window does not move, so the window comes back without it:
//!
//! - its place and size: the window manager places the new window at the default size;
//! - a maximised or full-screen state;
//! - the views' scroll offsets, so a long listing comes back at its top, its cursor perhaps off
//!   screen;
//! - the focused widget, and the keyboard until Rune's first focus event gives it
//!   (`shell::reopened`);
//! - Rune's own transient UI, such as a tooltip.
//!
//! The old window is seen to go before the new one maps, since Rune hides it before `on_close`.
//!
//! **Why not `keep_running`.** With it, Rune's close hides the window and keeps its native
//! window and state, and `AppHandle::show` brings it back as it was. But at the pin
//! `keep_running` holds for every close of the window's `App`, refused or not, and nothing drops
//! one window of several: `AppHandle::quit` ends the whole application. A window whose close
//! goes ahead would then stay hidden with its state alive (its browsers and their watchers, its
//! share of the thumbnail budget, a place in `is_last_window`), and the process would have to
//! quit by hand when the last shown window closes. Rune packet entry 110 asks for the close
//! request that makes both workarounds unneeded.

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

/// Strata's default window size (`src/ui/window`).
const SIZE: (f32, f32) = (1200.0, 760.0);

thread_local! {
    /// The interface thread's GLib context in Rune's loop, from before the first frame.
    static GLIB: OnceCell<GlibLoop> = const { OnceCell::new() };
    /// The interface thread, for the windows opened after the first.
    static UI: OnceCell<UiThread> = const { OnceCell::new() };
    /// Every window's queue and handle by window id, the floating choosers' among them: what a
    /// dispatch in one window, or another window's change of a setting, queues into
    /// ([`wake_pending`]).
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

/// Dispatches what GLib has ready, then applies what the browser's observers queued, and opens
/// the windows the dispatch asked for. Runs when the helper thread says GLib has work, and in
/// `poll`: a handler may have attached a source since, which the helper cannot know of.
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

/// Lets [`wake_pending`] wake window `app`, which `handle` reaches, in place of any handle the
/// window had before (a refused close opens its state in a new window).
pub(super) fn follow_queue(app: &App, handle: AppHandle<App>) {
    let (id, later) = (shell::window_id(app), shell::window_later(app).downgrade());
    QUEUES.with(|queues| {
        let mut queues = queues.borrow_mut();
        queues.retain(|(open, ..)| *open != id);
        queues.push((id, later, handle));
    });
}

/// Has every window with an update queued apply it now (the module docs, "Windows"). A window
/// whose queue is empty is not asked, so a wake-up that applies nothing draws nothing.
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

/// Keeps window `id`'s handle, in place of one it had before.
fn opened(id: u64, handle: AppHandle<App>) {
    let mut windows = WINDOWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    windows.retain(|(open, _)| *open != id);
    windows.push((id, handle));
}

/// Whether the application ends when window `id` closes: it is the only browser window open,
/// or no window is registered, as on the offscreen hosts.
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

/// Opens a window for each launch the bus handed over since the last pump, each created with
/// the activation token its launch forwarded, so the compositor lets it take the focus.
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

/// What a window starts from: new state on `initial`, or the state of a window whose close was
/// refused ([`refused`]).
enum Start {
    New(Box<(Saved, launch::Initial)>),
    Kept(Box<App>),
}

/// Opens the window on `initial` and exits when the last window closes, returning an error if
/// startup fails.
pub(crate) fn run(ui: UiThread, settings: Saved, initial: launch::Initial) -> Result<(), String> {
    run_initial(ui, settings, initial, |app| app)
}

/// Opens the window on `initial` with the app handed to `extend` before it runs: the QA mode's
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
    initial: launch::Initial,
    extend: impl FnOnce(rune::App<App>) -> rune::App<App>,
) -> Result<(), String> {
    run_initial(ui, settings, initial, extend)
}

/// One window's app from `start`: its state, its hooks, and the clipboard, drag and drop and
/// frame that attach to each native window it gets.
fn window(ui: UiThread, start: Start) -> rune::App<App> {
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
    .compositor(crate::fx::FxCompositor::new())
    // The last write of the per-folder views, before the writer is flushed. A quit runs no
    // `on_close`, so both ask for it; it is written once.
    .on_close(|app: &mut App, _: &mut Tx| {
        if refused::keep_open(app) {
            return;
        }
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
    // Rune ends the process after the quit hooks (`rune::fast_exit`), before `try_run`
    // returns: flush any queued setting or session write in the final hook, and keep the
    // fallback flush below for an early startup error.
    rune::fast_exit(strata_core::storage::writer::flush_at_exit);
    let ran = app.try_run();
    strata_core::storage::writer::flush_at_exit();
    ran
}
