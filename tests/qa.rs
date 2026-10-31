// SPDX-License-Identifier: MIT

// The process's own parser, so a malformed step fails at load, not as a failure `known_bug` could excuse.
#[path = "../src/qa/protocol/mod.rs"]
#[expect(dead_code, reason = "the runner only validates commands")]
mod protocol;

#[path = "qa/peers.rs"]
mod peers;

#[path = "qa/harness.rs"]
mod harness;

#[path = "qa/baseline.rs"]
mod baseline;

#[path = "qa/drive.rs"]
mod drive;

#[path = "qa/fixture.rs"]
mod fixture;

#[path = "qa/fs_check.rs"]
mod fs_check;

#[path = "qa/fs_helpers.rs"]
mod fs_helpers;

#[path = "qa/input_method.rs"]
mod input_method;

#[path = "qa/keyboard.rs"]
mod keyboard;

#[path = "qa/main_io.rs"]
mod main_io;

#[path = "qa/peer_steps.rs"]
mod peer_steps;

#[path = "qa/process.rs"]
mod process;

#[path = "qa/run.rs"]
mod run;

#[path = "qa/runner_steps.rs"]
mod runner_steps;

#[path = "qa/scenario.rs"]
mod scenario;

#[path = "qa/scenario_tests.rs"]
mod scenario_tests;

#[path = "qa/step_tests.rs"]
mod step_tests;

#[path = "qa/steps.rs"]
mod steps;

#[path = "qa/unit_tests.rs"]
mod unit_tests;

#[path = "qa/variables.rs"]
mod variables;

#[path = "qa/wayland.rs"]
mod wayland;

use std::{
    fmt::Write as _,
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use x11rb::{protocol::xkb, rust_connection::RustConnection};

use drive::{Context, drive, wait_for_fs};
use fixture::{
    FIXED_FIXTURES, Manifest, OrderedMap, expand_tree, fixed_fixture_name, fixture_time,
    gtk_settings, listing, pin_times,
};
use fs_check::FsState;
use input_method::InputMethod;
use peer_steps::{Peers, Relaunch};
use process::{Bus, SPAWN, spawn, tail, terminate, wait_for_exit};
use run::{Outcome, pretty, run_result, settled_state};
use scenario::{Scenario, planned_tests};
use steps::run_steps;
use variables::{Places, Run, substitute, substitute_text, unescape, variables};

// scripts/test_qa_drive.py checks qa-drive.sh against the launch values in this file.
const REPOSITORY: &str = env!("CARGO_MANIFEST_DIR");
const BINARY: &str = env!("CARGO_BIN_EXE_strata");
const DEFAULT_WAIT: Duration = Duration::from_millis(15_000);
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(90);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
const WAIT_MARGIN: Duration = Duration::from_secs(30);
const EXIT_TIMEOUT: Duration = Duration::from_secs(10);
/// `TERMINATE_GRACE_SECONDS` in `scripts/headless/process.py`.
const TERMINATE_GRACE: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const FIRST_DISPLAY: u32 = 90;
const DISPLAY_TRIES: u32 = 256;
const XVFB_START_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_LAYOUT: &str = "us";
const CONTROLLER_PANIC: &str = "the QA controller panicked";
const RUNNER_COMMANDS: [&str; 24] = [
    "layout",
    "fs",
    "increment",
    "slice",
    "screenshot",
    "baseline",
    "stem",
    "parent",
    "flag",
    "restart",
    "dbus_send",
    "log",
    "proc",
    "each",
    "launch",
    "portal_open",
    "portal_response",
    "http",
    "http_requests",
    "foreign",
    "foreign_label",
    "foreign_stop",
    "zipcrypto",
    "fail",
];
const NOTED_PLACEHOLDER: &str = "noted";
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(30);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);
/// `SESSION_BUS_CONFIG` in `scripts/headless/display.py`: no service directories, so
/// nothing on the bus is D-Bus-activatable.
const SESSION_BUS_CONFIG: &str = r#"<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={socket}</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;
const STEP_MODES: &str = "modes";
const STEP_VARIANTS: &str = "variants";
const STEP_WHEN: &str = "when";
const STEP_IF: &str = "if";
const STEP_UNLESS: &str = "unless";
const DEFAULT_TREE: &str = "canonical";
const INPUT_LOG: &str = "input.log";
const STATE_FILE: &str = "state.json";
const RESULT_FILE: &str = "result.json";
const XDG_DIRECTORIES: [(&str, &str); 5] = [
    ("HOME", "home"),
    ("XDG_CONFIG_HOME", "config"),
    ("XDG_DATA_HOME", "data"),
    ("XDG_CACHE_HOME", "cache"),
    ("XDG_STATE_HOME", "state"),
];
const RUNNER_OWNED_ENVIRONMENT: [&str; 8] = [
    "DISPLAY",
    "WAYLAND_DISPLAY",
    wayland::SWAY_SOCKET_VARIABLE,
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "STRATA_INPUT_LOG",
    "STRATA_STATE_DUMP",
    SHADER_CACHE_VARIABLE,
];
/// No AT-SPI: the process reads its own widget tree.
const QA_ENVIRONMENT: [(&str, &str); 2] = [("GTK_A11Y", "none"), ("NO_AT_BRIDGE", "1")];
/// One Mesa shader cache for every run, so launches do not recompile shaders in their
/// fresh `XDG_CACHE_HOME`.
const SHADER_CACHE_VARIABLE: &str = "MESA_SHADER_CACHE_DIR";
static SHADER_CACHE: Mutex<Option<tempfile::TempDir>> = Mutex::new(None);

fn shader_cache() -> Result<PathBuf, String> {
    let mut cache = SHADER_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(directory) = cache.as_ref() {
        return Ok(directory.path().to_owned());
    }
    let directory = tempfile::Builder::new()
        .prefix("strata-qa-shaders-")
        .tempdir_in("/tmp")
        .map_err(|error| format!("cannot create the shader cache: {error}"))?;
    Ok(cache.insert(directory).path().to_owned())
}

fn main() -> std::process::ExitCode {
    let arguments = match harness::Arguments::parse(std::env::args().skip(1)) {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("error: {error}");
            return std::process::ExitCode::from(harness::FAILED);
        }
    };
    if host_run() {
        eprintln!(
            "UNSUPPORTED: STRATA_QA_HOST=1 runs QA scenarios on the host toolkit. \
             This run is not validation evidence; use ./scripts/qa.sh."
        );
    }
    let status = harness::run(planned_tests(), &arguments);
    drop(
        SHADER_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take(),
    );
    status
}

fn in_container() -> bool {
    std::env::var_os("STRATA_QA_CONTAINER").is_some_and(|value| value == "1")
}

fn host_run() -> bool {
    !in_container() && std::env::var_os("STRATA_QA_HOST").is_some_and(|value| value == "1")
}

fn require_container() {
    if in_container() || host_run() {
        return;
    }
    panic!(
        "QA scenarios run only in the pinned container: use ./scripts/qa.sh \
         (STRATA_QA_CONTAINER=1 is set there). A host run needs the owner's explicit \
         consent and STRATA_QA_HOST=1, and is never validation evidence."
    );
}

fn render_toml(values: &OrderedMap) -> String {
    let mut output = String::new();
    for (key, value) in &values.0 {
        let _ = writeln!(output, "{key} = {}", render_value(value));
    }
    output
}

fn render_value(value: &Value) -> String {
    match value {
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(render_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::String(text) => render_string(text),
        Value::Object(entries) => format!(
            "{{ {} }}",
            entries
                .iter()
                .map(|(key, value)| format!("{} = {}", render_string(key), render_value(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Null => panic!("settings.toml cannot hold {value}"),
    }
}

fn render_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn populate(directory: &Path, tree: &Value) {
    let Value::Object(entries) = tree else {
        panic!("a fixture directory must be an object");
    };
    for (name, value) in entries {
        let path = directory.join(name);
        match value {
            Value::Object(_) => {
                fs::create_dir_all(&path).expect("the fixture directory should be created");
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                    .expect("the fixture directory mode should be set");
                populate(&path, value);
            }
            Value::String(contents) => {
                fs::write(&path, contents).expect("the fixture file should be written");
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
                    .expect("the fixture file mode should be set");
            }
            _ => panic!("fixture {name} must be a file's text or a directory"),
        }
    }
}

static NEXT_DISPLAY: AtomicU32 = AtomicU32::new(0);

/// `x11` (the default) or `wayland`: the display server every run of this process gets.
const DISPLAY_VARIABLE: &str = "STRATA_QA_DISPLAY";

struct Display {
    name: String,
    server: Child,
    log: PathBuf,
    kind: Server,
}

enum Server {
    X11 { connection: Box<RustConnection> },
    Wayland(wayland::Sway),
}

impl Display {
    fn start(scratch: &Path, runtime: &Path) -> Result<Self, String> {
        match std::env::var(DISPLAY_VARIABLE).as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("" | "x11") => {
                Self::start_x11(scratch.join("xvfb.log"))
            }
            Ok("wayland") => wayland::start(scratch, runtime),
            Ok(other) => Err(format!(
                "{DISPLAY_VARIABLE} is x11 or wayland, not {other:?}"
            )),
            Err(error) => Err(format!("{DISPLAY_VARIABLE}: {error}")),
        }
    }

    /// `-displayfd` confirms this server, not a concurrently starting one, owns the number;
    /// one that loses a number to another server tries the next.
    fn start_x11(log: PathBuf) -> Result<Self, String> {
        let first = NEXT_DISPLAY.fetch_add(1, Ordering::Relaxed);
        for offset in 0..DISPLAY_TRIES {
            let number = FIRST_DISPLAY + first.wrapping_add(offset) % DISPLAY_TRIES;
            if Path::new(&format!("/tmp/.X{number}-lock")).exists()
                || Path::new(&format!("/tmp/.X11-unix/X{number}")).exists()
            {
                continue;
            }
            if let Some(display) = Self::try_start(number, &log)? {
                return Ok(display);
            }
        }
        Err(format!(
            "no free X display in :{FIRST_DISPLAY}-:{}",
            FIRST_DISPLAY + DISPLAY_TRIES - 1
        ))
    }

    fn try_start(number: u32, log: &Path) -> Result<Option<Self>, String> {
        let io = |error: std::io::Error| error.to_string();
        let output = File::create(log).map_err(io)?;
        let errors = output.try_clone().map_err(io)?;
        let (reader, writer) = std::io::pipe().map_err(io)?;
        let mut command = Command::new("Xvfb");
        command
            .arg(format!(":{number}"))
            .args(["-displayfd", &writer.as_raw_fd().to_string()])
            .args(["-screen", "0", "1440x900x24", "-dpi", "96"])
            .args(["-nolisten", "tcp", "-noreset", "-ac"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(errors);
        let mut server = {
            let _spawning = SPAWN.lock().map_err(|error| error.to_string())?;
            rustix::io::fcntl_setfd(&writer, rustix::io::FdFlags::empty())
                .map_err(|error| error.to_string())?;
            let spawned = command.spawn();
            rustix::io::fcntl_setfd(&writer, rustix::io::FdFlags::CLOEXEC)
                .map_err(|error| error.to_string())?;
            spawned.map_err(|error| format!("cannot start Xvfb: {error}"))?
        };
        let (sender, advertised) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(reader).read_line(&mut line);
            let _ = sender.send(line);
        });
        let deadline = Instant::now() + XVFB_START_TIMEOUT;
        let owned = loop {
            match advertised.recv_timeout(POLL_INTERVAL) {
                Ok(line) => break line == format!("{number}\n"),
                Err(RecvTimeoutError::Timeout)
                    if Instant::now() < deadline && matches!(server.try_wait(), Ok(None)) => {}
                Err(_) => break false,
            }
        };
        // Xvfb writes the number and the newline separately; the pipe stays
        // open until both arrived so the newline cannot hit EPIPE.
        drop(writer);
        let name = format!(":{number}");
        let connection = owned.then(|| x11rb::connect(Some(&name)).ok()).flatten();
        let Some((connection, _)) = connection else {
            terminate(&mut server);
            return Ok(None);
        };
        xkb::use_extension(&connection, 1, 0)
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?;
        Ok(Some(Self {
            name,
            server,
            log: log.to_owned(),
            kind: Server::X11 {
                connection: Box::new(connection),
            },
        }))
    }

    /// What a client needs to reach this display.
    fn environment(&self) -> Vec<(&'static str, String)> {
        match &self.kind {
            Server::X11 { .. } => vec![("DISPLAY", self.name.clone())],
            Server::Wayland(sway) => vec![
                ("WAYLAND_DISPLAY", self.name.clone()),
                (
                    wayland::SWAY_SOCKET_VARIABLE,
                    sway.ipc.display().to_string(),
                ),
                ("GDK_BACKEND", "wayland".to_owned()),
            ],
        }
    }

    fn x11(&self) -> Result<&RustConnection, String> {
        match &self.kind {
            Server::X11 { connection } => Ok(connection),
            Server::Wayland(_) => Err("keyboard layouts need the X11 display".to_owned()),
        }
    }

    fn screenshot(&self, destination: &Path) -> Result<(), String> {
        let mut child = match &self.kind {
            Server::X11 { .. } => spawn(
                Command::new("import")
                    .args(["-display", &self.name, "-window", "root"])
                    .arg(destination)
                    .env_clear()
                    .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )
            .map_err(|error| format!("cannot run import: {error}"))?,
            Server::Wayland(sway) => sway.screenshot(&self.name, destination)?,
        };
        let program = match &self.kind {
            Server::X11 { .. } => "import",
            Server::Wayland(_) => "grim",
        };
        let deadline = Instant::now() + CAPTURE_TIMEOUT;
        loop {
            match child.try_wait().map_err(|error| error.to_string())? {
                Some(status) if status.success() => return Ok(()),
                Some(status) => return Err(format!("{program} exited with {status}")),
                None if Instant::now() >= deadline => {
                    terminate(&mut child);
                    return Err(format!(
                        "{program} ran past {} s",
                        CAPTURE_TIMEOUT.as_secs()
                    ));
                }
                None => std::thread::sleep(POLL_INTERVAL),
            }
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        terminate(&mut self.server);
    }
}

struct Launch<'a> {
    manifest: &'a Manifest,
    home: &'a Path,
    fixture: &'a Path,
    display: &'a Display,
    runtime: &'a Path,
    log: PathBuf,
    input_log: &'a Path,
    extra: Vec<(String, String)>,
    bus: Option<&'a str>,
    portal: bool,
    audit: Option<&'a main_io::Audit>,
}

impl Launch<'_> {
    fn start(&self) -> Result<Application, String> {
        Application::start(self, &Relaunch::default())
    }

    fn helper_environment(&self) -> Vec<(String, String)> {
        let mut variables = vec![("PATH".to_owned(), std::env::var("PATH").unwrap_or_default())];
        for (variable, directory) in XDG_DIRECTORIES {
            variables.push((
                variable.to_owned(),
                self.home.join(directory).display().to_string(),
            ));
        }
        variables.extend(
            self.manifest
                .environment
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        variables.extend(
            self.display
                .environment()
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value)),
        );
        variables.extend(self.extra.iter().cloned());
        variables.push((
            "XDG_RUNTIME_DIR".to_owned(),
            self.runtime.display().to_string(),
        ));
        if let Some(bus) = self.bus {
            variables.push(("DBUS_SESSION_BUS_ADDRESS".to_owned(), bus.to_owned()));
        }
        variables.extend(stall_limit());
        variables
    }

    /// `program`, under `strace` when the run is part of a main-thread I/O audit.
    fn command(&self, program: &str) -> Result<(Command, Option<main_io::Capture>), String> {
        match self.audit {
            Some(audit) => {
                let capture = audit.capture(self)?;
                Ok((capture.command(program), Some(capture)))
            }
            None => Ok((Command::new(program), None)),
        }
    }
}

/// The UI stall watchdog's limit, passed on to every launch when the runner has one.
fn stall_limit() -> Option<(String, String)> {
    std::env::var(main_io::STALL_VARIABLE)
        .ok()
        .filter(|limit| !limit.is_empty())
        .map(|limit| (main_io::STALL_VARIABLE.to_owned(), limit))
}

struct Application {
    child: Child,
    stdin: Option<ChildStdin>,
    replies: Receiver<String>,
    log: PathBuf,
    responsive: bool,
    capture: Option<main_io::Capture>,
    traced: std::cell::OnceCell<u32>,
}

impl Application {
    fn start(launch: &Launch, relaunch: &Relaunch) -> Result<Self, String> {
        let log = launch.log.clone();
        let output = File::create(&log).map_err(|error| error.to_string())?;
        let (mut command, capture) = launch.command(BINARY)?;
        // Its own process group, so a restart can signal the application and whatever it started.
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command.arg("--qa");
        match (&relaunch.directory, launch.portal) {
            (Some(directory), _) => command.arg(directory),
            (None, true) => command.arg("--portal"),
            (None, false) => command.arg(launch.fixture),
        };
        command
            .current_dir(launch.fixture)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .envs(&launch.manifest.environment)
            .envs(QA_ENVIRONMENT)
            .envs(launch.display.environment())
            .env("XDG_RUNTIME_DIR", launch.runtime)
            .env("STRATA_INPUT_LOG", launch.input_log)
            .env(SHADER_CACHE_VARIABLE, shader_cache()?)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                launch.bus.map_or_else(
                    || {
                        format!(
                            "unix:path={}",
                            launch.runtime.join("no-session-bus").display()
                        )
                    },
                    str::to_owned,
                ),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(output);
        for (variable, directory) in XDG_DIRECTORIES {
            command.env(variable, launch.home.join(directory));
        }
        command.envs(launch.extra.iter().map(|(key, value)| (key, value)));
        command.envs(stall_limit());
        command.envs(&relaunch.env);
        if let Some(streams) = relaunch.streams() {
            command.env("STRATA_QA_STREAMS", streams);
        }
        let mut child =
            spawn(&mut command).map_err(|error| format!("cannot start {BINARY}: {error}"))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (sender, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            stdin: child.stdin.take(),
            child,
            replies,
            log,
            responsive: true,
            capture,
            traced: std::cell::OnceCell::new(),
        })
    }

    /// Strata's own pid, which is not the child's when `strace` runs it.
    fn pid(&self) -> Result<u32, String> {
        match self.capture {
            Some(_) => main_io::traced_child(self.child.id(), &self.traced),
            None => Ok(self.child.id()),
        }
    }

    fn receive(&mut self, timeout: Duration) -> Result<Value, String> {
        let received = self.replies.recv_timeout(timeout);
        self.responsive = received.is_ok();
        match received {
            Ok(line) => {
                let reply: Value = serde_json::from_str(&line)
                    .map_err(|error| format!("unreadable reply {line:?}: {error}"))?;
                // A pane shows once the first window is up: the browser, or the
                // portal backend's first chooser.
                if let Some(capture) = &mut self.capture
                    && reply["app"]["panes"]
                        .as_array()
                        .is_some_and(|panes| !panes.is_empty())
                {
                    capture.ready();
                }
                Ok(reply)
            }
            Err(RecvTimeoutError::Timeout) => {
                Err(format!("no reply within {} s", timeout.as_secs()))
            }
            Err(RecvTimeoutError::Disconnected) => {
                // Its output closes a moment before the exit can be reaped.
                let status = wait_for_exit(&mut self.child, EXIT_TIMEOUT)
                    .map_or_else(|| "still running".to_owned(), |status| status.to_string());
                Err(format!(
                    "strata exited ({status}); log tail:\n{}",
                    tail(&self.log)
                ))
            }
        }
    }

    fn send(&mut self, command: &Value, timeout: Duration) -> Result<Value, String> {
        let stdin = self.stdin.as_mut().ok_or("stdin is closed")?;
        writeln!(stdin, "{command}")
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("cannot send {command}: {error}"))?;
        self.receive(timeout)
    }

    fn terminate_group(&mut self) {
        self.keep_log();
        let group = rustix::process::Pid::from_child(&self.child);
        for signal in [rustix::process::Signal::TERM, rustix::process::Signal::KILL] {
            let _ = rustix::process::kill_process_group(group, signal);
            if wait_for_exit(&mut self.child, TERMINATE_GRACE).is_some() {
                return;
            }
        }
        let _ = self.child.wait();
    }

    fn stop(&mut self) {
        drop(self.stdin.take());
        if wait_for_exit(&mut self.child, EXIT_TIMEOUT).is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.keep_log();
    }

    fn keep_log(&mut self) {
        if let Some(capture) = &mut self.capture {
            capture.keep_log(&self.log);
        }
    }
}

impl Drop for Application {
    fn drop(&mut self) {
        self.stop();
    }
}

fn execute(scenario: &Scenario, run: &Run, manifest: &Manifest) -> Outcome {
    let scratch = tempfile::Builder::new()
        .prefix("strata-qa-")
        .tempdir_in("/tmp")
        .expect("the run's scratch directory should be created");
    let fixed_name = scenario
        .header
        .fixture_root
        .as_deref()
        .and_then(fixed_fixture_name);
    // Held until the run's fixture is gone.
    let _fixed_claim = fixed_name.map(|_| {
        FIXED_FIXTURES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    });
    let mut fixture = tempfile::Builder::new();
    let fixture = match fixed_name {
        Some(name) => fixture.prefix(name).rand_bytes(0),
        None => fixture.prefix("strata-qa-fixture-"),
    }
    .tempdir_in("/tmp");
    // Another run's directory, or anything else already there, is never reused.
    let fixture = match fixture {
        Ok(fixture) => fixture,
        Err(error) => {
            return Outcome {
                errors: vec![format!(
                    "setup: cannot create the fixture directory: {error}"
                )],
                ..Outcome::default()
            };
        }
    };
    let artifacts = Path::new(REPOSITORY)
        .join("target/qa-artifacts")
        .join(&scenario.header.scenario)
        .join(&run.label);
    let _ = fs::remove_dir_all(&artifacts);
    let setup_error = |error: String| Outcome {
        errors: vec![error],
        ..Outcome::default()
    };

    let home = scratch.path().join("home");
    let runtime = scratch.path().join("runtime");
    let places = Places {
        fixture: fixture.path(),
        scratch_home: &home,
    };
    let steps = match run_steps(scenario, run, &variables(run, &places)) {
        Ok(steps) => steps,
        Err(error) => return setup_error(format!("scenario: {error}")),
    };
    let mut fs_state = FsState {
        roots: vec![
            fixture.path().to_owned(),
            home.clone(),
            scratch.path().to_owned(),
        ],
        ..FsState::default()
    };
    let teardown = |fs_state: &mut FsState| -> Vec<String> {
        scenario
            .header
            .teardown
            .iter()
            .chain(&run.variant.teardown)
            .filter_map(|step| {
                let step = unescape(&substitute(step, &variables(run, &places)));
                wait_for_fs(&step, fixture.path(), fixture_time(manifest), fs_state)
                    .err()
                    .map(|error| format!("teardown: {error}"))
            })
            .collect()
    };
    let failed_setup = |error: String, fs_state: &mut FsState| {
        let mut outcome = setup_error(error);
        outcome.errors.extend(teardown(fs_state));
        outcome
    };
    if let Err(error) = seed(manifest, scenario, run, &home, &runtime, fixture.path()) {
        return failed_setup(format!("setup: {error}"), &mut fs_state);
    }
    for step in scenario.header.setup.iter().chain(&run.variant.setup) {
        let step = unescape(&substitute(step, &variables(run, &places)));
        if let Err(error) =
            wait_for_fs(&step, fixture.path(), fixture_time(manifest), &mut fs_state)
        {
            return failed_setup(format!("setup: {error}"), &mut fs_state);
        }
    }
    let audit = match main_io::Audit::for_run(&scenario.header.scenario, &run.label) {
        Ok(audit) => audit,
        Err(error) => return failed_setup(format!("setup: {error}"), &mut fs_state),
    };
    let display = match Display::start(scratch.path(), &runtime) {
        Ok(display) => display,
        Err(error) => return failed_setup(format!("setup: {error}"), &mut fs_state),
    };
    let bus = if scenario.header.session_bus {
        match Bus::start(&runtime) {
            Ok(bus) => Some(bus),
            Err(error) => return failed_setup(format!("setup: {error}"), &mut fs_state),
        }
    } else {
        None
    };
    let input_method = match (&scenario.header.input_method, &bus) {
        (Some(name), Some(bus)) => {
            match InputMethod::start(name, manifest, &home, &runtime, &display, &bus.address) {
                Ok(input_method) => Some(input_method),
                Err(error) => return failed_setup(format!("setup: {error}"), &mut fs_state),
            }
        }
        _ => None,
    };
    let launch = Launch {
        manifest,
        home: &home,
        fixture: fixture.path(),
        display: &display,
        runtime: &runtime,
        log: scratch.path().join("strata.log"),
        input_log: &scratch.path().join(INPUT_LOG),
        extra: scenario
            .header
            .env
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    unescape(&Value::String(substitute_text(
                        value,
                        &variables(run, &places),
                    )))
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                )
            })
            .collect(),
        bus: bus.as_ref().map(|bus| bus.address.as_str()),
        portal: scenario.header.portal,
        audit: audit.as_ref(),
    };
    let mut application = match launch.start() {
        Ok(application) => application,
        Err(error) => return failed_setup(format!("launch: {error}"), &mut fs_state),
    };

    let mut layout = DEFAULT_LAYOUT.to_owned();
    let mut peers = Peers::default();
    let mut outcome = drive(
        &mut application,
        &mut peers,
        &Context {
            steps: &steps,
            initial_layout: run
                .variant
                .layout
                .as_deref()
                .unwrap_or(&scenario.header.layout),
            manifest,
            display: &display,
            fixture: fixture.path(),
            fixture_time: fixture_time(manifest),
            screens: &scratch.path().join("screenshots"),
            artifacts: &artifacts,
            launch: &launch,
        },
        &mut layout,
        &mut fs_state,
    );
    outcome.errors.extend(teardown(&mut fs_state));
    if layout != DEFAULT_LAYOUT
        && let Err(error) = display.restore_layout()
    {
        outcome.errors.push(format!("teardown: {error}"));
    }
    let failed = !outcome.errors.is_empty() || !outcome.failures.is_empty();
    let collect =
        failed || std::env::var_os("STRATA_QA_KEEP_ARTIFACTS").is_some_and(|value| value == "1");
    let mut final_state = Value::Null;
    if collect && application.responsive {
        final_state = settled_state(&mut application);
    }
    // Peers end only once the final state is read.
    drop(peers);
    if collect {
        if outcome.last_tree.is_none()
            && application.responsive
            && let Ok(reply) = application.send(&json!({ "query": "tree" }), COMMAND_TIMEOUT)
        {
            outcome.last_tree = reply["result"].as_str().map(str::to_owned);
        }
        let _ = fs::create_dir_all(&artifacts);
        if let Err(error) = display.screenshot(&artifacts.join("screenshot.png")) {
            outcome.errors.push(format!("artifacts: {error}"));
        }
    }
    application.stop();
    if collect {
        let mut files = vec![
            ("transcript.jsonl", std::mem::take(&mut outcome.transcript)),
            ("fixture.txt", listing(fixture.path())),
            (
                "strata.log",
                fs::read_to_string(&application.log).unwrap_or_default(),
            ),
            (
                display
                    .log
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("display.log"),
                fs::read_to_string(&display.log).unwrap_or_default(),
            ),
            (
                INPUT_LOG,
                fs::read_to_string(scratch.path().join(INPUT_LOG)).unwrap_or_default(),
            ),
            (STATE_FILE, pretty(&final_state)),
            (
                RESULT_FILE,
                pretty(&run_result(scenario, run, &steps, &outcome)),
            ),
        ];
        if failed {
            let all: Vec<String> = outcome
                .errors
                .iter()
                .chain(outcome.failure_texts())
                .cloned()
                .collect();
            files.push(("failures.txt", all.join("\n")));
        }
        if let Some(tree) = outcome.last_tree.take() {
            files.push(("tree.txt", tree));
        }
        if let Some(input_method) = &input_method {
            files.push((
                input_method::LOG,
                fs::read_to_string(&input_method.log).unwrap_or_default(),
            ));
        }
        for (file, contents) in files {
            let _ = fs::write(artifacts.join(file), contents);
        }
        for screen in fs::read_dir(scratch.path().join("screenshots"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let _ = fs::copy(screen.path(), artifacts.join(screen.file_name()));
        }
        if failed {
            outcome.artifacts = Some(artifacts);
        }
    }
    outcome
}

fn seed(
    manifest: &Manifest,
    scenario: &Scenario,
    run: &Run,
    home: &Path,
    runtime: &Path,
    fixture: &Path,
) -> Result<(), String> {
    let io = |error: std::io::Error| error.to_string();
    for (_, directory) in XDG_DIRECTORIES {
        fs::create_dir_all(home.join(directory)).map_err(io)?;
    }
    fs::create_dir_all(runtime).map_err(io)?;
    fs::set_permissions(runtime, fs::Permissions::from_mode(0o700)).map_err(io)?;
    let gtk = home.join("config/gtk-4.0");
    fs::create_dir_all(&gtk).map_err(io)?;
    fs::write(
        gtk.join("settings.ini"),
        gtk_settings(&manifest.gtk_settings, &scenario.header.gtk_settings)?,
    )
    .map_err(io)?;
    let strata = home.join("config/strata");
    fs::create_dir_all(&strata).map_err(io)?;
    // Pre-answers the first-run portal offer, which would cover every scenario.
    fs::write(strata.join(&manifest.portal_offer_marker), "1\n").map_err(io)?;

    let mut preferences = manifest.preferences.clone();
    for (key, value) in scenario.header.prefs.iter().chain(run.variant.prefs.iter()) {
        preferences.update(key, value.clone());
    }
    preferences.update("browser_mode", Value::String(run.mode.clone()));
    fs::write(strata.join("settings.toml"), render_toml(&preferences)).map_err(io)?;

    let tree = expand_tree(
        run.variant
            .tree
            .as_ref()
            .or(scenario.header.tree.as_ref())
            .unwrap_or(&Value::String(DEFAULT_TREE.to_owned())),
        manifest,
    )?;
    populate(fixture, &tree);
    pin_times(fixture, fixture_time(manifest));
    for (path, image) in &scenario.header.images {
        image::RgbImage::from_pixel(image.width, image.height, image::Rgb(image.rgb))
            .save_with_format(fixture.join(path), image::ImageFormat::Png)
            .map_err(|error| format!("cannot write {path}: {error}"))?;
    }
    Ok(())
}
