// SPDX-License-Identifier: MIT

mod argument;
mod reveal;

use std::{cell::RefCell, collections::VecDeque, ffi::OsString, path::Path};

use strata_core::model::Location;
use strata_fs::adapters::{RevealMethod, RevealRequest, location_for_file, reveal_requests};
use strata_platform::single_instance::{
    self, AppId, Claim, Event, Launch, Options, Reveal, RevealMethod as Called,
};

pub(crate) use argument::open as open_argument;
pub(crate) use reveal::open as open_reveal;

use crate::startup::APPLICATION_ID;

/// A window the process is asked to open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Opening {
    /// The process's own launch with no argument: the saved session, or the default folder.
    Plain,
    Present,
    /// An argument: a folder opens, a file is shown selected in its folder.
    Argument(Location),
    Reveal(RevealRequest),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Asked {
    pub(crate) opening: Opening,
    pub(crate) token: Option<String>,
}

thread_local! {
    /// What the bus asked for since the host last looked: on the interface thread.
    static QUEUED: RefCell<VecDeque<Asked>> = const { RefCell::new(VecDeque::new()) };
    /// The primary, kept for the life of the process and released as it quits.
    static PRIMARY: RefCell<Option<single_instance::Primary>> = const { RefCell::new(None) };
}

pub(crate) fn claim(launch: Launch, service: bool) -> Claim {
    let id = AppId::new(APPLICATION_ID).expect("Strata's application id is a valid one");
    single_instance::claim(
        &id,
        launch,
        &Options {
            service,
            file_manager1: true,
            ..Options::default()
        },
    )
}

pub(crate) fn attach(primary: single_instance::Primary) {
    primary.attach(|event| glib::MainContext::default().invoke(move || heard(event)));
    PRIMARY.with(|kept| *kept.borrow_mut() = Some(primary));
}

pub(crate) fn release() {
    if let Some(primary) = PRIMARY.with(|kept| kept.borrow_mut().take()) {
        primary.release();
    }
}

pub(crate) fn take_asked() -> Vec<Asked> {
    QUEUED.with(|queued| queued.borrow_mut().drain(..).collect())
}

/// [`take_asked`], for a host whose windows take no activation token.
pub(crate) fn take() -> Vec<Opening> {
    take_asked()
        .into_iter()
        .map(|asked| asked.opening)
        .collect()
}

pub(crate) fn queue_first(openings: impl IntoIterator<Item = Opening>) {
    let openings: Vec<Opening> = openings.into_iter().collect();
    QUEUED.with(|queued| {
        let mut queued = queued.borrow_mut();
        for opening in openings.into_iter().rev() {
            queued.push_front(Asked {
                opening,
                token: None,
            });
        }
    });
}

pub(crate) fn ask_for_window(location: Location) {
    queue_with([Opening::Argument(location)], None);
}

/// Queues `openings`, each to be created with `token`.
fn queue_with(openings: impl IntoIterator<Item = Opening>, token: Option<&str>) {
    let token = token.filter(|token| !token.is_empty()).map(str::to_owned);
    QUEUED.with(|queued| {
        queued
            .borrow_mut()
            .extend(openings.into_iter().map(|opening| Asked {
                opening,
                token: token.clone(),
            }));
    });
}

#[cfg(test)]
fn queue(openings: impl IntoIterator<Item = Opening>) {
    queue_with(openings, None);
}

fn heard(event: Event) {
    match event {
        Event::CommandLine(command) => queue_with(
            openings(command.arguments(), command.cwd(), command.is_remote()),
            command.activation().token(),
        ),
        Event::Activate(activation) => queue_with([Opening::Present], activation.token()),
        Event::Open {
            uris, activation, ..
        } => queue_with(
            uris.iter()
                .filter_map(|uri| location_for_file(&gio::File::for_uri(uri)))
                .map(Opening::Argument),
            activation.token(),
        ),
        // FileManager1's startup id is the caller's activation token, when it has one.
        Event::Reveal(reveal) => queue_with(revealed(&reveal), Some(&reveal.startup_id)),
        // No actions are exported.
        Event::Action { .. } => {}
    }
}

fn revealed(reveal: &Reveal) -> Vec<Opening> {
    let method = match reveal.method {
        Called::Folders => RevealMethod::Folders,
        Called::Items => RevealMethod::Items,
        Called::ItemProperties => RevealMethod::ItemProperties,
    };
    let requests = reveal_requests(method, &reveal.uris);
    requests.into_iter().map(Opening::Reveal).collect()
}

fn openings(arguments: &[OsString], cwd: Option<&Path>, handed_over: bool) -> Vec<Opening> {
    let paths = paths(arguments);
    if paths.is_empty() {
        return vec![if handed_over {
            Opening::Present
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

fn location_for_argument(argument: &OsString, cwd: Option<&Path>) -> Option<Location> {
    let file = match cwd {
        Some(cwd) => gio::File::for_commandline_arg_and_cwd(argument, cwd),
        None => gio::File::for_commandline_arg(argument),
    };
    location_for_file(&file)
}

#[cfg(test)]
mod tests;
