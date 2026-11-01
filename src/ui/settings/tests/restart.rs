// SPDX-License-Identifier: MIT

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};

use super::super::{
    remove_unless_replaced, restart, restart_waiter, restart_with, schedule_rollback_cleanup_with,
};
use crate::test_support::gtk_test;

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

#[test]
fn rollback_cleanup_removes_only_the_preserved_copy_and_off_the_ui_thread() {
    gtk_test(
        "ui::settings::tests::restart::rollback_cleanup_removes_only_the_preserved_copy_and_off_the_ui_thread",
        || {
            let context = glib::MainContext::default();
            let settle_until = |what: &str, until: &dyn Fn() -> bool| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !until() {
                    assert!(Instant::now() < deadline, "{what}");
                    context.iteration(false);
                    std::thread::sleep(Duration::from_millis(2));
                }
            };
            for replaced_later in [false, true] {
                let directory = tempfile::tempdir().expect("install fixture");
                let rollback = crate::services::rollback_path(directory.path());
                fs::write(&rollback, "previous").expect("preserved previous version");
                let steps_on_ui_thread = Arc::new(Mutex::new(Vec::new()));
                let (located, removing, path) = (
                    steps_on_ui_thread.clone(),
                    steps_on_ui_thread.clone(),
                    rollback.clone(),
                );
                schedule_rollback_cleanup_with(
                    move || {
                        let on_ui_thread = crate::util::ui_thread::is_current();
                        located.lock().expect("steps").push(on_ui_thread);
                        Some(path)
                    },
                    Duration::from_millis(300),
                    move |rollback, preserved| {
                        let on_ui_thread = crate::util::ui_thread::is_current();
                        removing.lock().expect("steps").push(on_ui_thread);
                        remove_unless_replaced(rollback, preserved);
                    },
                );
                let steps = || steps_on_ui_thread.lock().expect("steps").clone();
                settle_until("the rollback copy is located", &|| !steps().is_empty());
                assert_eq!(steps(), [false], "the start-up check runs on a worker");
                if replaced_later {
                    let checked = Instant::now();
                    settle_until("the preserved copy is recorded", &|| {
                        checked.elapsed() > Duration::from_millis(100)
                    });
                    let staged = directory.path().join("staged");
                    fs::write(&staged, "newer").expect("a later install's rollback copy");
                    fs::rename(&staged, &rollback).expect("replace the preserved copy");
                    settle_until("the retention passes", &|| {
                        checked.elapsed() > Duration::from_millis(900)
                    });
                    assert_eq!(
                        fs::read_to_string(&rollback).expect("later rollback copy"),
                        "newer",
                        "a later install's rollback copy is kept"
                    );
                } else {
                    settle_until("the preserved copy is removed", &|| !rollback.exists());
                }
                settle_until("the identity check runs", &|| steps().len() == 2);
                assert_eq!(
                    steps(),
                    [false, false],
                    "the identity check and unlink run on a worker"
                );
            }
        },
    );
}

#[test]
fn restart_starts_its_waiter_off_the_ui_thread_and_stops_it_when_a_window_stays_open() {
    gtk_test(
        "ui::settings::tests::restart::restart_starts_its_waiter_off_the_ui_thread_and_stops_it_when_a_window_stays_open",
        || {
            let ui = std::thread::current().id();
            let context = glib::MainContext::default();
            for window_stays_open in [true, false] {
                let application = gtk::Application::builder()
                    .application_id(if window_stays_open {
                        "org.strata.RestartWaiterTest.Refused"
                    } else {
                        "org.strata.RestartWaiterTest.Closed"
                    })
                    .flags(gio::ApplicationFlags::NON_UNIQUE)
                    .build();
                application
                    .register(None::<&gio::Cancellable>)
                    .expect("register isolated application");
                let window = gtk::ApplicationWindow::builder()
                    .application(&application)
                    .build();
                if window_stays_open {
                    window.connect_close_request(|_| glib::Propagation::Stop);
                }
                window.present();
                let entered = Arc::new(AtomicUsize::new(0));
                let started = Arc::new(Mutex::new(Vec::new()));
                for _ in 0..2 {
                    let (entered, record) = (entered.clone(), started.clone());
                    restart_with(Some(&application), move || {
                        entered.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(200));
                        let waiter = crate::trusted_command::command("sleep")
                            .expect("sleep")
                            .arg("30")
                            .spawn()
                            .expect("stand-in waiter");
                        record
                            .lock()
                            .expect("start")
                            .push((std::thread::current().id(), waiter.id()));
                        Some(waiter)
                    });
                }
                let deadline = Instant::now() + Duration::from_secs(10);
                let running = |pid: u32| std::path::Path::new(&format!("/proc/{pid}")).exists();
                let settled = || {
                    let started = started.lock().expect("start");
                    !started.is_empty()
                        && started.len() == entered.load(Ordering::SeqCst)
                        && if window_stays_open {
                            started.iter().all(|&(_, pid)| !running(pid))
                        } else {
                            application.windows().is_empty()
                        }
                };
                while !settled() {
                    assert!(
                        Instant::now() < deadline,
                        "window stays open: {window_stays_open}"
                    );
                    if !context.iteration(false) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                let started = started.lock().expect("start").clone();
                assert_eq!(
                    started.len(),
                    1,
                    "a second restart while one is starting starts no waiter, window stays open: {window_stays_open}"
                );
                let (thread, pid) = started[0];
                assert_ne!(thread, ui, "window stays open: {window_stays_open}");
                assert_eq!(application.windows().len(), usize::from(window_stays_open));
                if !window_stays_open {
                    assert!(
                        running(pid),
                        "the waiter outlives a restart that goes ahead"
                    );
                    let pid = i32::try_from(pid)
                        .ok()
                        .and_then(rustix::process::Pid::from_raw)
                        .expect("waiter pid");
                    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
                }
                window.destroy();
            }
        },
    );
}
