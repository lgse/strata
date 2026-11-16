// SPDX-License-Identifier: MIT

//! Running a program: "Run this program?", the confirmation GTK asks before it starts an
//! executable file (`confirm_run_program` in `src/ui/browser/desktop.rs`), and the start.
//!
//! - **What asks.** Opening a regular local executable file when no default application is
//!   registered for its type ([`crate::open_with`] hands `BrowserEvent::OpenRequested` to
//!   [`confirm`] once `resolve_default_application` finds none), and the entry menu's Run row,
//!   shown for one regular executable entry (`entry_is_regular_executable`).
//! - **The dialog.** GTK's message dialog on C15's [`StrataDialog`]: the Terminal icon in a
//!   danger box, "Run this program?", the file's name under it, and why it asks. Cancel and
//!   Run, with Run focused when it opens. Escape, a click outside, the close button and
//!   Cancel run nothing. Return presses the focused button only. The keyboard goes back to the
//!   listing when it closes.
//! - **The start** (`run_program`). Regular binaries start in their own folder, with no input
//!   and their output dropped, under the environment Strata was started with
//!   (`child_environment::restore_command`); shell scripts (`.sh`) start in the resolved
//!   terminal emulator ([`crate::terminal`]) with an exit-status prompt (`program_command`).
//!   Each starts on a thread of its own, which then waits for it, so it never lingers as a
//!   zombie (`spawn_reaped`). If it cannot start, "Unable to run program" says why, once the
//!   thread has answered.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use rune::Tx;
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::{
    i18n::tr,
    model::{EntryKind, FileEntry, MetadataValue},
};
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::{Action, Call, Holds, On, Outcome},
    error_dialog::{self, Error},
    shell,
    slots::{At, Frame, Later, Part, Place, Setup},
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

    fn keyboard(app: &App) -> Option<Holds> {
        app.run.open.then_some(Holds::Modal { field: false })
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("run", run_row)];
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
pub(crate) fn confirm(app: &mut App, path: PathBuf, name: String) {
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
    let later = app.run.later.clone();
    start(path, later);
}

/// "Unable to run program", with why.
fn failed(app: &mut App, detail: String) {
    tracing::warn!(%detail, "unable to run program");
    error_dialog::show(app, Error::new(tr("Unable to run program"), detail));
}

/// `program_command` and `spawn_reaped`: the program (and, for `.sh` scripts, terminal
/// resolution) starts on a thread of its own, which then waits for it; a start that fails
/// comes back through `later`.
fn start(path: PathBuf, later: Later<App>) {
    crate::terminal::spawn_reaped(
        later,
        move || {
            program_command(&path, crate::terminal::Emulator::resolve)
                .and_then(|mut command| command.spawn())
                .map_err(|error| {
                    tracing::warn!(%error, "unable to run program");
                    error.to_string()
                })
        },
        failed,
    );
}

fn is_shell_script(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "sh")
}

fn program_command(
    path: &Path,
    resolve_terminal: impl FnOnce() -> Option<crate::terminal::Emulator>,
) -> std::io::Result<Command> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if is_shell_script(path) {
        let terminal = resolve_terminal().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                crate::terminal::no_terminal_message(),
            )
        })?;
        // Pass the path as data, preserving the script's shebang and avoiding shell injection.
        let prompt = exit_prompt_format();
        let mut command = terminal.exec_command(&[
            OsStr::new("/bin/sh"),
            OsStr::new("-c"),
            OsStr::new(
                "cd -- \"$2\" && \"$1\"; status=$?; printf '\\n'; printf \"$3\" \"$status\"; IFS= read -r answer; exit \"$status\"",
            ),
            OsStr::new("strata-run"),
            path.as_os_str(),
            parent.as_os_str(),
            OsStr::new(&prompt),
        ]);
        command.current_dir(parent);
        return Ok(command);
    }
    let mut command = Command::new(path);
    command
        .current_dir(parent)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    strata_fs::child_environment::restore_command(&mut command);
    Ok(command)
}

/// A printf format whose only conversion is the exit status.
fn exit_prompt_format() -> String {
    rust_i18n::t!(
        "Process exited with status %{status}. Press Enter to close…",
        status = "\u{1}"
    )
    .replace('%', "%%")
    .replace('\u{1}', "%s")
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
