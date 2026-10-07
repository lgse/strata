// SPDX-License-Identifier: MIT

use std::{fs, os::unix::fs::PermissionsExt};

use super::super::{restart, restart_waiter};

const RESTART_CHILD: &str = "STRATA_TEST_RESTART_WITHOUT_APPLICATION";
const RESTART_RETURNED: &str = "restart without an application returned";

/// Runs in a child process, since a regression would exit the process
/// instead of failing an assertion.
#[test]
fn restart_without_an_application_leaves_the_process_running() {
    if std::env::var_os(RESTART_CHILD).is_some() {
        restart(None);
        println!("{RESTART_RETURNED}");
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "ui::settings::tests::restart::restart_without_an_application_leaves_the_process_running",
            "--nocapture",
        ])
        .env(RESTART_CHILD, "1")
        .output()
        .expect("run the restart child");
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(RESTART_RETURNED),
        "{output:?}"
    );
}

#[test]
fn restart_waiter_recovers_early_failure_but_preserves_success_and_late_failure() {
    for (exit_code, grace, expected, backup_remains) in [
        (1, "20", "restored", false),
        (0, "20", "updated", true),
        (1, "0", "updated", true),
    ] {
        let directory = tempfile::Builder::new()
            .prefix("strata restart ")
            .tempdir()
            .expect("restart fixture");
        let binary = directory.path().join("strata");
        let rollback = crate::services::rollback_path(directory.path());
        let marker = directory.path().join("result");
        for (path, value, code) in [(&binary, "updated", exit_code), (&rollback, "restored", 0)] {
            fs::write(
                path,
                format!(
                    "#!/bin/sh\nprintf '{value}' > \"$(dirname -- \"$0\")/result\"\nexit {code}\n"
                ),
            )
            .expect("executable fixture");
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable mode");
        }
        let command = restart_waiter(&binary, u32::MAX).expect("restart command");
        let mut arguments: Vec<_> = command
            .get_args()
            .map(std::ffi::OsStr::to_os_string)
            .collect();
        *arguments.last_mut().expect("grace period") = grace.into();
        let status = std::process::Command::new(command.get_program())
            .args(arguments)
            .status()
            .expect("run restart waiter");
        assert_eq!(status.success(), exit_code == 0 || !backup_remains);
        assert_eq!(
            fs::read_to_string(marker).expect("launched executable"),
            expected
        );
        assert_eq!(rollback.exists(), backup_remains);
        if !backup_remains {
            assert!(
                fs::read_to_string(binary)
                    .expect("restored binary")
                    .contains("restored")
            );
        }
    }
}
