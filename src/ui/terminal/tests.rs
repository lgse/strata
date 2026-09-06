// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn a_missing_launcher_is_named_in_the_failure() {
    let message = launch_failure(&std::io::Error::from(ErrorKind::NotFound));

    assert_eq!(
        message,
        "Terminal launcher “xdg-terminal-exec” was not found on your PATH"
    );
}

#[test]
fn other_launch_failures_name_the_launcher_and_keep_the_cause() {
    let error = std::io::Error::from(ErrorKind::PermissionDenied);
    let message = launch_failure(&error);

    assert!(
        message.starts_with("Terminal launcher “xdg-terminal-exec” could not be started: "),
        "unexpected message: {message}"
    );
    assert!(
        message.ends_with(&error.to_string()),
        "the cause is dropped: {message}"
    );
}

#[test]
fn the_launcher_command_runs_the_xdg_helper() {
    assert_eq!(command().get_program(), LAUNCHER);
}
