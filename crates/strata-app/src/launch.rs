// SPDX-License-Identifier: MIT

//! The command line, and the location a launch opens.

use std::{
    env,
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::ExitCode,
};

use strata_core::model::Location;
use strata_desktop::tabs_session::{self, RestoredSession};
use strata_fs::adapters::{list_archive_entries_direct, location_for_file};
use strata_preview::sandbox_helper;

use crate::{host, settings::Saved, startup};

const USAGE: &str = "usage: strata-rune [DIRECTORY]";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    /// Open the directory, or the home directory without one.
    Open(Option<OsString>),
    Help,
}

/// Byte-safe: an argument that is not UTF-8 is a path, never an option.
fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut directory = None;
    let mut options_ended = false;
    for argument in arguments {
        let option = (!options_ended)
            .then(|| argument.to_str())
            .flatten()
            .filter(|text| text.starts_with('-') && *text != "-");
        match option {
            Some("--") => options_ended = true,
            Some("--help" | "-h") => return Ok(Command::Help),
            Some(unknown) => return Err(format!("unknown option {unknown}")),
            None if directory.is_some() => {
                return Err("only one directory can be opened".to_owned());
            }
            None => directory = Some(argument),
        }
    }
    Ok(Command::Open(directory))
}

/// The location an argument names, resolved as GIO resolves one for the GTK application: a
/// path, relative to the launch folder or absolute, or a URI.
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

/// What a launch opens in its first window (`docs/rune/shared-state.md`, "Tab sessions").
pub(crate) enum Initial {
    /// A directory argument, or a test's or QA run's folder: opens that folder in one tab and
    /// leaves `tabs.toml` alone.
    Explicit(Location),
    /// A plain launch (`strata-rune` with no directory argument): restores the saved session
    /// when `Some`, or opens the default folder, and persists this window's tabs.
    Plain(Option<RestoredSession>),
}

/// Resolves what a launch opens, reading `tabs.toml` at start-up before the first window shows
/// only on a plain launch with `restore_tabs` on.
pub(crate) fn initial(directory: Option<&OsStr>, settings: &Saved) -> Result<Initial, String> {
    match directory {
        Some(argument) => location(Some(argument))
            .map(Initial::Explicit)
            .ok_or_else(|| "the argument is not a location".to_owned()),
        None => {
            let restored = settings
                .preferences()
                .restore_tabs
                .then(tabs_session::load_restorable)
                .flatten();
            Ok(Initial::Plain(restored))
        }
    }
}

fn open(directory: Option<&OsStr>) -> Result<(), String> {
    let settings = startup::initialize_process();
    let initial = initial(directory, &settings)?;
    leave_launch_folder();
    host::windowed::run(startup::UiThread::claim()?, settings, initial)
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
    match parse(arguments.into_iter().skip(1)) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Open(directory)) => match open(directory.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
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
