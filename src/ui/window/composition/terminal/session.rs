// SPDX-License-Identifier: MIT

use std::{os::fd::AsRawFd, path::PathBuf, rc::Rc};

use gtk::{gio, glib, prelude::*};
use vte4::prelude::*;

use super::PaneTerminal;

struct Launch {
    directory: std::fs::File,
    working_directory: String,
    shell: String,
}

impl PaneTerminal {
    pub(super) fn start(self: &Rc<Self>) {
        if self.starting.replace(true) {
            return;
        }
        self.message.set_visible(false);
        self.retry.set_visible(false);
        let path = self
            .view
            .browser()
            .active_location()
            .and_then(|location| location.native_path().map(PathBuf::from))
            .unwrap_or_else(glib::home_dir);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let launch = gio::spawn_blocking(move || prepare(path))
                .await
                .unwrap_or_else(|_| Err(std::io::ErrorKind::Interrupted.into()));
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.cancellable.is_cancelled() {
                return;
            }
            match launch {
                Ok(launch) => state.spawn(launch),
                Err(error) => state.start_failed(&error.to_string()),
            }
        });
    }

    fn spawn(self: &Rc<Self>, launch: Launch) {
        let Launch {
            directory,
            working_directory,
            shell,
        } = launch;
        let weak = Rc::downgrade(self);
        self.terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            Some(&working_directory),
            &[&shell],
            &["TERM=xterm-256color", "COLORTERM=truecolor"],
            glib::SpawnFlags::SEARCH_PATH,
            || {},
            -1,
            Some(&self.cancellable),
            move |result| {
                let Some(state) = weak.upgrade() else {
                    if let Ok(pid) = result {
                        terminate(pid);
                    }
                    return;
                };
                state.spawn_finished(result, directory);
            },
        );
    }

    fn spawn_finished(&self, result: Result<glib::Pid, glib::Error>, directory: std::fs::File) {
        if self.cancellable.is_cancelled() {
            if let Ok(pid) = result {
                terminate(pid);
            }
            return;
        }
        self.starting.set(false);
        match result {
            Ok(pid) => {
                self.directory.replace(Some(directory));
                self.pid.set(Some(pid));
            }
            Err(error) => self.start_failed(&error.to_string()),
        }
    }

    fn start_failed(&self, error: &str) {
        self.starting.set(false);
        self.message.set_text(&rust_i18n::t!(
            "Unable to start terminal: %{error}",
            error = error
        ));
        self.message.set_visible(true);
        self.retry.set_visible(true);
    }

    pub(in crate::ui::window::composition) fn has_jobs(&self) -> bool {
        self.pid.get().is_some_and(|pid| {
            let tasks_dir = format!("/proc/{}/task", pid.0);
            if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
                entries.flatten().any(|entry| {
                    std::fs::read_to_string(entry.path().join("children"))
                        .is_ok_and(|children| !children.trim().is_empty())
                })
            } else {
                std::fs::read_to_string(format!("/proc/{0}/task/{0}/children", pid.0))
                    .is_ok_and(|children| !children.trim().is_empty())
            }
        })
    }

    pub(in crate::ui::window::composition) fn shutdown(&self) {
        self.cancellable.cancel();
        if let Some(pid) = self.pid.take() {
            terminate(pid);
        }
        self.directory.borrow_mut().take();
    }
}

fn prepare(path: PathBuf) -> std::io::Result<Launch> {
    let directory = std::fs::File::open(&path)?;
    // VTE's working-directory API accepts UTF-8 only. A held directory FD
    // supplies a stable alias for native paths with other bytes.
    let working_directory = path
        .to_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("/proc/{}/fd/{}", std::process::id(), directory.as_raw_fd()));
    Ok(Launch {
        directory,
        working_directory,
        shell: shell(),
    })
}

fn shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| std::path::Path::new(shell).is_file())
        .or_else(|| {
            glib::find_program_in_path("bash").map(|path| path.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "/bin/sh".into())
}

pub(super) fn terminate(pid: glib::Pid) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid.0)
        && rustix::process::kill_process_group(pid, rustix::process::Signal::HUP).is_err()
    {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::HUP);
    }
}
