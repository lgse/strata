// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use gio::prelude::*;
use rune::{App, AppHandle, AppSpec, Cx, DesktopSettingWatch, El, Tx, col};
use serde_json::json;
use strata_gallery::scenes::x1::reload::{self, Change, Debounce};
use strata_gallery::scenes::x1::shell::{Desktop, OverridesStatus, Volume, Volumes};
use strata_kit::look::Look;
use strata_kit::overrides::Overrides;
use strata_kit::theme::{DEFAULT_ID, Theme, bundled_theme};
use strata_platform::glib_bridge::{GlibThread, Keep, Signal, Task, TaskState};
use strata_platform::single_instance::{
    AppId, Claim, CommandLine, Event, Launch, Options as ClaimOptions, Primary, claim,
};

use crate::args::{self, Options};
use crate::log;
use crate::window::{self, Window};

/// The demo's own id: never Strata's, so a launch never reaches a running Strata.
pub const APP_ID: &str = "io.github.wmfeht.StrataComponents.Shell";

static PRIMARY: OnceLock<Primary> = OnceLock::new();
static CONTROLLER: OnceLock<AppHandle<Controller>> = OnceLock::new();
/// The open windows' keys and handles, newest last.
static WINDOWS: Mutex<Vec<(String, AppHandle<Window>)>> = Mutex::new(Vec::new());

pub fn windows() -> MutexGuard<'static, Vec<(String, AppHandle<Window>)>> {
    WINDOWS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn controller() -> Option<&'static AppHandle<Controller>> {
    CONTROLLER.get()
}

/// What a read of the overrides file found: its bytes, no file, or why it couldn't be read.
type Read = Result<Option<Vec<u8>>, String>;

pub struct Controller {
    pub theme: &'static Theme,
    pub borderless: bool,
    pub all_buttons: bool,
    pub desktop: Desktop,
    enable_animations: bool,
    reduced_motion: bool,
    overrides: Overrides,
    pub status: OverridesStatus,
    pub look: Look,
    path: Option<PathBuf>,
    debounce: Debounce,
    bridge: Option<GlibThread>,
    file_changes: Option<Signal<()>>,
    reading: Option<Task<Read>>,
    mount_changes: Option<Signal<()>>,
    listing: Option<Task<Vec<Volume>>>,
    pub mounts: Volumes,
    pub launches: u32,
}

/// `AppSpec::init` needs a plain function; `run` replaces this state with `App::init`.
fn placeholder() -> Controller {
    let theme = bundled_theme(DEFAULT_ID).unwrap_or_else(|| panic!("{DEFAULT_ID} is bundled"));
    let desktop = Desktop::default();
    Controller {
        theme,
        borderless: true,
        all_buttons: false,
        desktop,
        enable_animations: true,
        reduced_motion: false,
        look: reload::build(theme, &desktop, &Overrides::default()),
        overrides: Overrides::default(),
        status: OverridesStatus::default(),
        path: None,
        debounce: Debounce::default(),
        bridge: None,
        file_changes: None,
        reading: None,
        mount_changes: None,
        listing: None,
        mounts: Volumes::Loading,
        launches: 0,
    }
}

fn view(_: &Controller, _: &mut Cx<Controller>) -> El {
    col()
}

fn claim_primary() -> Result<Primary, i32> {
    let id = AppId::new(APP_ID).unwrap_or_else(|e| panic!("{APP_ID}: {e:?}"));
    match claim(&id, Launch::from_env(), &ClaimOptions::default()) {
        Claim::Primary(p) => {
            log(
                "claim",
                json!({ "result": "primary", "pid": std::process::id() }),
            );
            Ok(p)
        }
        Claim::Standalone(p, reason) => {
            log(
                "claim",
                json!({ "result": "standalone", "reason": format!("{reason:?}") }),
            );
            Ok(p)
        }
        Claim::Forwarded(status) => {
            log("claim", json!({ "result": "forwarded", "status": status }));
            Err(status)
        }
    }
}

fn start_glib(state: &mut Controller, wake: AppHandle<Controller>) {
    match GlibThread::start(Arc::new(move || wake.wake())) {
        Ok(bridge) => {
            state.mount_changes = Some(watch_mounts(&bridge));
            state.file_changes = state.path.clone().map(|path| watch_file(&bridge, path));
            state.listing = Some(list_mounts(&bridge));
            state.bridge = Some(bridge);
        }
        Err(e) => {
            log("glib", json!({ "error": e.to_string() }));
            state.mounts = Volumes::Failed(format!("No GLib bridge: {e}"));
        }
    }
}

fn watch_desktop_settings(handle: AppHandle<Controller>) -> Vec<DesktopSettingWatch> {
    let text_scale = {
        let handle = handle.clone();
        rune::desktop_setting(
            "org.gnome.desktop.interface",
            "text-scaling-factor",
            move |value| {
                if let Some(scale) = value.as_f64().filter(|s| s.is_finite() && *s > 0.0) {
                    #[expect(clippy::cast_possible_truncation, reason = "text scale fits f32")]
                    let scale = scale as f32;
                    handle.update(move |c: &mut Controller, _| {
                        c.update_desktop(|c| c.desktop.text_scale = scale);
                    });
                }
            },
        )
    };
    let double_click = {
        let handle = handle.clone();
        rune::desktop_setting(
            "org.gnome.desktop.peripherals.mouse",
            "double-click",
            move |value| {
                if let Some(ms) = value
                    .as_i64()
                    .filter(|ms| *ms > 0)
                    .and_then(|ms| u32::try_from(ms).ok())
                {
                    handle.update(move |c: &mut Controller, _| {
                        c.update_desktop(|c| c.desktop.double_click_ms = ms);
                    });
                }
            },
        )
    };
    let enable_animations = {
        let handle = handle.clone();
        rune::desktop_setting(
            "org.gnome.desktop.interface",
            "enable-animations",
            move |value| {
                if let Some(enabled) = value.as_bool() {
                    handle.update(move |c: &mut Controller, _| {
                        c.update_desktop(|c| c.enable_animations = enabled);
                    });
                }
            },
        )
    };
    let reduced_motion = rune::desktop_setting(
        "org.freedesktop.appearance",
        "reduced-motion",
        move |value| {
            if let Some(reduced) = value.as_i64().map(|v| v == 1) {
                handle.update(move |c: &mut Controller, _| {
                    c.update_desktop(|c| c.reduced_motion = reduced);
                });
            }
        },
    );
    vec![text_scale, double_click, enable_animations, reduced_motion]
}

pub fn run(options: Options) -> i32 {
    let primary = match claim_primary() {
        Ok(p) => p,
        Err(status) => return status,
    };
    rune::orderly_exit();
    rune::set_app_id(APP_ID);

    let theme_id = options.theme.as_deref().unwrap_or(DEFAULT_ID);
    let Some(theme) = bundled_theme(theme_id) else {
        eprintln!("shell: no bundled theme {theme_id}");
        primary.release();
        return 2;
    };
    let app = App::new(AppSpec {
        poll: Some(poll),
        fonts: strata_kit::fonts::FONTS,
        families: strata_kit::fonts::FAMILIES,
        ..AppSpec::new("Strata shell", (100.0, 100.0), placeholder, view)
    })
    .windowless()
    .args(Vec::new());
    let handle = app.handle();
    let _ = CONTROLLER.set(handle.clone());

    let mut state = placeholder();
    state.theme = theme;
    state.borderless = !options.native;
    state.all_buttons = options.all_buttons;
    state.path = options.overrides_path();

    // Read the file once before any window opens, so the first frame has its look.
    let first = state.path.as_deref().map(read_now);
    state.first_read(first.unwrap_or(Ok(None)));
    start_glib(&mut state, handle.clone());

    let watches = watch_desktop_settings(handle.clone());

    let app = app.init(move || state);
    primary.attach_rune(handle, on_event);
    let primary = PRIMARY.get_or_init(|| primary);
    let result = app.try_run();
    drop(watches);
    primary.release();
    if let Err(error) = result {
        log("exit-error", json!({ "error": error }));
        return 1;
    }
    primary.exit_status()
}

/// Gives the name up and quits, once the last window has closed.
pub fn last_window_closed() {
    if let Some(primary) = PRIMARY.get() {
        primary.release();
    }
    if let Some(controller) = controller() {
        controller.quit();
    }
}

fn on_event(c: &mut Controller, event: Event, _: &mut Tx) {
    match event {
        Event::CommandLine(cmd) => on_launch(c, cmd),
        Event::Activate(activation) => {
            let token = activation.token().map(str::to_owned);
            let newest = windows().last().map(|(_, h)| h.clone());
            match (newest, token) {
                (Some(w), Some(token)) => w.activate(token),
                (Some(w), None) => w.open(),
                (None, token) => window::open(c, home(), token.as_deref()),
            }
        }
        Event::Open {
            uris, activation, ..
        } => {
            let token = activation.token().map(str::to_owned);
            let last = uris.len().saturating_sub(1);
            for (i, uri) in uris.into_iter().enumerate() {
                window::open(c, uri, if i == last { token.as_deref() } else { None });
            }
        }
        Event::Action { .. } | Event::Reveal(_) => {}
    }
}

fn on_launch(c: &mut Controller, cmd: CommandLine) {
    let args::LaunchArgs {
        paths,
        startup_only,
    } = args::launch(cmd.arguments());
    let token = cmd.activation().token().map(str::to_owned);
    log(
        "launch",
        json!({
            "remote": cmd.is_remote(),
            "paths": paths.iter().map(|p| args::shown(p)).collect::<Vec<_>>(),
            "token": token.is_some(),
        }),
    );
    // A running shell keeps the theme, file and frame it started with.
    if cmd.is_remote() && !startup_only.is_empty() {
        let flags = startup_only.join(", ");
        log("launch", json!({ "ignored": startup_only }));
        cmd.print_err(&format!(
            "shell: {flags} apply only when the shell starts; the running one keeps its own\n"
        ));
    }
    let cwd = cmd.cwd().map(Path::to_path_buf);
    let newest = windows().last().map(|(_, h)| h.clone());
    match (paths.is_empty(), newest) {
        // A bare launch of a running app brings it forward.
        (true, Some(w)) => match token {
            Some(token) => w.activate(token),
            None => w.open(),
        },
        (true, None) => {
            let at = cwd.map_or_else(home, |p| p.display().to_string());
            window::open(c, at, token.as_deref());
        }
        (false, _) => {
            let last = paths.len() - 1;
            for (i, path) in paths.iter().enumerate() {
                let full = match &cwd {
                    Some(cwd) if !args::is_uri(path) => cwd.join(path),
                    _ => PathBuf::from(path),
                };
                let token = if i == last { token.as_deref() } else { None };
                window::open(c, args::shown(full.as_os_str()), token);
            }
        }
    }
    if cmd.is_remote() {
        cmd.print("opened in the running shell\n");
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

fn poll(c: &mut Controller, t: f64, tx: &mut Tx) -> bool {
    if c.file_changes
        .as_ref()
        .is_some_and(|s| s.drain().count() > 0)
    {
        let due = c.debounce.changed(t);
        tx.wake_after(Duration::from_secs_f64((due - t).max(0.0)));
    }
    if c.debounce.take_due(t) {
        c.read_again();
    }
    if let Some(read) = c.reading.as_ref().and_then(Task::take) {
        c.reading = None;
        c.reread(read);
    }
    if c.mount_changes
        .as_ref()
        .is_some_and(|s| s.drain().count() > 0)
    {
        c.list_volumes();
    }
    if let Some(list) = c.listing.as_ref().and_then(Task::take) {
        c.listing = None;
        log("volumes", json!({ "count": list.len() }));
        c.mounts = Volumes::Listed(list);
        c.send_mounts();
    } else if c
        .listing
        .as_ref()
        .is_some_and(|task| task.state() == TaskState::Stopped)
    {
        // The future panicked or the bridge stopped: it will never land.
        c.listing = None;
        log("volumes", json!({ "error": "the listing stopped" }));
        c.mounts = Volumes::Failed("The volume listing stopped before it finished".into());
        c.send_mounts();
    }
    false
}

impl Controller {
    pub fn list_volumes(&mut self) {
        match &self.bridge {
            Some(bridge) => self.listing = Some(list_mounts(bridge)),
            None => self.send_mounts(),
        }
    }

    fn send_mounts(&self) {
        let mounts = self.mounts.clone();
        broadcast(move |w, _| w.shell.volumes = mounts.clone());
    }

    fn read_again(&mut self) {
        let (Some(bridge), Some(path)) = (&self.bridge, self.path.clone()) else {
            return;
        };
        self.reading = Some(bridge.handle().spawn(move || async move {
            match gio::File::for_path(&path).load_contents_future().await {
                Ok((bytes, _)) => Ok(Some(bytes.to_vec())),
                Err(e) if e.matches(gio::IOErrorEnum::NotFound) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        }));
    }

    /// The first read, before any window opens: whatever it finds is the look.
    fn first_read(&mut self, read: Read) {
        let (overrides, present) = parse(read);
        self.take(overrides, present, 0);
    }

    fn reread(&mut self, read: Read) {
        let (overrides, present) = parse(read);
        match Change::of(&self.overrides, self.status.present, &overrides, present) {
            Change::Nothing => {}
            Change::Presence => {
                self.status.present = present;
                log(
                    "overrides",
                    json!({ "present": present, "reloads": self.status.reloads }),
                );
                let status = self.status.clone();
                broadcast(move |w, _| w.shell.overrides = status.clone());
            }
            Change::Look => {
                self.take(overrides, present, self.status.reloads + 1);
                self.send_look();
            }
        }
    }

    fn take(&mut self, overrides: Overrides, present: bool, reloads: u32) {
        for e in overrides.errors() {
            log("overrides", json!({ "problem": e.to_string() }));
        }
        self.status = reload::status(&overrides, present, reloads);
        log(
            "overrides",
            json!({
                "present": present,
                "problems": self.status.problems,
                "reloads": reloads,
            }),
        );
        self.overrides = overrides;
        self.look = reload::build(self.theme, &self.desktop, &self.overrides);
    }

    fn update_desktop(&mut self, update: impl FnOnce(&mut Controller)) {
        let before = self.desktop;
        update(self);
        self.desktop.animations = self.enable_animations && !self.reduced_motion;
        if self.desktop == before {
            return;
        }
        let desktop = self.desktop;
        log(
            "desktop",
            json!({
                "text_scale": desktop.text_scale,
                "double_click_ms": desktop.double_click_ms,
                "animations": desktop.animations,
            }),
        );
        self.look = reload::build(self.theme, &self.desktop, &self.overrides);
        self.send_look();
    }

    /// Every window moves to the controller's look, as a theme switch does.
    fn send_look(&self) {
        let look = self.look.clone();
        let desktop = self.desktop;
        let status = self.status.clone();
        broadcast(move |w, tx| {
            w.shell.desktop = desktop;
            w.shell.overrides = status.clone();
            strata_kit::theme::switch(&mut w.look, look.clone(), tx);
        });
    }
}

/// Runs `f` in every open window, on its own loop.
pub fn broadcast(f: impl Fn(&mut Window, &mut Tx) + Clone + Send + 'static) {
    for (_, handle) in windows().iter() {
        let f = f.clone();
        handle.update(move |w, tx| f(w, tx));
    }
}

fn parse(read: Read) -> (Overrides, bool) {
    let bytes = match read {
        Ok(bytes) => bytes,
        Err(why) => {
            log(
                "overrides",
                json!({ "error": format!("cannot read overrides file: {why}") }),
            );
            None
        }
    };
    reload::read(bytes.as_deref())
}

/// The file's bytes now, before the loop runs.
fn read_now(path: &Path) -> Read {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn list_mounts(bridge: &GlibThread) -> Task<Vec<Volume>> {
    bridge.handle().spawn(|| async {
        gio::VolumeMonitor::get()
            .mounts()
            .iter()
            .map(|mount| {
                let root = mount.root();
                Volume {
                    name: mount.name().to_string(),
                    detail: root
                        .path()
                        .map_or_else(|| root.uri().to_string(), |p| p.display().to_string()),
                }
            })
            .collect()
    })
}

fn watch_mounts(bridge: &GlibThread) -> Signal<()> {
    bridge.handle().listen(|emit| {
        let monitor = gio::VolumeMonitor::get();
        let removed = emit.clone();
        let added = monitor.connect_mount_added(move |_, _| {
            let _ = emit.emit(());
        });
        let gone = monitor.connect_mount_removed(move |_, _| {
            let _ = removed.emit(());
        });
        Keep::new().handler(&monitor, added).handler(&monitor, gone)
    })
}

fn watch_file(bridge: &GlibThread, path: PathBuf) -> Signal<()> {
    bridge.handle().listen(move |emit| {
        let file = gio::File::for_path(&path);
        match file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
            Ok(monitor) => {
                let live = emit.clone();
                let changed = monitor.connect_changed(move |_, _, _, _| {
                    let _ = emit.emit(());
                });
                let _ = live.emit(());
                Keep::new().object(&monitor).handler(&monitor, changed)
            }
            Err(e) => {
                log(
                    "overrides",
                    json!({ "watch-error": e.to_string(), "path": path.display().to_string() }),
                );
                Keep::new()
            }
        }
    })
}
