//! The interfaces the primary serves: GApplication's (`org.gtk.Application`,
//! `org.freedesktop.Application`, `org.gtk.Actions`) at the app id's object path, and, opted
//! into, `org.freedesktop.FileManager1`.
//!
//! The signatures are GLib's (`gio/gapplicationimpl-dbus.c`), so a GApplication of the same id
//! forwards to this primary and this crate's forwarder reaches a GApplication primary.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc::unbounded_channel;
use zbus::message::Header;
use zbus::names::OwnedUniqueName;
use zbus::wire::StructureBuilder;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Signature, Variant, fdo, interface};

use super::event::{CommandLine, Event, Out, Reveal, RevealMethod};
use super::launch::{Activation, Launch, PlatformData};
use super::primary::{InFlight, Inbox};

/// The forwarder's object for a launch's output, and GLib's interface for it.
pub(super) const CALLBACK_PATH: &str = "/org/gtk/Application/CommandLine";
pub(super) const CALLBACK_INTERFACE: &str = "org.gtk.private.CommandLine";
pub(super) const FILE_MANAGER1: &str = "org.freedesktop.FileManager1";
pub(super) const FILE_MANAGER1_PATH: &str = "/org/freedesktop/FileManager1";

pub(super) struct GtkApplication {
    pub inbox: Arc<Inbox>,
    pub in_flight: Arc<InFlight>,
}

#[interface(name = "org.gtk.Application")]
impl GtkApplication {
    async fn activate(&self, platform_data: PlatformData) {
        self.inbox
            .deliver(Event::Activate(Activation::from_platform_data(
                &platform_data,
            )));
    }

    async fn open(&self, uris: Vec<String>, hint: String, platform_data: PlatformData) {
        self.inbox.deliver(Event::Open {
            uris,
            hint,
            activation: Activation::from_platform_data(&platform_data),
        });
    }

    /// A forwarded launch. The reply, its exit status, waits until the app is done with the
    /// [`CommandLine`]; what it prints meanwhile goes to the caller's `path` first, in order.
    ///
    /// Once the primary is giving the app id up, a launch the app hasn't answered yet (and any
    /// that arrives) fails with `Disconnected`, which the caller reads as a primary gone: it
    /// claims the app id again. One already answered is sent to the end.
    async fn command_line(
        &self,
        path: OwnedObjectPath,
        arguments: Vec<Vec<u8>>,
        platform_data: PlatformData,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> fdo::Result<i32> {
        let gone = || fdo::Error::Disconnected("the primary is giving the app id up".into());
        let Some(_answering) = self.in_flight.enter() else {
            return Err(gone());
        };
        let caller: Option<OwnedUniqueName> = header.sender().map(|s| s.to_owned().into());
        let (tx, mut rx) = unbounded_channel();
        let launch = Launch::from_wire(arguments, &platform_data);
        self.inbox
            .deliver(Event::CommandLine(CommandLine::remote(launch, tx)));
        loop {
            let out = tokio::select! {
                biased;
                out = rx.recv() => out,
                () = self.in_flight.stopped() => match rx.try_recv() {
                    Ok(out) => Some(out),
                    Err(_) => return Err(gone()),
                },
            };
            let (method, text) = match out {
                Some(Out::Exit(status)) => return Ok(status),
                // Dropping a `CommandLine` always sends `Exit` first.
                None => return Ok(0),
                Some(Out::Print(text)) => ("Print", text),
                Some(Out::PrintErr(text)) => ("PrintError", text),
            };
            // Each line is answered before the next goes, so the caller sees them in order
            // and before the exit status. A caller that went away just misses them.
            let _ = conn
                .call_method(
                    caller.as_ref(),
                    &path,
                    Some(CALLBACK_INTERFACE),
                    method,
                    &(text,),
                )
                .await;
        }
    }
}

pub(super) struct FreedesktopApplication {
    pub inbox: Arc<Inbox>,
    pub actions: Arc<[String]>,
}

#[interface(name = "org.freedesktop.Application")]
impl FreedesktopApplication {
    async fn activate(&self, platform_data: PlatformData) {
        self.inbox
            .deliver(Event::Activate(Activation::from_platform_data(
                &platform_data,
            )));
    }

    async fn open(&self, uris: Vec<String>, platform_data: PlatformData) {
        self.inbox.deliver(Event::Open {
            uris,
            hint: String::new(),
            activation: Activation::from_platform_data(&platform_data),
        });
    }

    async fn activate_action(
        &self,
        action_name: String,
        parameter: Vec<OwnedVariant>,
        platform_data: PlatformData,
    ) -> fdo::Result<()> {
        let event = action(
            &self.actions,
            action_name,
            parameter,
            &platform_data,
            Pack::Tuple,
        )?;
        self.inbox.deliver(event);
        Ok(())
    }
}

/// GLib's action group interface. A GApplication forwarder lists the primary's actions first
/// (`DescribeAll`) and gives up if that fails, so the primary serves the interface. `List` and
/// `DescribeAll` return nothing even when `Options::actions` accepts some: a listing needs
/// each action's parameter type, which the app doesn't give. `Activate` still runs the
/// accepted ones.
pub(super) struct GtkActions {
    pub inbox: Arc<Inbox>,
    pub actions: Arc<[String]>,
}

#[interface(name = "org.gtk.Actions")]
impl GtkActions {
    fn list(&self) -> Vec<String> {
        Vec::new()
    }

    fn describe(&self, action_name: String) -> fdo::Result<(bool, Signature, Vec<OwnedVariant>)> {
        Err(fdo::Error::InvalidArgs(format!(
            "the app exports no action {action_name:?}"
        )))
    }

    fn describe_all(&self) -> HashMap<String, (bool, Signature, Vec<OwnedVariant>)> {
        HashMap::new()
    }

    async fn activate(
        &self,
        action_name: String,
        parameter: Vec<OwnedVariant>,
        platform_data: PlatformData,
    ) -> fdo::Result<()> {
        let event = action(
            &self.actions,
            action_name,
            parameter,
            &platform_data,
            Pack::First,
        )?;
        self.inbox.deliver(event);
        Ok(())
    }
}

/// How an action call's parameter array (`av`) becomes one parameter, as GLib reads each
/// interface's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pack {
    /// `org.freedesktop.Application.ActivateAction`: several pack into one tuple.
    Tuple,
    /// `org.gtk.Actions.Activate`: the first, the rest ignored (GLib's callers send at most one).
    First,
}

/// An action call as GLib reads one: refused unless the app accepts the name, its parameters
/// read as `pack` says.
pub(super) fn action(
    actions: &[String],
    name: String,
    parameter: Vec<OwnedVariant>,
    platform_data: &PlatformData,
    pack: Pack,
) -> fdo::Result<Event> {
    if !actions.contains(&name) {
        return Err(fdo::Error::InvalidArgs(format!("Unknown action ‘{name}’")));
    }
    let parameter = match (parameter.len(), pack) {
        (0 | 1, _) | (_, Pack::First) => parameter.into_iter().next(),
        _ => {
            let tuple = parameter
                .into_iter()
                .fold(StructureBuilder::new(), |b, p| {
                    b.append_field(Variant::from(p))
                })
                .build()
                .and_then(OwnedVariant::try_from)
                .map_err(|e| fdo::Error::InvalidArgs(e.to_string()))?;
            Some(tuple)
        }
    };
    Ok(Event::Action {
        name,
        parameter,
        activation: Activation::from_platform_data(platform_data),
    })
}

pub(super) struct FileManager1 {
    pub inbox: Arc<Inbox>,
}

impl FileManager1 {
    fn reveal(&self, method: RevealMethod, uris: Vec<String>, startup_id: String) {
        self.inbox.deliver(Event::Reveal(Reveal {
            method,
            uris,
            startup_id,
        }));
    }
}

#[interface(name = "org.freedesktop.FileManager1")]
impl FileManager1 {
    pub(super) async fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        self.reveal(RevealMethod::Folders, uris, startup_id);
    }

    pub(super) async fn show_items(&self, uris: Vec<String>, startup_id: String) {
        self.reveal(RevealMethod::Items, uris, startup_id);
    }

    pub(super) async fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        self.reveal(RevealMethod::ItemProperties, uris, startup_id);
    }
}
