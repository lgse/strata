// SPDX-License-Identifier: MIT

use crate::adapters::gio_file_for_location;
use crate::app::Browser;
use crate::model::{FileEntry, Location};
use crate::ui::browser::paths::is_trash_location;
use crate::ui::controls::{ModalTone, message_dialog_description, message_dialog_layout};
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog};
use crate::ui::terminal;
use gtk::gio;
use gtk::prelude::*;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::{Rc, Weak};

pub(in crate::ui) fn open_location(
    location: &Location,
    parent: &impl IsA<gtk::Widget>,
    browser: &Rc<Browser>,
) {
    if is_trash_location(location) {
        show_error_dialog(
            parent,
            "Unable to open item",
            "Items in Trash cannot be opened",
        );
        return;
    }
    let file = gio_file_for_location(location);
    let parent = parent.as_ref().downgrade();
    let location = location.clone();
    let browser = Rc::downgrade(browser);
    glib::MainContext::default().spawn_local(async move {
        match resolve_default_application(&file).await {
            Ok((_content_type, Some(app))) => {
                let result = crate::ui::open_with::launch(
                    &app,
                    std::slice::from_ref(&file),
                    None::<&gio::AppLaunchContext>,
                );
                if let Some(parent) = parent.upgrade() {
                    report_open_result(&location, &parent, result);
                }
            }
            Ok((content_type, None)) => {
                let Some(parent) = parent.upgrade() else {
                    return;
                };
                if file.is_native() && location.native_path().is_some_and(is_regular_executable) {
                    confirm_run_program(&location, &parent);
                } else {
                    show_open_with_fallback(&parent, file, &content_type, browser);
                }
            }
            Err(error) => {
                if let Some(parent) = parent.upgrade() {
                    report_open_result(&location, &parent, Err(error));
                }
            }
        }
    });
}

async fn resolve_default_application(
    file: &gio::File,
) -> Result<(String, Option<gio::AppInfo>), glib::Error> {
    let info = file
        .query_info_future(
            "standard::type,standard::content-type",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    if info.file_type() == gio::FileType::SymbolicLink {
        return Err(glib::Error::new(
            gio::IOErrorEnum::Failed,
            "Broken symbolic links cannot be opened with an application",
        ));
    }
    let content_type = info
        .content_type()
        .map(|value| value.to_string())
        .ok_or_else(|| {
            glib::Error::new(
                gio::IOErrorEnum::Failed,
                "Unable to determine the selected file type",
            )
        })?;
    let requires_uris = crate::ui::open_with::requires_uri_handlers(std::slice::from_ref(file));
    let default = gio::AppInfo::default_for_type(&content_type, requires_uris);
    Ok((content_type, default))
}

#[derive(Default)]
pub(super) struct OpenWithRequest(Option<glib::JoinHandle<()>>);

impl OpenWithRequest {
    pub(super) fn cancel(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

impl Drop for OpenWithRequest {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) fn show_open_with_for_entries(state: &Rc<super::ViewState>, entries: Vec<FileEntry>) {
    let files: Vec<_> = entries
        .iter()
        .map(|entry| gio_file_for_location(&entry.location))
        .collect();
    show_open_with_when_ready(state, entries, async move {
        crate::ui::open_with::applications_for_files(&files).await
    });
}

fn show_open_with_when_ready(
    state: &Rc<super::ViewState>,
    entries: Vec<FileEntry>,
    choices: impl std::future::Future<
        Output = Result<crate::ui::open_with::ApplicationChoices, glib::Error>,
    > + 'static,
) {
    state.open_with_request.borrow_mut().cancel();
    if entries.is_empty() {
        return;
    }
    let Some(window) = state.overlay.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let focus = gtk::prelude::GtkWindowExt::focus(&window).map(|focus| focus.downgrade());
    let window = window.downgrade();
    let navigation = state.browser.navigation_generation();
    let weak = Rc::downgrade(state);
    let task = glib::MainContext::default().spawn_local(async move {
        let choices = choices.await;
        let Some(state) = weak.upgrade() else {
            return;
        };
        state.open_with_request.borrow_mut().0.take();
        let Some(window) = window.upgrade() else {
            return;
        };
        let view = super::BrowserView {
            state: state.clone(),
        };
        let current = view.open_with_entries();
        if !window.is_mapped()
            || !crate::ui::preferences::PreferenceManager::shared().minimal_mode()
            || state.browser.navigation_generation() != navigation
            || focus.and_then(|focus| focus.upgrade()) != gtk::prelude::GtkWindowExt::focus(&window)
            || entries.len() != current.len()
            || !entries.iter().all(|entry| {
                current.iter().any(|candidate| {
                    super::clipboard::locations_equal(&entry.location, &candidate.location)
                })
            })
        {
            return;
        }
        match choices {
            Ok(choices) => {
                let files = entries
                    .iter()
                    .map(|entry| gio_file_for_location(&entry.location))
                    .collect();
                let browser = Rc::downgrade(&state.browser);
                crate::ui::open_with::show_prepared(
                    &state.overlay,
                    files,
                    choices,
                    Rc::new(move || {
                        if let Some(browser) = browser.upgrade() {
                            browser.focus_active();
                        }
                    }),
                );
            }
            Err(error) => show_error_dialog(
                &state.overlay,
                "Unable to open with application",
                error.message(),
            ),
        }
    });
    state.open_with_request.borrow_mut().0 = Some(task);
}

fn show_open_with_fallback(
    parent: &impl IsA<gtk::Widget>,
    file: gio::File,
    content_type: &str,
    browser: Weak<Browser>,
) {
    let requires_uris = crate::ui::open_with::requires_uri_handlers(std::slice::from_ref(&file));
    let (recommended_apps, other_apps) =
        crate::ui::open_with::categorized_apps(content_type, requires_uris);
    crate::ui::open_with::show(
        parent,
        vec![file],
        recommended_apps,
        other_apps,
        crate::ui::open_with::OpenWithContext::ActivationFallback,
        Rc::new(move || {
            if let Some(browser) = browser.upgrade() {
                browser.focus_active();
            }
        }),
    );
}

fn report_open_result(
    location: &Location,
    parent: &impl IsA<gtk::Widget>,
    result: Result<(), glib::Error>,
) {
    if let Err(error) = result {
        tracing::warn!(
            backend = %location.backend_name(),
            error_domain = ?error.domain(),
            error_code = error.code(),
            "unable to open file"
        );
        tracing::debug!(
            location = %location.diagnostic_path(),
            "file open location"
        );
        let detail = if error.matches(gio::IOErrorEnum::NotSupported)
            && gio_file_for_location(location).is_native()
        {
            "No application is registered for this file"
        } else {
            error.message()
        };
        show_error_dialog(parent, "Unable to open file", detail);
    }
}

pub(super) fn is_regular_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub(super) fn entry_is_regular_executable(entry: &FileEntry) -> bool {
    entry.location.native_path().is_some()
        && matches!(
            entry.kind,
            crate::model::EntryKind::File | crate::model::EntryKind::FileSymbolicLink
        )
        && matches!(entry.mode, crate::model::MetadataValue::Known(mode) if mode & 0o111 != 0)
}

pub(super) fn confirm_run_program(location: &Location, parent: &impl IsA<gtk::Widget>) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };
    let name = location.display_name();
    let layout = message_dialog_layout(
        crate::assets::icons::TERMINAL,
        "Run this program?",
        &name,
        "Run",
        ModalTone::Danger,
    );
    layout.body.append(&message_dialog_description(&format!(
        "\u{201c}{name}\u{201d} is an executable file. Only run programs you trust."
    )));
    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let run = layout.confirm;

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    window_overlay.add_overlay(&layer);
    let weak_cancel = cancel.downgrade();
    gtk::glib::idle_add_local_once(move || {
        if let Some(cancel) = weak_cancel.upgrade() {
            cancel.grab_focus();
        }
    });
    for button in [close, cancel] {
        let dismiss_layer = layer.clone();
        let dismiss_overlay = window_overlay.clone();
        let dismiss_root = blurred_root.clone();
        button.connect_clicked(move |_| {
            dismiss_modal_layer(&dismiss_layer, &dismiss_overlay, dismiss_root.as_ref());
        });
    }
    let run_layer = layer.clone();
    let run_overlay = window_overlay;
    let run_root = blurred_root;
    let run_location = location.clone();
    let error_parent = parent.as_ref().clone();
    run.connect_clicked(move |_| {
        dismiss_modal_layer(&run_layer, &run_overlay, run_root.as_ref());
        if let Err(error) = launch_program(&run_location) {
            tracing::warn!(%error, "unable to run program");
            show_error_dialog(&error_parent, "Unable to run program", &error.to_string());
        }
    });
}

fn launch_program(location: &Location) -> std::io::Result<()> {
    let path = location.native_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "program is not a local file",
        )
    })?;
    let mut child = program_command(path, terminal::Terminal::resolve)?.spawn()?;
    std::thread::spawn(move || {
        if let Err(error) = child.wait() {
            tracing::warn!(%error, "unable to reap program");
        }
    });
    Ok(())
}

fn program_command(
    path: &Path,
    resolve_terminal: impl FnOnce() -> Option<terminal::Terminal>,
) -> std::io::Result<Command> {
    if path.extension().is_some_and(|extension| extension == "sh") {
        let terminal = resolve_terminal().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                terminal::no_terminal_message(),
            )
        })?;
        // Pass the path as data, preserving the script's shebang and avoiding shell injection.
        let mut command = terminal.exec_command(&[
            std::ffi::OsStr::new("/bin/sh"),
            std::ffi::OsStr::new("-c"),
            std::ffi::OsStr::new(
                "cd -- \"$2\" && \"$1\"; status=$?; printf '\\nProcess exited with status %s. Press Enter to close…' \"$status\"; IFS= read -r answer; exit \"$status\"",
            ),
            std::ffi::OsStr::new("strata-run"),
            path.as_os_str(),
            path.parent().unwrap_or_else(|| Path::new(".")).as_os_str(),
        ]);
        command.current_dir(path.parent().unwrap_or_else(|| Path::new(".")));
        return Ok(command);
    }
    let mut command = Command::new(path);
    command
        .current_dir(path.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

pub(super) fn can_open_terminal(location: &Location) -> bool {
    location.native_path().is_some() && !is_trash_location(location)
}

pub(super) fn selected_terminal_location(entries: &[FileEntry]) -> Option<Location> {
    let [entry] = entries else {
        return None;
    };
    entry.is_directory().then(|| entry.location.clone())
}

pub(in crate::ui) fn launch_terminal(location: &Location, parent: &impl IsA<gtk::Widget>) {
    let Some(path) = location.native_path() else {
        show_error_dialog(
            parent,
            "Unable to open terminal",
            "This location is not a local folder",
        );
        return;
    };
    if is_trash_location(location) {
        show_error_dialog(
            parent,
            "Unable to open terminal",
            "Terminal cannot be opened in Trash",
        );
        return;
    }
    let path = path.to_path_buf();
    tracing::debug!(
        location = %location.diagnostic_path(),
        "opening terminal"
    );
    let Some(terminal) = terminal::Terminal::resolve() else {
        tracing::warn!("no terminal emulator found on PATH");
        show_error_dialog(
            parent,
            "Unable to open terminal",
            &terminal::no_terminal_message(),
        );
        return;
    };
    let program = terminal.program().to_string_lossy().into_owned();
    if let Err(error) = terminal.directory_command(&path).spawn() {
        tracing::warn!(%error, %program, "unable to launch terminal");
        show_error_dialog(
            parent,
            "Unable to open terminal",
            &terminal.launch_failure(&error),
        );
    }
}

#[cfg(test)]
mod tests;
