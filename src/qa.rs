// SPDX-License-Identifier: MIT

mod check;
mod clipboard;
mod criticals;
mod dispatch;
mod driver;
mod input;
mod keyboard;
mod note;
mod observe;
mod pixels;
mod pointer;
mod protocol;
mod reveal;
mod samples;
mod state;
mod targets;
mod waits;

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::OsString,
    fs::File,
    io::{BufRead, Write},
    panic::AssertUnwindSafe,
    time::{Duration, Instant},
};

use futures_channel::mpsc;
use futures_lite::{FutureExt as _, StreamExt};
use gtk::{gdk, glib, prelude::*};
use serde_json::{Value, json};

use self::{
    input::Injector,
    pixels::Capture,
    protocol::{Failure, Reply},
    targets::{Boxes, Memory},
};

/// The release workflow and QA job grep for its `usage: strata --qa` prefix;
/// `scripts/test_qa_mutation.py` checks the patterns against this text.
const USAGE: &str = "usage: strata --qa <directory|--portal>";
/// Mirrors the private `ui::browser::peek::PEEK_LABEL`; a peek test fails if they drift.
pub(crate) const PEEK_LABEL: &str = "Folder peek";
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const READY_POLL: Duration = Duration::from_millis(5);
const FRAME_TIMEOUT: Duration = Duration::from_millis(250);
/// A layout change from the press makes GTK synthesize motion a frame later, and a
/// release it has not read yet would cancel the click.
const BUTTON_HOLD: Duration = Duration::from_millis(100);
/// Keeps two clicks inside GTK's 400 ms double-click time; nothing may settle between them.
const DOUBLE_CLICK_GAP: Duration = Duration::from_millis(90);
/// Outside the double-click time, so a drag after a selecting click is no double-click.
const SEPARATE_CLICK_GAP: Duration = Duration::from_millis(600);
const EVENT_GAP: Duration = Duration::from_millis(20);
const POINTER_GAP: Duration = Duration::from_millis(50);
/// A click can land before GTK maps the row for input, so it clicks again after this long.
const SELECT_RETRY_AFTER: Duration = Duration::from_secs(4);
const LOCATE_TIMEOUT: Duration = Duration::from_secs(15);
const SETTLE_SAMPLES: usize = 3;
const MENU_RETRY_AFTER: Duration = Duration::from_secs(1);
const KEYBOARD_SELECT_STEPS: usize = 40;
const PANIC_EXIT: i32 = 70;
const REVEAL_SCROLL_ATTEMPTS: usize = 12;
const WHEEL_REVEAL_CLICKS: u32 = 3;
const ARRIVAL_ATTEMPTS: usize = 5;
/// Approach a point in two steps so GTK sees enter and motion, not a teleport.
const APPROACH_OFFSET: i16 = 6;
const DROP_SAMPLE_DELAYS: [Duration; 3] = [
    Duration::from_millis(60),
    Duration::from_millis(120),
    Duration::from_millis(180),
];
const DROP_SAMPLE_LIMIT: Duration = Duration::from_millis(240);
const DROP_SAMPLE_DRIFT: f64 = 1.5;
const DROP_SAMPLE_RECOVERY: f64 = 1.25;

pub(crate) struct Session {
    protocol: File,
    application_arguments: Vec<OsString>,
    portal: bool,
}

const PORTAL_ARGUMENT: &str = "--portal";
const STREAMS_VARIABLE: &str = "STRATA_QA_STREAMS";

enum Host {
    Application(gtk::Application),
    Portal(glib::MainLoop),
}

impl Session {
    /// Takes over stdout for replies before anything can log there; app output goes to stderr.
    pub(crate) fn prepare(arguments: &[OsString]) -> Result<Self, String> {
        let [program, _, directory] = arguments else {
            return Err(USAGE.to_owned());
        };
        // The directory is opened like any launch argument, so a main-thread
        // I/O audit sees no file call that a release build would not make.
        let portal = directory == PORTAL_ARGUMENT;
        let protocol = rustix::io::fcntl_dupfd_cloexec(std::io::stdout(), 3)
            .map_err(|error| format!("cannot keep stdout for replies: {error}"))?;
        match std::env::var(STREAMS_VARIABLE) {
            Ok(streams) => replace_streams(&streams)?,
            Err(_) => rustix::stdio::dup2_stdout(std::io::stderr())
                .map_err(|error| format!("cannot move application output to stderr: {error}"))?,
        }
        criticals::capture();
        Ok(Self {
            protocol: File::from(protocol),
            application_arguments: vec![program.clone(), directory.clone()],
            portal,
        })
    }

    pub(crate) fn portal(&self) -> bool {
        self.portal
    }

    pub(crate) fn start_portal(self, main_loop: &glib::MainLoop) {
        self.start(Host::Portal(main_loop.clone()));
    }

    pub(crate) fn application_arguments(&self) -> &[OsString] {
        &self.application_arguments
    }

    pub(crate) fn attach(self, application: &gtk::Application) {
        let session = RefCell::new(Some(self));
        application.connect_startup(move |application| {
            if let Some(session) = session.take() {
                session.start(Host::Application(application.clone()));
            }
        });
    }

    fn start(self, host: Host) {
        let (sender, receiver) = mpsc::unbounded();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                if sender.unbounded_send(line).is_err() {
                    break;
                }
            }
        });
        let last_words = self.protocol.try_clone();
        let controller = Controller {
            host,
            protocol: self.protocol,
            injector: None,
            main: glib::WeakRef::new(),
            previous: None,
            settled: RefCell::default(),
            memory: RefCell::default(),
            widgets: RefCell::default(),
            samples: RefCell::default(),
            clipboard: RefCell::default(),
        };
        // GLib would keep a panic in the task's join handle and leave the
        // runner waiting; report it and exit instead.
        glib::MainContext::default().spawn_local(async move {
            let Err(panic) = AssertUnwindSafe(controller.serve(receiver))
                .catch_unwind()
                .await
            else {
                return;
            };
            let message = panic
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            if let Ok(mut protocol) = last_words {
                let reply = Reply {
                    ok: false,
                    app: Value::Null,
                    changed: Vec::new(),
                    criticals: criticals::take(),
                    result: Value::Null,
                    failures: Vec::new(),
                    error: Some(format!("the QA controller panicked: {message}")),
                };
                if let Ok(line) = serde_json::to_string(&reply) {
                    let _ = writeln!(protocol, "{line}");
                    let _ = protocol.flush();
                }
            }
            std::process::exit(PANIC_EXIT);
        });
    }
}

struct Controller {
    host: Host,
    protocol: File,
    injector: Option<Injector>,
    /// A floating chooser becomes the active window while open; state is read from the browser.
    main: glib::WeakRef<gtk::Window>,
    previous: Option<Value>,
    settled: RefCell<Boxes>,
    memory: RefCell<Memory>,
    widgets: RefCell<Memory>,
    samples: RefCell<HashMap<String, Sample>>,
    /// Owns the clipboard a `clipboard_set` set, until the `clipboard` read closes it.
    clipboard: RefCell<Option<gdk::Display>>,
}

#[derive(Clone)]
enum Sample {
    Pixels(Capture),
    Text { position: f64, contrast: u64 },
    Instant(Instant),
}

#[derive(Default)]
struct Outcome {
    result: Value,
    failures: Vec<Failure>,
    error: Option<String>,
}

impl Outcome {
    fn error(error: String) -> Self {
        Self {
            error: Some(error),
            ..Self::default()
        }
    }

    fn from(result: Result<(), String>) -> Self {
        result.map_or_else(Self::error, |()| Self::default())
    }

    fn timed_out(window: &gtk::Window, failure: Failure, timeout: Duration) -> Self {
        Self {
            result: json!({ "tree": state::dump(window) }),
            failures: vec![failure],
            error: Some(format!("timed out after {} ms", timeout.as_millis())),
        }
    }
}

impl Controller {
    async fn serve(mut self, mut commands: mpsc::UnboundedReceiver<String>) {
        let _hold = match &self.host {
            Host::Application(application) => Some(application.hold()),
            Host::Portal(_) => None,
        };
        let ready = match self.host {
            Host::Application(_) => self.await_ready().await,
            Host::Portal(_) => Ok(()),
        };
        let ready = match ready.and_then(|()| Injector::connect()) {
            Ok(injector) => {
                self.injector = Some(injector);
                Outcome {
                    result: json!({ "ready": true }),
                    ..Outcome::default()
                }
            }
            Err(error) => Outcome::error(error),
        };
        let started = ready.error.is_none();
        self.reply(ready);
        if started {
            while let Some(line) = commands.next().await {
                let outcome = match protocol::parse_command(&line) {
                    Ok(command) => self.execute(command).await,
                    Err(error) => Outcome::error(error),
                };
                self.reply(outcome);
            }
        }
        self.quit();
    }

    fn quit(&self) {
        match &self.host {
            Host::Application(application) => application.quit(),
            Host::Portal(main_loop) => main_loop.quit(),
        }
    }

    fn window(&self) -> Option<gtk::Window> {
        self.main.upgrade().or_else(|| match &self.host {
            Host::Application(application) => application
                .active_window()
                .or_else(|| application.windows().into_iter().next()),
            Host::Portal(_) => None,
        })
    }

    fn windows(&self) -> Vec<gtk::Window> {
        match &self.host {
            Host::Application(application) => application.windows(),
            Host::Portal(_) => gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|window| window.downcast::<gtk::Window>().ok())
                .collect(),
        }
    }

    async fn await_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let window = self.window();
            if let Some(window) = window.as_ref().filter(|window| state::is_ready(window)) {
                self.main.set(Some(window));
                settle(Some(window)).await;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("the Strata window never showed an entry list".to_owned());
            }
            match window.as_ref().and_then(WidgetExt::frame_clock) {
                Some(clock) => next_paint(&clock, POLL_INTERVAL).await,
                None => glib::timeout_future(READY_POLL).await,
            }
        }
    }

    fn reply(&mut self, outcome: Outcome) {
        let app = self
            .window()
            .map(|window| state::scan(&window).observation.app_state())
            .and_then(|app| serde_json::to_value(app).ok())
            .unwrap_or_else(|| match self.host {
                Host::Portal(_) => state::empty_app_state(),
                Host::Application(_) => Value::Null,
            });
        let changed = protocol::changed_keys(self.previous.as_ref(), &app);
        self.previous = Some(app.clone());
        let reply = Reply {
            ok: outcome.error.is_none() && outcome.failures.is_empty(),
            app,
            changed,
            criticals: criticals::take(),
            result: outcome.result,
            failures: outcome.failures,
            error: outcome.error,
        };
        let line = serde_json::to_string(&reply).unwrap_or_else(|error| {
            format!(r#"{{"ok":false,"error":"cannot encode the reply: {error}"}}"#)
        });
        if writeln!(self.protocol, "{line}")
            .and_then(|()| self.protocol.flush())
            .is_err()
        {
            self.quit();
        }
    }
}

fn replace_streams(spec: &str) -> Result<(), String> {
    for part in spec.split(',') {
        let (stream, kind) = part
            .split_once('=')
            .ok_or_else(|| format!("{STREAMS_VARIABLE} has no stream=kind in {part:?}"))?;
        let replacement: rustix::fd::OwnedFd = match kind {
            "closed" => {
                let (reader, writer) =
                    rustix::pipe::pipe().map_err(|error| format!("cannot open a pipe: {error}"))?;
                drop(reader);
                writer
            }
            "null" => File::options()
                .write(true)
                .open("/dev/null")
                .map_err(|error| format!("cannot open /dev/null: {error}"))?
                .into(),
            other => return Err(format!("{STREAMS_VARIABLE}: unknown kind {other:?}")),
        };
        match stream {
            "stdout" => rustix::stdio::dup2_stdout(&replacement),
            "stderr" => rustix::stdio::dup2_stderr(&replacement),
            other => return Err(format!("{STREAMS_VARIABLE}: unknown stream {other:?}")),
        }
        .map_err(|error| format!("cannot replace {stream}: {error}"))?;
    }
    Ok(())
}

async fn settle(window: Option<&gtk::Window>) {
    glib::timeout_future_with_priority(glib::Priority::LOW, Duration::ZERO).await;
    if let Some(clock) = window.and_then(WidgetExt::frame_clock) {
        let painted = next_paint(&clock, FRAME_TIMEOUT);
        clock.request_phase(gdk::FrameClockPhase::AFTER_PAINT);
        painted.await;
    }
    glib::timeout_future_with_priority(glib::Priority::LOW, Duration::ZERO).await;
}

/// Listens from the call, so a frame requested afterwards is not missed.
fn next_paint(clock: &gdk::FrameClock, limit: Duration) -> impl Future<Output = ()> + use<> {
    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = RefCell::new(Some(sender));
    let handler = clock.connect_after_paint(move |_| {
        if let Some(sender) = sender.take() {
            let _ = sender.send(());
        }
    });
    let clock = clock.clone();
    async move {
        let _ = glib::future_with_timeout(limit, receiver).await;
        clock.disconnect(handler);
    }
}

#[cfg(test)]
mod tests;
