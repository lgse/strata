// SPDX-License-Identifier: MIT

//! Running a program: "Run this program?", the confirmation GTK asks before it starts an
//! executable file (`confirm_run_program` in `src/ui/browser/desktop.rs`), and the start.
//!
//! - **What asks.** Opening a regular executable file: a double click, Enter or a filter
//!   result opened, which the browser raises as `OpenRequested`. Its target is read through
//!   links, as GIO's `query_info` reads it, and must be a local path. And the entry menu's
//!   Run row, shown for one regular executable entry (`entry_is_regular_executable`).
//! - **The dialog.** GTK's message dialog on C15's [`StrataDialog`]: the Terminal icon in a
//!   danger box, "Run this program?", the file's name under it, and why it asks. Cancel and
//!   Run, with Run focused when it opens. Escape, a click outside, the close button and
//!   Cancel run nothing. Return presses the focused button only. The keyboard goes back to the
//!   listing when it closes.
//! - **The start** (`run_program`). The program starts in its own folder, with no input and
//!   its output dropped, under the environment Strata was started with
//!   (`child_environment::restore_command`). It starts on a thread of its own, which then
//!   waits for it, so it never lingers as a zombie (`spawn_reaped`). If it cannot start,
//!   "Unable to run program" says why, once the thread has answered.
//!
//! Left out, with what brings each back:
//!
//! - **Opening with an application first.** GTK asks only once its lookup finds no default
//!   application for the file's type (`resolve_default_application`); here every executable
//!   asks. The lookup, and opening every other file, are A14's
//!   (`docs/rune/parity/packages/A14.md`).
//! - **A shell script (`.sh`)**, which GTK runs in a terminal (`program_command`): the
//!   terminal is A14's. Here Run on one starts nothing and says "Unable to run program",
//!   "Unable to open terminal", until A14 replaces it with the terminal.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
};

use rune::Tx;
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::{
    i18n::tr,
    model::{EntryKind, FileEntry, Location, MetadataValue},
};
use strata_fs::app::BrowserEvent;
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::{Action, Call, Holds, On, Outcome},
    error_dialog::{self, Error},
    shell,
    slots::{At, Frame, Later, Listener, Part, Place, Setup},
};

/// The program a confirmation asks about.
struct Asked {
    path: PathBuf,
    name: String,
    /// Told apart from the confirmation before, so each takes its own first focus.
    generation: u64,
}

pub(crate) struct Run {
    later: Later<App>,
    /// The confirmation, kept after it closes for its closing motion.
    asked: Option<Asked>,
    open: bool,
    opened: u64,
}

impl Part for Run {
    const PLACE: Place<App> = Place::Dialogs(build);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            asked: None,
            open: false,
            opened: 0,
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if let BrowserEvent::OpenRequested { location } = event
                && let Some(path) = executable(location)
            {
                let name = location.display_name();
                heard.then(move |app: &mut App, _: &mut Tx| {
                    confirm(app, path, name);
                    true
                });
            }
        }))
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.run.open.then_some(Holds::Modal { field: false })
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("run", run_row)];
}

/// The local path of a regular executable file at `location`, read through links.
fn executable(location: &Location) -> Option<PathBuf> {
    let path = location.native_path()?;
    let metadata = std::fs::metadata(path).ok()?;
    is_executable_file(&metadata).then(|| path.to_path_buf())
}

fn is_executable_file(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

/// `entry_is_regular_executable`: what the menu's Run row is shown for.
pub(crate) fn runs(entry: &FileEntry) -> bool {
    entry.location.native_path().is_some()
        && matches!(entry.kind, EntryKind::File | EntryKind::FileSymbolicLink)
        && matches!(entry.mode, MetadataValue::Known(mode) if mode & 0o111 != 0)
}

/// The Run row: the menu's one entry, when it runs.
fn run_row(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let On::Row(at) = call.on else {
        return Outcome::Declined;
    };
    let Some(entry) = row_entry(app, at).filter(runs) else {
        return Outcome::Declined;
    };
    let Some(path) = entry.location.native_path().map(Path::to_path_buf) else {
        return Outcome::Declined;
    };
    confirm(app, path, entry.location.display_name());
    Outcome::Ran
}

fn row_entry(app: &App, (depth, id): At) -> Option<FileEntry> {
    shell::shown_view(app)
        .and_then(|view| (view.rows.position)(app, (depth, id)))
        .and_then(|position| app.browser.entry_at(depth, position))
}

/// Opens the confirmation for the program at `path`, in place of one already open.
fn confirm(app: &mut App, path: PathBuf, name: String) {
    let run = &mut app.run;
    run.opened += 1;
    run.asked = Some(Asked {
        path,
        name,
        generation: run.opened,
    });
    run.open = true;
}

/// Cancel, Escape, the close button and a click outside.
fn cancelled(app: &mut App) {
    app.run.open = false;
}

/// Run: the dialog closes, and the program starts.
fn confirmed(app: &mut App) {
    if !app.run.open {
        return;
    }
    app.run.open = false;
    let Some(path) = app.run.asked.as_ref().map(|asked| asked.path.clone()) else {
        return;
    };
    if is_shell_script(&path) {
        tracing::warn!(
            path = %path.display(),
            "a shell script runs in a terminal, which this interface does not open yet"
        );
        failed(app, tr("Unable to open terminal"));
        return;
    }
    let later = app.run.later.clone();
    if let Err(error) = start(&path, later) {
        failed(app, error.to_string());
    }
}

/// "Unable to run program", with why.
fn failed(app: &mut App, detail: String) {
    tracing::warn!(%detail, "unable to run program");
    error_dialog::show(app, Error::new(tr("Unable to run program"), detail));
}

/// `program_command` and `spawn_reaped`: the program starts on a thread of its own, which
/// then waits for it. Nothing runs when the thread cannot start; a start that fails on it
/// comes back through GLib, and `later` shows it.
fn start(path: &Path, later: Later<App>) -> std::io::Result<()> {
    let mut command = program_command(path);
    let (answer, answered) = mpsc::channel();
    std::thread::Builder::new()
        .name("process-reaper".into())
        .spawn(move || match command.spawn() {
            Ok(mut child) => {
                let _ = answer.send(Ok(()));
                if let Err(error) = child.wait() {
                    tracing::warn!(%error, pid = child.id(), "unable to reap child process");
                }
            }
            Err(error) => {
                let _ = answer.send(Err(error.to_string()));
            }
        })?;
    glib::MainContext::default().spawn_local(async move {
        let started = gio::spawn_blocking(move || answered.recv()).await;
        if let Ok(Ok(Err(detail))) = started {
            later.then(move |app: &mut App, _: &mut Tx| {
                failed(app, detail);
                true
            });
        }
    });
    Ok(())
}

fn is_shell_script(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "sh")
}

fn program_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command
        .current_dir(path.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    strata_fs::child_environment::restore_command(&mut command);
    command
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<rune::El> {
    let run = &app.run;
    let asked = run.asked.as_ref()?;
    let message = rust_i18n::t!(
        "“%{name}” is an executable file. Only run programs you trust.",
        name = asked.name
    );
    let mut dialog = StrataDialog::new(
        format!("run-confirm-{}", asked.generation),
        tr("Run this program?"),
        cancelled,
    )
    .open(run.open)
    .subtitle(asked.name.clone())
    .icon(IconName::Terminal)
    .tone(Tone::Danger)
    .message(message.into_owned())
    .secondary(tr("Cancel"), cancelled)
    .primary(tr("Run"), confirmed)
    .initial_focus(Focus::Primary)
    // GTK's Return presses the focused button only: with nothing focused it runs nothing.
    .default_button(false)
    .close_label(tr("Close dialog"))
    .width(MESSAGE_WIDTH);
    if let Some(view) = shell::view_focus(app) {
        dialog = dialog.return_focus(view);
    }
    dialog.build(&app.look, frame.layer);
    None
}

#[cfg(test)]
mod tests;
