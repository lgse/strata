// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::ErrorKind;
use std::process::{Command, Stdio};

pub(super) const LAUNCHER: &str = "xdg-terminal-exec";

pub(super) fn command() -> Command {
    let mut command = Command::new(LAUNCHER);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub(super) fn launch_failure(error: &std::io::Error) -> String {
    if error.kind() == ErrorKind::NotFound {
        return format!("Terminal launcher “{LAUNCHER}” was not found on your PATH");
    }
    format!("Terminal launcher “{LAUNCHER}” could not be started: {error}")
}

#[cfg(test)]
mod tests;
