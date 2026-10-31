// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};

use crate::{
    assets::{self, icons},
    ui::{
        accessibility,
        controls::copyable_command,
        terminal::{self, Terminal},
    },
};

#[cfg(test)]
mod tests;

/// The copyable command, with a play button that runs it in the user's
/// terminal. `finished` says what to do once the command completes.
pub(super) fn install_command(command: &str, finished: &str) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let field = copyable_command(command);
    field.add_css_class("install-command");
    container.append(&field);
    let label = crate::i18n::tr("Run in terminal");
    let run = gtk::Button::builder()
        .tooltip_text(&label)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .margin_end(32)
        .build();
    run.add_css_class("preview-command-copy");
    run.set_widget_name("run-install-command");
    run.set_has_frame(false);
    run.set_cursor_from_name(Some("pointer"));
    run.set_child(Some(&assets::primary_icon(icons::PLAY, 16)));
    accessibility::set_label(&run, &label);
    field.add_overlay(&run);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("settings-option-description");
    status.add_css_class("install-command-status");
    status.set_visible(false);
    container.append(&status);
    let arguments = terminal_install_arguments(command);
    let finished = finished.to_owned();
    run.connect_clicked(move |_| {
        let arguments = arguments.clone();
        let (status, finished) = (status.clone(), finished.clone());
        glib::MainContext::default().spawn_local(async move {
            let launched = terminal::spawn_reaped(move || {
                let terminal = Terminal::resolve().ok_or_else(terminal::no_terminal_message)?;
                terminal
                    .exec_command(&arguments)
                    .spawn()
                    .map_err(|error| terminal.launch_failure(&error))
            })
            .await;
            match launched {
                Ok(()) => {
                    status.remove_css_class("error");
                    status.set_text(&finished);
                }
                Err(message) => {
                    status.add_css_class("error");
                    status.set_text(&message);
                }
            }
            status.set_visible(true);
        });
    });
    container
}

/// Keeps the terminal open after the install so its output stays readable. The
/// prompt and command words are positional arguments, never parsed by the shell.
fn terminal_install_arguments(command: &str) -> Vec<String> {
    let mut arguments = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        r#"prompt=$1; shift; "$@"; printf '\n%s ' "$prompt"; read -r _"#.to_owned(),
        "sh".to_owned(),
        crate::i18n::tr("Press Enter to close."),
    ];
    arguments.extend(command.split_whitespace().map(str::to_owned));
    arguments
}
