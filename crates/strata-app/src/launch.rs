// SPDX-License-Identifier: MIT

//! The command line, and the windows a launch opens.

#[cfg(any(test, feature = "qa"))]
use std::ffi::OsStr;
use std::{env, ffi::OsString, path::PathBuf, process::ExitCode};

use strata_core::model::Location;
use strata_desktop::tabs_session::{self, RestoredSession};
use strata_fs::adapters::list_archive_entries_direct;
#[cfg(any(test, feature = "qa"))]
use strata_fs::adapters::location_for_file;
use strata_platform::single_instance::{Activation, Claim, Launch};
use strata_preview::sandbox_helper;

use crate::{
    application::{self, Opening},
    host,
    settings::Saved,
    startup,
};

const USAGE: &str = "usage: strata-rune [PATH...|--portal]";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    /// Open a window for each path, or the saved session or default folder without one. The
    /// running instance opens them when there is one.
    Open(Vec<OsString>),
    /// Run the XDG Desktop Portal `FileChooser` D-Bus service.
    Portal,
    Help,
}

/// Byte-safe: an argument that is not UTF-8 is a path, never an option.
fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut paths = Vec::new();
    let mut portals = 0_usize;
    let mut options_ended = false;
    for argument in arguments {
        let option = (!options_ended)
            .then(|| argument.to_str())
            .flatten()
            .filter(|text| text.starts_with('-') && *text != "-");
        match option {
            Some("--") => options_ended = true,
            Some("--help" | "-h") => return Ok(Command::Help),
            Some("--portal") => portals += 1,
            Some(unknown) => return Err(format!("unknown option {unknown}")),
            None => paths.push(argument),
        }
    }
    match (portals, paths.is_empty()) {
        (0, _) => Ok(Command::Open(paths)),
        (1, true) => Ok(Command::Portal),
        _ => Err("--portal cannot be combined with a directory".to_owned()),
    }
}

/// The location an argument names, resolved as GIO resolves one for the GTK application: a
/// path, relative to the launch folder or absolute, or a URI.
#[cfg(any(test, feature = "qa"))]
pub(crate) fn location(argument: Option<&OsStr>) -> Option<Location> {
    match argument {
        Some(argument) => location_for_file(&gio::File::for_commandline_arg(argument)),
        None => Some(Location::local(home_directory())),
    }
}

pub(crate) fn home_directory() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The launch folder may sit on a volume the user later wants to unmount.
pub(crate) fn leave_launch_folder() {
    if let Err(error) = env::set_current_dir("/") {
        tracing::warn!(%error, "unable to leave the launch folder");
    }
}

/// What a window opens (`docs/rune/shared-state.md`, "Tab sessions").
pub(crate) enum Initial {
    /// A test's or QA run's folder: opens that folder in one tab and leaves `tabs.toml` alone.
    Explicit(Location),
    /// A plain launch (`strata-rune` with no argument): restores the saved session when
    /// `Some`, or opens the default folder, and persists this window's tabs.
    Plain(Option<RestoredSession>),
    /// A later launch with no argument, as GTK's `present` opens it: the saved session, read
    /// off the interface thread once the window is up, or the default folder, and this
    /// window's tabs persisted.
    Present,
    /// An argument: a folder opens, a file shows selected in its folder, and `tabs.toml` is
    /// left alone (`crate::application::open_argument`).
    Argument(Location),
}

/// What a test launch opens: a plain launch without `directory`, that folder with one.
#[cfg(test)]
pub(crate) fn initial(directory: Option<&OsStr>, settings: &Saved) -> Result<Initial, String> {
    match directory {
        Some(argument) => location(Some(argument))
            .map(Initial::Explicit)
            .ok_or_else(|| "the argument is not a location".to_owned()),
        None => Ok(initial_for(Opening::Plain, settings)),
    }
}

/// The window `opening` asks for, reading `tabs.toml` only for a plain launch with
/// `restore_tabs` on: at start-up, before its window shows.
pub(crate) fn initial_for(opening: Opening, settings: &Saved) -> Initial {
    match opening {
        Opening::Plain => {
            let restored = settings
                .preferences()
                .restore_tabs
                .then(tabs_session::load_restorable)
                .flatten();
            Initial::Plain(restored)
        }
        Opening::Present => Initial::Present,
        Opening::Argument(location) => Initial::Argument(location),
    }
}

/// Claims the app id for this launch, or hands it to the instance that holds it and returns
/// that one's answer. The primary opens a window for each of its own arguments, as for a
/// launch handed to it later.
fn open(arguments: Vec<OsString>) -> Result<i32, String> {
    let launch = Launch {
        args: arguments,
        cwd: env::current_dir().ok(),
        activation: Activation::from_env(),
    };
    let (primary, unclaimed) = match application::claim(launch, false) {
        Claim::Forwarded(status) => return Ok(status),
        Claim::Primary(primary) => (primary, None),
        Claim::Standalone(primary, why) => (primary, Some(why)),
    };
    let settings = startup::initialize_process();
    if let Some(why) = unclaimed {
        tracing::debug!(?why, "running alone, not as the session's one instance");
    }
    leave_launch_folder();
    let ui = startup::UiThread::claim()?;
    application::attach(primary);
    let mut openings = application::take().into_iter();
    // As GTK's `present_open` skips an argument that is no location, and the application then
    // ends with no window and status 0.
    let Some(first) = openings.next() else {
        return Ok(0);
    };
    application::queue_first(openings);
    let initial = initial_for(first, &settings);
    host::windowed::run(ui, settings, initial)?;
    Ok(0)
}

fn portal() -> Result<(), String> {
    let settings = startup::initialize_process();
    leave_launch_folder();
    host::portal::run(startup::UiThread::claim()?, settings)
}

/// The argument the sandbox's worker pool runs this executable with
/// (`strata_preview::sandbox::browser`), as the GTK application's `main` reads it.
const BROWSER_WORKER: &str = "--browser-worker";

/// Whether this process is a thumbnail worker the sandbox started: the first argument, and
/// only the first, names it, as `launch_mode` in `src/main.rs` reads it.
fn is_browser_worker(arguments: &[OsString]) -> bool {
    arguments
        .get(1)
        .is_some_and(|argument| argument == BROWSER_WORKER)
}

/// Serves the sandbox's thumbnail requests on stdin, as the GTK application's
/// `LaunchMode::BrowserWorker` does. The confinement is `strata-preview`'s, the same for both
/// binaries: the pool starts this executable under bubblewrap, bound read-only at
/// `/app/strata`, and the worker restricts itself (`sandbox::browser::worker::run`).
fn browser_worker() -> ExitCode {
    match strata_preview::sandbox::browser::run(sandbox_helper::browser_render) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Browser helper failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The argument `strata_preview::sandbox` runs this executable with inside bubblewrap for
/// one-shot preview rendering (office documents, PDFs, archives, media, 3D models).
const PREVIEW_HELPER: &str = "--preview-helper";

/// Whether this process is a sandboxed preview helper: the first argument, and only the first,
/// names it, as `launch_mode` in `src/main.rs` reads it.
fn is_preview_helper(arguments: &[OsString]) -> bool {
    arguments
        .get(1)
        .is_some_and(|argument| argument == PREVIEW_HELPER)
}

fn run_preview_helper(arguments: &[OsString]) -> Result<(), String> {
    let arguments = arguments
        .iter()
        .map(|argument| {
            argument
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| "Invalid UTF-8 in preview helper arguments".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    sandbox_helper::install_archive_lister(list_archive_entries_direct);
    sandbox_helper::run(&arguments)
}

fn preview_helper(arguments: &[OsString]) -> ExitCode {
    match run_preview_helper(arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Preview helper failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs `strata-rune` with the process's arguments, the program name first.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> ExitCode {
    let arguments: Vec<OsString> = arguments.into_iter().collect();
    // Before anything else starts: a worker sets up no logging, settings or window.
    if is_browser_worker(&arguments) {
        return browser_worker();
    }
    if is_preview_helper(&arguments) {
        return preview_helper(&arguments[2..]);
    }
    #[cfg(feature = "qa")]
    if let Some(code) = crate::qa::launch(&arguments) {
        return code;
    }
    match parse(arguments.iter().skip(1).cloned()) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Portal) => match portal() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("strata-rune: {error}");
                ExitCode::FAILURE
            }
        },
        Ok(Command::Open(_)) => match open(arguments) {
            Ok(status) => ExitCode::from(u8::try_from(status).unwrap_or(1)),
            Err(error) => {
                eprintln!("strata-rune: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("strata-rune: {error}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests;
