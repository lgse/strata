// SPDX-License-Identifier: MIT

//! Copyable and terminal-runnable package install command widget and reaped terminal launcher,
//! ported from `src/ui/settings/install_command.rs` and shared by Diagnostics and Integrations.

use std::sync::mpsc;

use rune::{El, Tx, row, text};
use rune_kit::{IconButton, IconButtonVariant};
use strata_core::i18n::tr;
use strata_kit::{icons::IconName, look::Look};

use crate::{
    App, buttons,
    settings_dialog::controls::{PickHandler, control_border},
    slots::WeakLater,
    terminal::{Emulator, no_terminal_message},
};

pub(super) const COPY_FEEDBACK_SECS: f64 = 2.0;

pub(super) struct CommandBoxSpec {
    pub(super) key: String,
    pub(super) run_key: String,
    pub(super) copy_key: String,
    pub(super) command: String,
    pub(super) copied: bool,
    pub(super) on_run: PickHandler,
    pub(super) on_copy: PickHandler,
}

pub(super) fn command_box(look: &Look, spec: CommandBoxSpec) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let accent_kit = look.accent_icon_kit();
    let run_label = tr("Run in terminal");
    let on_run = spec.on_run;
    let run_btn = IconButton::new(look.icon(IconName::Play))
        .variant(IconButtonVariant::Ghost)
        .label(run_label.clone())
        .on_press_tx(move |app: &mut App, tx: &mut Tx| on_run(app, tx))
        .build(&accent_kit)
        .help(run_label);
    let copy_icon = if spec.copied {
        IconName::Check
    } else {
        IconName::Copy
    };
    let copy_label = tr(if spec.copied {
        "Install command copied"
    } else {
        "Copy install command"
    });
    let on_copy = spec.on_copy;
    let copy_btn = IconButton::new(look.icon(copy_icon))
        .variant(IconButtonVariant::Ghost)
        .label(copy_label.clone())
        .on_press_tx(move |app: &mut App, tx: &mut Tx| on_copy(app, tx))
        .build(&accent_kit)
        .help(copy_label);
    row()
        .key(spec.key)
        .w_full()
        .min_w(0.0)
        .px(10.0)
        .py(6.0)
        .gap(6.0)
        .items_center()
        .radius(look.round(6.0))
        .fill(look.fill.muted)
        .border(1.0, control_border(look))
        .child(
            text(spec.command)
                .font(kit.ty.mono)
                .color(c.text_base)
                .selectable(true)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
        .child(buttons::on_return(spec.run_key, run_btn))
        .child(buttons::on_return(spec.copy_key, copy_btn))
}

/// Runs `command` in the user's resolved terminal on a reaped background thread, then delivers
/// the launch outcome back to `App` through `weak`.
pub(super) fn run_in_terminal(
    weak: WeakLater<App>,
    command: &str,
    on_done: impl FnOnce(&mut App, Result<(), String>) -> bool + 'static,
) {
    let arguments = terminal_install_arguments(command);
    spawn_reaped(
        weak,
        move || {
            let terminal = Emulator::resolve().ok_or_else(no_terminal_message)?;
            terminal
                .exec_command(&arguments)
                .spawn()
                .map_err(|error| terminal.launch_failure(&error))
        },
        on_done,
    );
}

fn spawn_reaped(
    weak: WeakLater<App>,
    start: impl FnOnce() -> Result<std::process::Child, String> + Send + 'static,
    on_done: impl FnOnce(&mut App, Result<(), String>) -> bool + 'static,
) {
    let (answer, answered) = mpsc::channel();
    let reaper = std::thread::Builder::new()
        .name("settings-install-reaper".into())
        .spawn(move || match start() {
            Ok(mut child) => {
                let _ = answer.send(Ok(()));
                if let Err(error) = child.wait() {
                    tracing::warn!(%error, pid = child.id(), "unable to reap install terminal");
                }
            }
            Err(error) => {
                let _ = answer.send(Err(error));
            }
        });
    if let Err(error) = reaper {
        tracing::warn!(%error, "unable to start install reaper thread");
    }
    glib::MainContext::default().spawn_local(async move {
        let started = gio::spawn_blocking(move || answered.recv()).await;
        let outcome = match started {
            Ok(Ok(result)) => result,
            Ok(Err(_)) | Err(_) => Err(tr(strata_fs::services::UNEXPECTED_SYSTEM_ERROR)),
        };
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| on_done(app, outcome));
    });
}

/// Keeps the terminal open after the install so its output stays readable. The prompt and command
/// words are positional arguments, never parsed by the shell.
pub(super) fn terminal_install_arguments(command: &str) -> Vec<String> {
    let mut arguments = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        r#"prompt=$1; shift; "$@"; printf '\n%s ' "$prompt"; read -r _"#.to_owned(),
        "sh".to_owned(),
        tr("Press Enter to close."),
    ];
    arguments.extend(command.split_whitespace().map(str::to_owned));
    arguments
}
