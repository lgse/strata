// SPDX-License-Identifier: MIT

//! The command line, and the location a launch opens.

use std::{
    env,
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::ExitCode,
};

use strata_core::model::Location;
use strata_fs::adapters::location_for_file;

use crate::{host, startup};

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

fn open(directory: Option<&OsStr>) -> Result<(), String> {
    let settings = startup::initialize_process();
    let location = location(directory).ok_or("the argument is not a location")?;
    leave_launch_folder();
    host::windowed::run(startup::UiThread::claim()?, settings, location)
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
    match strata_preview::sandbox::browser::run(strata_preview::sandbox_helper::browser_render) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Browser helper failed: {error}");
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
