// SPDX-License-Identifier: MIT

use super::*;
use std::os::unix::ffi::OsStrExt;

fn bin_dir(names: &[&str]) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("fixture bin dir");
    for name in names {
        let path = dir.path().join(name);
        std::fs::write(&path, b"#!/bin/sh\n").expect("fixture executable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("fixture permissions");
    }
    dir
}

fn path_var(dir: &tempfile::TempDir) -> Option<OsString> {
    Some(dir.path().as_os_str().to_owned())
}

fn resolve_only(names: &[&str]) -> Terminal {
    let dir = bin_dir(names);
    Terminal::resolve_with(path_var(&dir).as_deref(), None).expect("terminal resolves")
}

#[test]
fn the_preferred_launcher_wins_when_it_is_installed() {
    let terminal = resolve_only(&["kitty", PREFERRED_LAUNCHER]);

    assert_eq!(
        Path::new(terminal.program()).file_name(),
        Some(OsStr::new(PREFERRED_LAUNCHER))
    );
}

#[test]
fn resolution_falls_back_to_an_installed_emulator() {
    let terminal = resolve_only(&["kitty"]);

    assert_eq!(
        Path::new(terminal.program()).file_name(),
        Some(OsStr::new("kitty"))
    );
}

#[test]
fn resolution_reports_no_terminal_when_nothing_is_installed() {
    let dir = bin_dir(&[]);

    assert_eq!(
        Terminal::resolve_with(path_var(&dir).as_deref(), None),
        None
    );
    assert!(
        no_terminal_message().contains(PREFERRED_LAUNCHER),
        "the guidance should name the preferred launcher"
    );
}

#[test]
fn an_explicit_terminal_takes_priority_over_the_path() {
    let dir = bin_dir(&[PREFERRED_LAUNCHER]);
    let terminal = Terminal::resolve_with(path_var(&dir).as_deref(), Some(OsStr::new("my-term")))
        .expect("explicit terminal resolves");

    assert_eq!(terminal.program(), OsStr::new("my-term"));
}

#[test]
fn an_explicit_terminal_keeps_known_emulator_flags() {
    let terminal = Terminal::resolve_with(None, Some(OsStr::new("kitty")))
        .expect("explicit terminal resolves");
    let command = terminal.directory_command(Path::new("/tmp/work"));

    assert_eq!(
        Path::new(terminal.program()).file_name(),
        Some(OsStr::new("kitty"))
    );
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["--working-directory", "/tmp/work"]
    );
}

#[test]
fn an_unknown_explicit_terminal_falls_back_to_the_xterm_convention() {
    let terminal = Terminal::resolve_with(None, Some(OsStr::new("my-term")))
        .expect("explicit terminal resolves");
    let command = terminal.exec_command(&["htop"]);

    assert_eq!(command.get_args().collect::<Vec<_>>(), ["-e", "htop"]);
}

#[test]
fn directory_commands_use_per_terminal_arguments() {
    let cases: &[(&str, &[&str])] = &[
        (PREFERRED_LAUNCHER, &["--dir=/tmp/work"]),
        ("kitty", &["--working-directory", "/tmp/work"]),
        ("konsole", &["--workdir", "/tmp/work"]),
        ("wezterm", &["start", "--cwd", "/tmp/work"]),
    ];
    for (program, expected) in cases.iter().copied() {
        let terminal = resolve_only(&[program]);
        let command = terminal.directory_command(Path::new("/tmp/work"));

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            expected,
            "unexpected directory arguments for {program}"
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/tmp/work")));
    }
}

#[test]
fn directory_commands_preserve_native_path_bytes() {
    let terminal = resolve_only(&["kitty"]);
    let path = Path::new(OsStr::from_bytes(b"/tmp/non-utf8-\xff"));
    let command = terminal.directory_command(path);

    assert_eq!(
        command.get_args().collect::<Vec<_>>()[1].as_encoded_bytes(),
        b"/tmp/non-utf8-\xff"
    );
    assert_eq!(command.get_current_dir(), Some(path));
}

#[test]
fn exec_commands_use_per_terminal_separators() {
    let cases: &[(&str, &[&str])] = &[
        (PREFERRED_LAUNCHER, &["--", "paru", "-Syu", "strata-bin"]),
        ("kitty", &["--", "paru", "-Syu", "strata-bin"]),
        ("konsole", &["-e", "paru", "-Syu", "strata-bin"]),
        ("foot", &["paru", "-Syu", "strata-bin"]),
        ("xfce4-terminal", &["-x", "paru", "-Syu", "strata-bin"]),
        ("terminator", &["-x", "paru", "-Syu", "strata-bin"]),
    ];
    for (program, expected) in cases.iter().copied() {
        let terminal = resolve_only(&[program]);
        let command = terminal.exec_command(&["paru", "-Syu", "strata-bin"]);

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            expected,
            "unexpected exec arguments for {program}"
        );
    }
}

#[test]
fn a_missing_explicit_terminal_is_named_in_the_failure() {
    let terminal = Terminal::resolve_with(None, Some(OsStr::new("my-term")))
        .expect("explicit terminal resolves");
    let message = terminal.launch_failure(&std::io::Error::from(ErrorKind::NotFound));

    assert_eq!(message, "Terminal “my-term” was not found on your PATH");
}

#[test]
fn other_launch_failures_name_the_terminal_and_keep_the_cause() {
    let terminal = resolve_only(&["kitty"]);
    let error = std::io::Error::from(ErrorKind::PermissionDenied);
    let message = terminal.launch_failure(&error);

    assert!(
        message.starts_with(&format!(
            "Terminal “{}” could not be started: ",
            terminal.program().to_string_lossy()
        )),
        "unexpected message: {message}"
    );
    assert!(
        message.ends_with(&error.to_string()),
        "the cause is dropped: {message}"
    );
}

#[test]
fn explicit_wezterm_preserves_subcommand_and_quoted_arguments() {
    let terminal =
        Terminal::resolve_with(None, Some(OsStr::new("wezterm --class 'Strata update'")))
            .expect("explicit terminal resolves");
    assert_eq!(
        terminal
            .exec_command(&["paru", "-Syu", "strata-bin"])
            .get_args()
            .collect::<Vec<_>>(),
        [
            "start",
            "--class",
            "Strata update",
            "--",
            "paru",
            "-Syu",
            "strata-bin"
        ]
    );
}

#[test]
fn resolved_launcher_still_runs_after_changing_directory() {
    let dir = bin_dir(&["kitty"]);
    let destination = tempfile::tempdir().expect("destination");
    let terminal =
        Terminal::resolve_with(path_var(&dir).as_deref(), None).expect("terminal resolves");
    let status = terminal
        .directory_command(destination.path())
        .env("PATH", "")
        .status()
        .expect("launch resolved executable");
    assert!(status.success());
}
