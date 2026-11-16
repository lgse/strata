// SPDX-License-Identifier: MIT

//! The host in a window. Rune runs winit's loop, which never looks at GLib, so the default
//! GLib context is embedded in it: strata-platform's `glib_loop` waits for what the context
//! waits for on a helper thread, and its dispatch runs here, on Rune's loop.
//!
//! The helper reaches the loop through `AppHandle::update_if`. A `Waker` call never runs
//! `poll` while a Wayland window is covered, and a `wake_after` timer runs four times a
//! second then (Rune #139; `docs/rune/slice/V5-shell-and-loop.md` has the measurement).

use std::cell::OnceCell;

use rune::{AppSpec, Tx, WindowHandle, WindowOptions};
use strata_core::model::Location;
use strata_kit::fonts::{FAMILIES, FONTS};
use strata_platform::{
    csd::{self, Backend, Csd, Policy},
    glib_loop::GlibLoop,
};

use crate::{
    App, commands, folder_views, launch,
    settings::Saved,
    shell,
    startup::{APPLICATION_ID, UiThread},
};

/// Strata's default window size (`src/ui/window`).
const SIZE: (f32, f32) = (1200.0, 760.0);

thread_local! {
    /// The interface thread's GLib context in Rune's loop, from before the first frame.
    static GLIB: OnceCell<GlibLoop> = const { OnceCell::new() };
}

/// `AppSpec` needs a plain function; `App::init` supplies the state, so Rune never calls it.
fn unused() -> App {
    unreachable!("the window's state comes from App::init")
}

/// Dispatches what GLib has ready, then applies what the browser's observers queued. Runs
/// when the helper thread says GLib has work, and in `poll`: a handler may have attached a
/// source since, which the helper cannot know of.
fn pump(app: &mut App, tx: &mut Tx) -> bool {
    shell::publish(app);
    GLIB.with(|glib| {
        if let Some(glib) = glib.get() {
            glib.pump(super::DISPATCH_BUDGET);
        }
    });
    shell::settle(app, tx)
}

fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let pumped = pump(app, tx);
    shell::poll(app, clock, tx) | pumped
}

/// Opens the window on `initial` and exits when it closes, returning an error if startup fails.
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

fn run_initial(
    ui: UiThread,
    settings: Saved,
    initial: launch::Initial,
    extend: impl FnOnce(rune::App<App>) -> rune::App<App>,
) -> Result<(), String> {
    rune::set_app_id(APPLICATION_ID);
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
    // The last write of the per-folder views, before the writer is flushed below. A quit runs
    // no `on_close`, so both ask for it; it is written once.
    .on_close(|app: &mut App, _: &mut Tx| {
        crate::dnd::shutdown();
        crate::clipboard::shutdown();
        folder_views::flush_at_exit(app);
    })
    .on_quit(|app: &mut App, _: &mut Tx| {
        crate::dnd::shutdown();
        crate::clipboard::shutdown();
        folder_views::flush_at_exit(app);
    })
    .on_modifiers(shell::modifiers)
    .init(move || {
        let mut app = App::new(&ui, Csd::new(frame, Backend::from_env()), settings);
        crate::clipboard::bind(&mut app, clip_for_init);
        crate::dnd::bind(&mut app, dnd_for_init);
        app.start(initial);
        app
    })
    // Rune's own flags (`--snap`, `--tree`) are not this binary's.
    .args(Vec::new())
    .window_options(csd::window_options(frame, WindowOptions::default()));
    let app = extend(app);
    let handle = app.handle();
    let notified = handle.clone();
    let glib = GlibLoop::start(glib::MainContext::default(), move || {
        notified.update_if(pump);
    })
    .map_err(|error| format!("unable to start the GLib loop: {error}"))?;
    GLIB.with(|cell| cell.set(glib))
        .map_err(|_| "the GLib loop is already running on this thread".to_owned())?;
    // Rune ends the process after the quit hooks (`rune::fast_exit`), before `try_run`
    // returns: flush any queued setting or session write in the final hook, and keep the
    // fallback flush below for an early startup error.
    rune::fast_exit(strata_core::storage::writer::flush_at_exit);
    let ran = app
        .on_window(move |window: &WindowHandle| {
            crate::clipboard::attach(&clip, window, &handle);
            crate::dnd::attach(&dnd, window, &handle);
            csd::attach(window, &handle, shell::csd_of);
        })
        .try_run();
    strata_core::storage::writer::flush_at_exit();
    ran
}
