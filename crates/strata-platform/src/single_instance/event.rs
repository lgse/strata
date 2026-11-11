//! What reaches the app: one [`Event`] per launch, activation or call.

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use tokio::sync::mpsc::UnboundedSender;
use zbus::OwnedVariant;

use super::launch::{Activation, Launch};

/// One thing the primary is asked to do. Everything in it comes from another process on the
/// session bus, so treat paths, URIs and arguments as untrusted input.
#[derive(Debug)]
pub enum Event {
    /// A launch: this process's own (unless `Options::service`), or a later one forwarded.
    CommandLine(CommandLine),
    /// `Activate`: bring the app forward, as a launcher does for a running app with no
    /// arguments.
    Activate(Activation),
    /// `Open`: open these URIs. `hint` is GApplication's (empty from
    /// `org.freedesktop.Application`).
    Open {
        uris: Vec<String>,
        hint: String,
        activation: Activation,
    },
    /// `ActivateAction`, or `org.gtk.Actions.Activate`: run the app's action `name`, one of
    /// `Options::actions`. As GLib reads them: several `ActivateAction` parameters arrive
    /// packed in one tuple, and `org.gtk.Actions.Activate` passes its first. `org.gtk.Actions`
    /// lists no actions, so a GApplication forwarder offers none.
    Action {
        name: String,
        parameter: Option<OwnedVariant>,
        activation: Activation,
    },
    /// An `org.freedesktop.FileManager1` call, with `Options::file_manager1`.
    Reveal(Reveal),
}

/// An `org.freedesktop.FileManager1` call: what to show, as the caller sent it. Grouping the
/// URIs into windows, selecting, mounting and showing properties are the app's policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reveal {
    pub method: RevealMethod,
    /// In the order the caller listed them.
    pub uris: Vec<String>,
    /// The caller's startup id: usually an activation token, or empty.
    pub startup_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevealMethod {
    /// `ShowFolders`: open each folder.
    Folders,
    /// `ShowItems`: open each item's folder, with the item selected.
    Items,
    /// `ShowItemProperties`: as `Items`, with the item's properties showing.
    ItemProperties,
}

/// What a forwarded launch's primary sends back, in order: lines for its stdout or stderr,
/// then its exit status.
#[derive(Debug)]
pub(super) enum Out {
    Print(String),
    PrintErr(String),
    Exit(i32),
}

#[derive(Debug)]
enum Reply {
    /// This process's own launch: prints go to its own stdout and stderr, and the status
    /// waits in `Primary::exit_status`.
    Local(Arc<AtomicI32>),
    /// A forwarded launch, waiting for its answer on the bus thread.
    Remote(UnboundedSender<Out>),
}

/// A launch's command line, and its answer. The launch's process waits until this is dropped
/// (or [`CommandLine::exit`]), then exits with the status set, 0 unless changed. Text printed
/// before that reaches its stdout or stderr first.
#[derive(Debug)]
pub struct CommandLine {
    launch: Launch,
    status: i32,
    reply: Option<Reply>,
}

impl CommandLine {
    pub(super) fn local(launch: Launch, status: Arc<AtomicI32>) -> CommandLine {
        CommandLine {
            launch,
            status: 0,
            reply: Some(Reply::Local(status)),
        }
    }

    pub(super) fn remote(launch: Launch, out: UnboundedSender<Out>) -> CommandLine {
        CommandLine {
            launch,
            status: 0,
            reply: Some(Reply::Remote(out)),
        }
    }

    /// The whole argument vector, the program first.
    pub fn arguments(&self) -> &[OsString] {
        &self.launch.args
    }

    /// The launch's working directory: resolve relative paths in the arguments against it.
    pub fn cwd(&self) -> Option<&Path> {
        self.launch.cwd()
    }

    pub fn activation(&self) -> &Activation {
        &self.launch.activation
    }

    pub fn launch(&self) -> &Launch {
        &self.launch
    }

    /// Whether another process forwarded this launch.
    pub fn is_remote(&self) -> bool {
        matches!(self.reply, Some(Reply::Remote(_)))
    }

    /// Writes `text` to the launch's stdout, as it is (add the newline). A closed stdout is
    /// ignored.
    pub fn print(&self, text: &str) {
        self.send(Out::Print(text.to_owned()));
    }

    /// Writes `text` to the launch's stderr, as [`CommandLine::print`] does to stdout.
    pub fn print_err(&self, text: &str) {
        self.send(Out::PrintErr(text.to_owned()));
    }

    pub fn exit_status(&self) -> i32 {
        self.status
    }

    pub fn set_exit_status(&mut self, status: i32) {
        self.status = status;
    }

    /// Answers the launch now, with `status`.
    pub fn exit(mut self, status: i32) {
        self.status = status;
    }

    fn send(&self, out: Out) {
        match &self.reply {
            Some(Reply::Local(_)) => match out {
                Out::Print(t) => write_ignoring_errors(&mut std::io::stdout().lock(), &t),
                Out::PrintErr(t) => write_ignoring_errors(&mut std::io::stderr().lock(), &t),
                Out::Exit(_) => {}
            },
            // A launch that has gone, or a bus thread that stopped, hears nothing.
            Some(Reply::Remote(tx)) => {
                let _ = tx.send(out);
            }
            None => {}
        }
    }
}

impl Drop for CommandLine {
    fn drop(&mut self) {
        match self.reply.take() {
            Some(Reply::Local(status)) => status.store(self.status, Ordering::Relaxed),
            Some(Reply::Remote(tx)) => {
                let _ = tx.send(Out::Exit(self.status));
            }
            None => {}
        }
    }
}

/// Writes without panicking on a closed pipe: a launch whose output nobody reads still runs.
pub(super) fn write_ignoring_errors(out: &mut impl Write, text: &str) {
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}
