// SPDX-License-Identifier: MIT

use super::*;
use std::time::{Duration, Instant};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn spawn_reaped_starts_children_off_the_ui_thread_and_reaps_them() {
    let (started_on_ui_thread, pid, missing) = std::thread::spawn(|| {
        crate::util::ui_thread::mark();
        let (sender, receiver) = std::sync::mpsc::channel();
        let exited = futures_lite::future::block_on(spawn_reaped(move || {
            let on_ui_thread = crate::util::ui_thread::is_current();
            let child = Command::new("/bin/sh")
                .args(["-c", "exit 0"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| error.to_string())?;
            let _ = sender.send((on_ui_thread, child.id()));
            Ok(child)
        }));
        assert_eq!(exited, Ok(()));
        let (started_on_ui_thread, pid) = receiver.recv().expect("started child");
        let missing = futures_lite::future::block_on(spawn_reaped(|| {
            Command::new("/nonexistent/strata-terminal")
                .spawn()
                .map_err(|error| format!("{:?}", error.kind()))
        }));
        (started_on_ui_thread, pid, missing)
    })
    .join()
    .expect("UI thread");
    assert!(!started_on_ui_thread, "children start on the reaper thread");
    assert_eq!(missing, Err("NotFound".to_owned()));

    let parent = format!("PPid:\t{}\n", std::process::id());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
            return;
        };
        if !status.contains(&parent) {
            return;
        }
        if Instant::now() >= deadline {
            let state = status
                .lines()
                .find(|line| line.starts_with("State:"))
                .unwrap_or("State: unknown");
            panic!("child {pid} was not reaped; last {state}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn omarchy_presentation_command_runs_the_launcher_only_when_installed() {
    let directory = tempfile::tempdir().expect("PATH fixture");
    let path = Some(directory.path().as_os_str());
    assert!(omarchy_presentation_command(path, "omarchy-update").is_none());

    let launcher = directory.path().join(OMARCHY_PRESENTATION_LAUNCHER);
    fs::write(&launcher, "#!/bin/sh\n").expect("launcher");
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).expect("executable");

    let command = omarchy_presentation_command(path, "omarchy-update").expect("installed launcher");
    assert_eq!(command.get_program(), launcher.as_os_str());
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [OsStr::new("omarchy-update")]
    );
}

#[test]
fn ghostty_opens_directories_through_new_window_only_from_1_3() {
    const DIRECTORY: &str = "--working-directory=/tmp/work";
    const CLASS: &str = "--class=com.example.strata";
    let cases: [(&str, &str, &[&str], &[&str]); 6] = [
        (
            "Ghostty 1.3.1",
            "ghostty",
            &["+new-window", DIRECTORY],
            &["-e", "htop"],
        ),
        (
            "Ghostty 1.4.0-main+5c3c7a2",
            "ghostty",
            &["+new-window", DIRECTORY],
            &["-e", "htop"],
        ),
        (
            "Ghostty 1.3.1",
            "ghostty --class=com.example.strata",
            &["+new-window", CLASS, DIRECTORY],
            &[CLASS, "-e", "htop"],
        ),
        ("Ghostty 1.2.3", "ghostty", &[DIRECTORY], &["-e", "htop"]),
        ("Ghostty 1.1.3", "ghostty", &[DIRECTORY], &["-e", "htop"]),
        ("", "ghostty", &[DIRECTORY], &["-e", "htop"]),
    ];
    for (version_output, terminal_var, directory_args, exec_args) in cases {
        let directory = tempfile::tempdir().expect("PATH fixture");
        let program = directory.path().join("ghostty");
        fs::write(&program, format!("#!/bin/sh\necho '{version_output}'\n")).expect("terminal");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("executable");

        let terminal = Terminal::resolve_with(
            Some(directory.path().as_os_str()),
            Some(OsStr::new(terminal_var)),
        )
        .expect("ghostty");

        let open = terminal.directory_command(Path::new("/tmp/work"));
        assert_eq!(
            open.get_args().collect::<Vec<_>>(),
            directory_args.iter().map(OsStr::new).collect::<Vec<_>>(),
            "{version_output:?} via {terminal_var:?}"
        );
        let run = terminal.exec_command(&["htop"]);
        assert_eq!(
            run.get_args().collect::<Vec<_>>(),
            exec_args.iter().map(OsStr::new).collect::<Vec<_>>(),
            "{version_output:?} via {terminal_var:?}"
        );
    }
}
