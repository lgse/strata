// SPDX-License-Identifier: MIT

//! Windowless `--portal` daemon host: owns `org.freedesktop.impl.portal.desktop.strata` on the
//! session bus, pumps GLib on Rune's event loop, opens each `ChooserRequest` as a foreign-parented
//! Rune window (`strata_platform::foreign_parent`), and exits after [`IDLE_TIMEOUT`](crate::portal::IDLE_TIMEOUT).

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

use rune::{AppHandle, AppSpec, Cx, El, Tx, WindowHandle, WindowOptions, WindowSpec, col};
use strata_core::model::Location;
use strata_kit::fonts::{FAMILIES, FONTS};
use strata_platform::{
    csd::{self, Backend, Csd, Policy},
    foreign_parent::{self, Options, Parent},
    glib_loop::GlibLoop,
};

use crate::{
    App,
    chooser::{geometry::chooser_initial_dimensions, source::ChooserFileSource},
    commands, folder_views,
    portal::{
        CHOOSER_APPLICATION_ID, ChooserRequest, Completion, PortalCallbacks, PortalError,
        RequestTracker, dbus::PortalService, window_geometry,
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
}

struct Daemon {
    tracker: Arc<RequestTracker>,
    _service: PortalService,
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

fn daemon_poll(daemon: &mut Daemon, _clock: f64, tx: &mut Tx) -> bool {
    pump_glib();
    if WINDOWS.with(|w| w.borrow().is_empty()) && daemon.tracker.begin_shutdown_if_idle(Instant::now()) {
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

pub(crate) fn run(ui: UiThread, _initial_settings: Saved) -> Result<(), String> {
    std::thread::spawn(strata_fs::services::prune_stale_downloads);
    window_geometry::apply_hyprland_centering_rule();
    rune::set_app_id(CHOOSER_APPLICATION_ID);

    let tracker = Arc::new(RequestTracker::default());
    let daemon_spec = AppSpec {
        poll: Some(daemon_poll),
        fonts: FONTS,
        families: FAMILIES,
        ..AppSpec::new("Strata File Chooser", (1.0, 1.0), || unreachable!(), daemon_view)
    };
    let daemon_app = rune::App::new(daemon_spec)
        .windowless()
        .keep_running()
        .args(Vec::new())
        .on_quit(|_: &mut Daemon, _: &mut Tx| {
            foreign_parent::shutdown();
        });
    let daemon_handle = daemon_app.handle();

    let present_ui = ui;
    let present_daemon = daemon_handle.clone();
    let cancel_daemon = daemon_handle.clone();
    let quit_daemon = daemon_handle.clone();
    let callbacks = PortalCallbacks {
        present: Rc::new(move |request, cancelled, completion| {
            present_daemon.update_if(move |_, _| {
                open_chooser_window(present_ui, request, cancelled, completion);
                false
            });
        }),
        cancel: Rc::new(move |token| {
            let token = token.to_owned();
            cancel_daemon.update_if(move |_, _| {
                cancel_chooser_window(&token);
                false
            });
        }),
        quit: Rc::new(move || {
            quit_daemon.update_if(|_, tx| {
                tx.quit();
                false
            });
        }),
    };
    let service = PortalService::start(tracker.clone(), callbacks)?;

    let glib_wake = daemon_handle;
    let glib = GlibLoop::start(glib::MainContext::default(), move || {
        glib_wake.update_if(|_, _| {
            pump_glib();
            WINDOWS.with(|windows| {
                for (_, (_, handle)) in windows.borrow().iter() {
                    handle.update_if(|app, tx| shell::settle(app, tx));
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
        .init(move || Daemon {
            tracker,
            _service: service,
        })
        .try_run();
    foreign_parent::shutdown();
    strata_core::storage::writer::flush_at_exit();
    ran
}

fn cancel_chooser_window(token: &str) {
    let entry = WINDOWS.with(|windows| windows.borrow().get(token).cloned());
    if let Some((_, handle)) = entry {
        handle.update_if(|app, tx| crate::chooser::cancel(app, tx));
    }
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
    if let Some((old_win, _)) = previous {
        old_win.close();
    }

    let settings = startup::load_settings();
    let frame = Policy::Auto.frame();
    let (w, h) = chooser_initial_dimensions(None, request.parent_size_hint);
    #[expect(clippy::cast_precision_loss, reason = "window dimensions fit f32")]
    let size = (w as f32, h as f32);
    let parent = request
        .parent
        .as_deref()
        .and_then(|raw| Parent::parse(raw).ok())
        .unwrap_or(Parent::None);
    let modal = request.modal;
    let initial_dir = request.initial_directory.clone();
    let leaked_title: &'static str = Box::leak(request.title.clone().into_boxed_str());

    let source = ChooserFileSource::new(None);
    let mut state = App::new_chooser(
        &ui,
        Csd::new(frame, Backend::from_env()),
        settings,
        (request, source, None, completion),
    );
    state.open(Location::local(initial_dir));

    let close_token = token.clone();
    let window_token = token.clone();
    let chooser_app = rune::App::new(AppSpec {
        poll: Some(chooser_poll),
        on_key: Some(commands::on_key),
        fonts: FONTS,
        families: FAMILIES,
        ..AppSpec::new(leaked_title, size, || unreachable!(), shell::view)
    })
    .window_options(WindowOptions {
        visible: false,
        ..csd::window_options(frame, WindowOptions::default())
    })
    .on_modifiers(shell::modifiers)
    .on_close(move |app: &mut App, tx: &mut Tx| {
        crate::chooser::cancel(app, tx);
        folder_views::flush_at_exit(app);
        foreign_parent::release(&close_token);
        WINDOWS.with(|windows| {
            windows.borrow_mut().remove(&close_token);
        });
    });

    let handle = chooser_app.handle();
    let attach_handle = handle.clone();
    let chooser_app = chooser_app.on_window(move |window: &WindowHandle| {
        csd::attach(window, &attach_handle, shell::csd_of);
        let options = Options::new()
            .modal(modal)
            .on_lost(|app: &mut App, tx: &mut Tx| {
                crate::chooser::cancel(app, tx);
            });
        let _ = foreign_parent::apply(window, &attach_handle, &parent, options);
        window.show();
    });

    let window = rune::open_window(WindowSpec::from_app(&window_token, chooser_app).state(state));
    if cancelled.load(Ordering::SeqCst) {
        handle.update_if(|app, tx| crate::chooser::cancel(app, tx));
        window.close();
        return;
    }
    WINDOWS.with(|windows| {
        windows.borrow_mut().insert(token, (window, handle));
    });
}
