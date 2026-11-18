// SPDX-License-Identifier: MIT

//! The process behind the windows: one primary instance on the session bus, which opens a
//! window for its own launch and one for each later launch handed to it, as the GTK
//! application's GApplication does (`handle_command_line` and `connect_open` in `src/main.rs`).
//!
//! The claim is strata-platform's P8 shim (`strata_platform::single_instance`), which speaks
//! GApplication's protocol on the session bus. It names this interface's own app id, so a
//! launch of it never reaches the GTK application. Rune's own `single_instance` hands a later
//! launch's `scheme://` links over, and none of its paths, its working folder or an answer: the
//! shim's module docs have the gap and its upstream issue (Rune #22, packet entry 58).
//!
//! The shim delivers its events on its bus thread. [`attach`] sends each to the interface
//! thread through GLib (`MainContext::invoke`), where it is dispatched like any other source:
//! it only queues ([`Opening`]), and the host opens what is queued after the dispatch
//! ([`take`]). A primary's own launch is delivered at [`attach`], before the first window.

mod argument;

use std::{cell::RefCell, collections::VecDeque, ffi::OsString, path::Path};

use strata_core::model::Location;
use strata_fs::adapters::location_for_file;
use strata_platform::single_instance::{self, AppId, Claim, Event, Launch, Options};

pub(crate) use argument::open as open_argument;

use crate::startup::APPLICATION_ID;

/// A window the process is asked to open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Opening {
    /// The process's own launch with no argument: the saved session, or the default folder.
    Plain,
    /// A later launch with no argument, or an activation: the default folder, in a window
    /// that keeps no session (`docs/rune/shared-state.md`, "Tab sessions").
    Fresh,
    /// An argument: a folder opens, a file is shown selected in its folder.
    Argument(Location),
}

thread_local! {
    /// What the bus asked for since the host last looked: on the interface thread.
    static QUEUED: RefCell<VecDeque<Opening>> = const { RefCell::new(VecDeque::new()) };
    /// The primary, kept for the life of the process and released as it quits.
    static PRIMARY: RefCell<Option<single_instance::Primary>> = const { RefCell::new(None) };
}

/// Claims this interface's app id for `launch`: the primary, or the status to exit with once
/// the running primary took the launch. A `service` claim delivers no launch of its own: the
/// QA mode's, whose host opens its folder itself.
pub(crate) fn claim(launch: Launch, service: bool) -> Claim {
    let id = AppId::new(APPLICATION_ID).expect("Strata's application id is a valid one");
    single_instance::claim(
        &id,
        launch,
        &Options {
            service,
            ..Options::default()
        },
    )
}

/// Keeps `primary` for the process and has its events reach the interface thread. Call it on
/// the interface thread, which owns the default GLib context: the events queued since the
/// claim, the process's own launch among them, are queued before it returns.
pub(crate) fn attach(primary: single_instance::Primary) {
    primary.attach(|event| glib::MainContext::default().invoke(move || heard(event)));
    PRIMARY.with(|kept| *kept.borrow_mut() = Some(primary));
}

/// Gives the app id up, so that a launch from now on starts a new primary rather than reach
/// this one as it quits.
pub(crate) fn release() {
    if let Some(primary) = PRIMARY.with(|kept| kept.borrow_mut().take()) {
        primary.release();
    }
}

/// The windows asked for since the last call, in the order they were asked for.
pub(crate) fn take() -> Vec<Opening> {
    QUEUED.with(|queued| queued.borrow_mut().drain(..).collect())
}

/// Puts `openings` back at the head of the queue, for the host's first pump.
pub(crate) fn queue_first(openings: impl IntoIterator<Item = Opening>) {
    let openings: Vec<Opening> = openings.into_iter().collect();
    QUEUED.with(|queued| {
        let mut queued = queued.borrow_mut();
        for opening in openings.into_iter().rev() {
            queued.push_front(opening);
        }
    });
}

fn queue(openings: impl IntoIterator<Item = Opening>) {
    QUEUED.with(|queued| queued.borrow_mut().extend(openings));
}

/// One event of the bus, on the interface thread. A launch is answered as it is dropped, with
/// status 0, as GTK answers once it has asked for its windows.
fn heard(event: Event) {
    match event {
        Event::CommandLine(command) => queue(openings(
            command.arguments(),
            command.cwd(),
            command.is_remote(),
        )),
        Event::Activate(_) => queue([Opening::Fresh]),
        Event::Open { uris, .. } => queue(
            uris.iter()
                .filter_map(|uri| location_for_file(&gio::File::for_uri(uri)))
                .map(Opening::Argument),
        ),
        // No actions are exported, and the shim serves FileManager1 only when asked to.
        Event::Action { .. } | Event::Reveal(_) => {}
    }
}

/// What a launch opens: a window for each argument that names a location, resolved against
/// the launch's working folder `cwd` as GIO resolves a command-line argument; with none, the
/// plain launch's window, or a fresh one for a launch another process handed over.
fn openings(arguments: &[OsString], cwd: Option<&Path>, handed_over: bool) -> Vec<Opening> {
    let paths = paths(arguments);
    if paths.is_empty() {
        return vec![if handed_over {
            Opening::Fresh
        } else {
            Opening::Plain
        }];
    }
    paths
        .iter()
        .filter_map(|path| location_for_argument(path, cwd))
        .map(Opening::Argument)
        .collect()
}

/// The arguments after the program that are paths: every one after `--`, and before it every
/// one that is not an option. The launch parsed its own options before it was handed over.
fn paths(arguments: &[OsString]) -> Vec<&OsString> {
    let mut ended = false;
    let mut paths = Vec::new();
    for argument in arguments.iter().skip(1) {
        if !ended && argument == "--" {
            ended = true;
            continue;
        }
        let option = !ended
            && argument
                .to_str()
                .is_some_and(|text| text.starts_with('-') && text != "-");
        if !option {
            paths.push(argument);
        }
    }
    paths
}

/// The location `argument` names from `cwd`, as `g_file_new_for_commandline_arg_and_cwd`
/// reads it: a URI, an absolute path, or a path relative to the launch's folder.
fn location_for_argument(argument: &OsString, cwd: Option<&Path>) -> Option<Location> {
    let file = match cwd {
        Some(cwd) => gio::File::for_commandline_arg_and_cwd(argument, cwd),
        None => gio::File::for_commandline_arg(argument),
    };
    location_for_file(&file)
}

#[cfg(test)]
mod tests;
