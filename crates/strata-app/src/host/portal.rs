// SPDX-License-Identifier: MIT

use std::{
    cell::{OnceCell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rune::{AppHandle, AppSpec, Cx, El, Tx, WindowHandle, WindowSpec, col};
use strata_core::model::Location;
use strata_kit::fonts::{FAMILIES, FONTS};
use strata_platform::{
    dnd::Dnd,
    foreign_parent::{self, Options, Parent},
    glib_loop::GlibLoop,
};

use super::chooser_window::{self, ChooserWindow};
use crate::{
    App,
    chooser::{geometry::chooser_initial_dimensions, source::ChooserFileSource},
    folder_views,
    portal::{
        CHOOSER_APPLICATION_ID, ChooserRequest, Completion, PortalError,
        dbus::PortalService,
        inbox::{PortalEvent, PortalInbox},
    },
    settings::Saved,
    shell,
    startup::{self, UiThread},
};

const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(10);

thread_local! {
    static GLIB: OnceCell<GlibLoop> = const { OnceCell::new() };
    static WINDOWS: RefCell<HashMap<String, (WindowHandle, AppHandle<App>)>> =
        RefCell::new(HashMap::new());
    static HOOKS: OnceCell<Hooks> = const { OnceCell::new() };
}

/// Ends the daemon, and so the process, from any thread.
pub(crate) type Quit = Box<dyn Fn() + Send>;
/// Runs once before the daemon's loop, with what ends it.
pub(crate) type Started = Box<dyn FnOnce(Quit)>;
/// Each chooser window's app before it opens, with its request's token.
pub(crate) type Opening = Box<dyn Fn(rune::App<App>, &str) -> rune::App<App>>;
/// The token of a chooser window that closed.
pub(crate) type Closed = Box<dyn Fn(&str)>;

#[derive(Default)]
pub(crate) struct Hooks {
    pub(crate) started: Option<Started>,
    pub(crate) chooser: Option<Opening>,
    pub(crate) closed: Option<Closed>,
    /// The daemon runs on while no request is active.
    pub(crate) stays: bool,
}

fn hooks<T>(read: impl FnOnce(&Hooks) -> T) -> Option<T> {
    HOOKS.with(|hooks| hooks.get().map(read))
}

struct Daemon {
    ui: UiThread,
    inbox: PortalInbox,
    service: PortalService,
}

fn daemon_view(_: &Daemon, _: &mut Cx<Daemon>) -> El {
    col()
}

fn pump_glib() {
    GLIB.with(|glib| {
        if let Some(glib) = glib.get() {
            glib.pump(super::DISPATCH_BUDGET);
        }
    });
}

/// Runs what the service queued: windows to open or cancel, or the end of the daemon.
fn drain(daemon: &mut Daemon, tx: &mut Tx) {
    for event in daemon.inbox.take() {
        match event {
            PortalEvent::Present {
                request,
                cancelled,
                completion,
            } => open_chooser_window(daemon.ui, *request, cancelled, completion),
            PortalEvent::Cancel(token) => cancel_chooser_window(&token),
            PortalEvent::Quit => tx.quit(),
        }
    }
}

fn ends_idle(daemon: &Daemon) -> bool {
    let idle = WINDOWS.with(|windows| windows.borrow().is_empty());
    let stays = hooks(|hooks| hooks.stays).unwrap_or(false);
    if !idle || stays {
        return false;
    }
    daemon
        .service
        .requests
        .begin_shutdown_if_idle(Instant::now())
}

fn daemon_poll(daemon: &mut Daemon, _clock: f64, tx: &mut Tx) -> bool {
    pump_glib();
    drain(daemon, tx);
    if ends_idle(daemon) {
        tx.quit();
        return false;
    }
    tx.wake_after(IDLE_CHECK_INTERVAL);
    false
}

fn chooser_poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    shell::publish(app);
    pump_glib();
    let settled = shell::settle(app, tx);
    shell::poll(app, clock, tx) | settled
}

pub(crate) fn run(ui: UiThread, initial_settings: Saved) -> Result<(), String> {
    run_with(ui, initial_settings, Hooks::default())
}

/// [`run`], with what `hooks` add.
pub(crate) fn run_with(
    ui: UiThread,
    _initial_settings: Saved,
    mut hooks: Hooks,
) -> Result<(), String> {
    std::thread::spawn(strata_fs::services::prune_stale_downloads);
    rune::set_app_id(CHOOSER_APPLICATION_ID);

    let daemon_spec = AppSpec {
        poll: Some(daemon_poll),
        fonts: FONTS,
        families: FAMILIES,
        ..AppSpec::new(
            "Strata File Chooser",
            (1.0, 1.0),
            || unreachable!(),
            daemon_view,
        )
    };
    let daemon_app = rune::App::new(daemon_spec)
        .windowless()
        .keep_running()
        .args(Vec::new())
        .on_quit(|_: &mut Daemon, _: &mut Tx| {
            crate::dnd::shutdown();
            let _ = foreign_parent::shutdown();
        })
        .on_display_close(|| {
            crate::dnd::shutdown();
            let _ = foreign_parent::shutdown();
        });
    let daemon_handle = daemon_app.handle();
    if let Some(started) = hooks.started.take() {
        let quitting = daemon_handle.clone();
        started(Box::new(move || quitting.quit()));
    }
    HOOKS
        .with(|cell| cell.set(hooks))
        .map_err(|_| "the portal daemon is already running on this thread".to_owned())?;

    let inbox = PortalInbox::default();
    let wake = daemon_handle.clone();
    let callbacks = inbox.callbacks(Rc::new(move || {
        wake.update_if(|daemon, tx| {
            drain(daemon, tx);
            false
        });
    }));
    let service = PortalService::start(callbacks)?;

    let glib_wake = daemon_handle;
    let glib = GlibLoop::start(glib::MainContext::default(), move || {
        glib_wake.update_if(|daemon, tx| {
            pump_glib();
            drain(daemon, tx);
            WINDOWS.with(|windows| {
                for (_, handle) in windows.borrow().values() {
                    handle.update_if(shell::settle);
                }
            });
            false
        });
    })
    .map_err(|error| format!("unable to start the GLib loop: {error}"))?;
    GLIB.with(|cell| cell.set(glib))
        .map_err(|_| "the GLib loop is already running on this thread".to_owned())?;

    rune::fast_exit(strata_core::storage::writer::flush_at_exit);
    let ran = daemon_app
        .init(move || Daemon { ui, inbox, service })
        .try_run();
    let _ = foreign_parent::shutdown();
    strata_core::storage::writer::flush_at_exit();
    ran
}

fn cancel_chooser_window(token: &str) {
    let entry = WINDOWS.with(|windows| windows.borrow().get(token).cloned());
    if let Some((_, handle)) = entry {
        handle.update_if(crate::chooser::cancel);
    }
}

fn lost_parent(app: &mut App, tx: &mut Tx) {
    crate::chooser::cancel(app, tx);
}

/// A chooser window closing, by its close button or after its request finished.
fn close_chooser(app: &mut App, tx: &mut Tx) {
    crate::chooser::cancel(app, tx);
    folder_views::flush_at_exit(app);
    let Some(token) = app.chooser.token() else {
        return;
    };
    foreign_parent::release(&token);
    WINDOWS.with(|windows| {
        windows.borrow_mut().remove(&token);
    });
    hooks(|hooks| hooks.closed.as_ref().map(|closed| closed(&token)));
}

/// The chooser's own app over `state`, which shows its window once it is parented.
fn chooser_app(request: &ChooserRequest, state_size: (f32, f32), dnd: Dnd<App>) -> rune::App<App> {
    let parent = request
        .parent
        .as_deref()
        .and_then(|raw| Parent::parse(raw).ok())
        .unwrap_or(Parent::None);
    let modal = request.modal;
    let app = super::chooser_window::chooser_app(ChooserWindow {
        size: state_size,
        visible: false,
        poll: chooser_poll,
        on_close: close_chooser,
    });
    let handle = app.handle();
    app.on_window(move |window: &WindowHandle| {
        chooser_window::attach_window(window, &handle);
        crate::dnd::attach(&dnd, window, &handle);
        let options = Options::new().modal(modal).on_lost(lost_parent);
        let _ = foreign_parent::apply(window, &handle, &parent, options);
        window.show();
    })
}

fn open_chooser_window(
    ui: UiThread,
    request: ChooserRequest,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
) {
    if cancelled.load(Ordering::SeqCst) {
        completion(Err(PortalError::Cancelled(
            "file chooser request was cancelled".into(),
        )));
        return;
    }
    let token = request.token.clone();
    let previous = WINDOWS.with(|windows| windows.borrow_mut().remove(&token));
    if let Some((old_window, _)) = previous {
        old_window.close();
    }
    let (width, height) = chooser_initial_dimensions(None, request.parent_size_hint);
    #[expect(clippy::cast_precision_loss, reason = "window dimensions fit f32")]
    let size = (width as f32, height as f32);
    let initial = Location::local(request.initial_directory.clone());
    let dnd = Dnd::<App>::new();
    let app = chooser_app(&request, size, dnd.clone());
    let app = HOOKS.with(
        |cell| match cell.get().and_then(|hooks| hooks.chooser.as_ref()) {
            Some(chooser) => chooser(app, &token),
            None => app,
        },
    );
    let mut state = App::new_chooser(
        &ui,
        startup::load_settings(),
        (request, ChooserFileSource::new(None), None, completion),
    );
    crate::dnd::bind(&mut state, dnd);
    state.open(initial);
    let handle = app.handle();
    let wake = handle.clone();
    state
        .window
        .set_image_cache(Arc::new(rune::ImageCache::new(Arc::new(move || {
            wake.wake();
        }))));
    let window = rune::open_window(WindowSpec::from_app(&token, app).state(state));
    if cancelled.load(Ordering::SeqCst) {
        handle.update_if(crate::chooser::cancel);
        window.close();
        return;
    }
    WINDOWS.with(|windows| {
        windows.borrow_mut().insert(token, (window, handle));
    });
}
