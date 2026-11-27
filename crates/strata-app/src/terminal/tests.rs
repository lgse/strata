// SPDX-License-Identifier: MIT

use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use rune::{Key, KeyEvent, Mods, Tx};
use strata_components::{contract::Rect, dialog::Tone};
use strata_core::model::Location;
use strata_kit::icons::IconName;

use super::{
    Emulator,
    launcher::{PREFERRED_LAUNCHER, spawn_reaped_receiver},
    open_focused_folder,
};
use crate::{
    App,
    commands::{self, Holds, On, Outcome, Situation},
    error_dialog::{self, Error},
    host::offscreen::Offscreen,
    menus::{self, Request},
    shell,
    startup::{self, UiThread},
    test_support::{PATIENCE, fixture, in_own_process, send_key},
};

const MODULE: &str = "terminal::tests";

fn hosted_with_env(name: &str, env: &[(&str, &OsStr)], run: impl FnOnce(Offscreen)) {
    in_own_process(&format!("{MODULE}::{name}"), env, || {
        let settings = startup::load_settings();
        let ui = UiThread::claim().expect("the UI thread");
        run(Offscreen::new(ui, settings));
    });
}

fn write_stub_launcher(dir: &Path, log: &Path) {
    let launcher = dir.join(PREFERRED_LAUNCHER);
    let script = format!(
        "#!/bin/sh\nprintf '%s\\t%s\\n' \"$(pwd)\" \"$*\" >> '{}'\nexit 0\n",
        log.display()
    );
    fs::write(&launcher, script).expect("write stub launcher");
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).expect("chmod launcher");
}

fn launches(log: &Path) -> Vec<(PathBuf, String)> {
    let Ok(contents) = fs::read_to_string(log) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let (dir, args) = line.split_once('\t')?;
            Some((PathBuf::from(dir), args.to_owned()))
        })
        .collect()
}

fn open_and_list(host: &mut Offscreen, folder: &Path, expected_entries: usize) {
    host.app().open(Location::local(folder));
    let listed = |host: &Offscreen| {
        host.app()
            .browser()
            .column_entry_counts(0)
            .is_some_and(|counts| counts.total == expected_entries)
    };
    assert!(host.run_until(PATIENCE, listed), "folder listed");
    while host.turn() {}
}

fn press_ctrl_alt_t(host: &mut Offscreen) {
    send_key(
        host,
        KeyEvent {
            key: Key::Command('t'),
            mods: Mods {
                ctrl: true,
                cmd: true,
                alt: true,
                ..Mods::default()
            },
        },
    );
    while host.turn() {}
}

fn click_row(host: &mut Offscreen, folder: &Path, name: &str) {
    let id = shell::id_of(host.app(), &Location::local(folder.join(name)));
    let row = strata_components::collection::element::row_key(id);
    while host.turn() {}
    let [x, y, w, h] = host
        .runtime()
        .elements()
        .into_iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(row.as_str()))
        .expect("row element")
        .visible;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(cx, cy);
    host.runtime().pointer_down(cx, cy);
    host.turn();
    host.runtime().pointer_up(cx, cy);
    while host.turn() {}
}

#[test]
fn spawn_reaped_starts_children_off_the_ui_thread_and_reaps_them() {
    let (started_on_ui_thread, pid, missing) = std::thread::spawn(|| {
        strata_core::util::ui_thread::mark();
        let (sender, receiver) = mpsc::channel();
        let exited = spawn_reaped_receiver(move || {
            let on_ui_thread = strata_core::util::ui_thread::is_current();
            let child = Command::new("/bin/sh")
                .args(["-c", "exit 0"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| error.to_string())?;
            let _ = sender.send((on_ui_thread, child.id()));
            Ok(child)
        })
        .recv()
        .expect("reaper answer");
        assert_eq!(exited, Ok(()));
        let (started_on_ui_thread, pid) = receiver.recv().expect("started child");
        let missing = spawn_reaped_receiver(|| {
            Command::new("/nonexistent/strata-terminal")
                .spawn()
                .map_err(|error| format!("{:?}", error.kind()))
        })
        .recv()
        .expect("missing answer");
        (started_on_ui_thread, pid, missing)
    })
    .join()
    .expect("UI thread");
    assert!(!started_on_ui_thread, "children start on the reaper thread");
    assert_eq!(missing, Err("NotFound".to_owned()));

    let parent = format!("PPid:\t{}\n", std::process::id());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) else {
            return;
        };
        if !status.contains(&parent) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "child {pid} was not reaped; status:\n{status}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
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

        let terminal = Emulator::resolve_with(
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

fn assert_command_and_focused_launches(
    host: &mut Offscreen,
    folder: &Path,
    documents: &Path,
    log: &Path,
) {
    let pictures = folder.join("pictures");
    let todo = folder.join("todo.txt");
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let file_id = shell::id_of(app, &Location::local(&todo));
        assert_eq!(
            commands::run(app, "terminal", On::Row((0, file_id)), tx),
            Outcome::Declined
        );
        let dir_id = shell::id_of(app, &Location::local(&pictures));
        assert_eq!(
            commands::run(app, "terminal", On::Row((0, dir_id)), tx),
            Outcome::Ran
        );
    });
    assert!(
        host.run_until(PATIENCE, |_| launches(log).len() == 4),
        "entry row launch"
    );
    assert_eq!(launches(log)[3].0, pictures);

    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        assert_eq!(
            commands::run(app, "terminal", On::Column(0), tx),
            Outcome::Ran
        );
    });
    assert!(
        host.run_until(PATIENCE, |_| launches(log).len() == 5),
        "column launch"
    );
    assert_eq!(launches(log)[4].0, folder);

    host.runtime().update(|app: &mut App, _: &mut Tx| {
        assert!(open_focused_folder(app));
    });
    assert!(
        host.run_until(PATIENCE, |_| launches(log).len() == 6),
        "focused-folder launch"
    );
    assert_eq!(launches(log)[5].0, documents);
}

#[test]
fn ctrl_alt_t_and_menu_rows_open_the_targeted_or_opened_folder() {
    let launcher_dir = tempfile::tempdir().expect("launcher dir");
    let parent_log = launcher_dir.path().join("launches.log");
    write_stub_launcher(launcher_dir.path(), &parent_log);
    hosted_with_env(
        "ctrl_alt_t_and_menu_rows_open_the_targeted_or_opened_folder",
        &[
            ("PATH", launcher_dir.path().as_os_str()),
            ("TERMINAL", OsStr::new("")),
        ],
        |mut host| {
            let log = PathBuf::from(std::env::var_os("PATH").expect("PATH")).join("launches.log");
            let folder = fixture();
            open_and_list(&mut host, folder.path(), 5);

            press_ctrl_alt_t(&mut host);
            assert!(
                host.run_until(PATIENCE, |_| launches(&log).len() == 1),
                "initial selected folder launch"
            );
            assert_eq!(launches(&log)[0].0, folder.path().join("archive"));

            click_row(&mut host, folder.path(), "todo.txt");
            press_ctrl_alt_t(&mut host);
            assert!(
                host.run_until(PATIENCE, |_| launches(&log).len() == 2),
                "root folder launch"
            );
            assert_eq!(launches(&log)[1].0, folder.path());

            let documents = folder.path().join("documents");
            click_row(&mut host, folder.path(), "documents");
            assert!(
                host.run_until(PATIENCE, |host| {
                    host.app().browser().location_at(1).as_ref()
                        == Some(&Location::local(&documents))
                }),
                "documents column open"
            );
            press_ctrl_alt_t(&mut host);
            assert!(
                host.run_until(PATIENCE, |_| launches(&log).len() == 3),
                "opened child folder launch"
            );
            assert_eq!(launches(&log)[2].0, documents);

            assert_command_and_focused_launches(&mut host, folder.path(), &documents, &log);
        },
    );
}

#[test]
fn missing_emulator_and_unsupported_places_report_terminal_errors() {
    let empty_dir = tempfile::tempdir().expect("empty PATH dir");
    hosted_with_env(
        "missing_emulator_and_unsupported_places_report_terminal_errors",
        &[
            ("PATH", empty_dir.path().as_os_str()),
            ("TERMINAL", OsStr::new("")),
        ],
        |mut host| {
            let folder = fixture();
            open_and_list(&mut host, folder.path(), 5);

            press_ctrl_alt_t(&mut host);
            let reported = host.run_until(PATIENCE, |host| {
                error_dialog::shown_details(host.app()).is_some()
            });
            assert!(reported, "missing terminal error dialog");
            let (_, tone, title, _, detail) =
                error_dialog::shown_details(host.app()).expect("shown");
            assert_eq!(tone, Tone::Danger);
            assert_eq!(title, "Unable to open terminal");
            assert!(detail.contains("No terminal emulator"), "{detail}");

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert!(
                error_dialog::shown_details(host.app()).is_none(),
                "Escape dismisses the prior error dialog"
            );

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                commands::run(
                    app,
                    "terminal",
                    On::Place(Location::uri("sftp://example.com/work")),
                    tx,
                );
            });
            while host.turn() {}
            let (icon, _, title, _, message) =
                error_dialog::shown_details(host.app()).expect("place error");
            assert_eq!(icon, IconName::Close);
            assert_eq!(title, "Unable to open terminal");
            assert_eq!(message, "This location is not a local folder");
        },
    );
}

fn current_holds(host: &mut Offscreen) -> Option<Holds> {
    let mut holds = None;
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        holds = Situation::of(app, tx).holds();
    });
    holds
}

fn assert_location_and_modal_after_yield(host: &mut Offscreen, folder: &Path, log: &Path) {
    send_key(
        host,
        KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        },
    );
    while host.turn() {}
    host.runtime().update(|app: &mut App, _: &mut Tx| {
        app.columns.hold_header(None);
    });

    send_key(
        host,
        KeyEvent {
            key: Key::Command('l'),
            mods: Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            },
        },
    );
    while host.turn() {}
    assert_eq!(current_holds(host), Some(Holds::Location));

    host.runtime().update(|app: &mut App, _: &mut Tx| {
        shell::saved(app).change(|settings| settings.tenxer_mode = true);
    });
    press_ctrl_alt_t(host);
    host.runtime().update(|app: &mut App, _: &mut Tx| {
        shell::saved(app).change(|settings| settings.tenxer_mode = false);
    });
    while host.turn() {}
    assert_eq!(current_holds(host), Some(Holds::Location));

    press_ctrl_alt_t(host);
    assert!(
        host.run_until(PATIENCE, |_| !launches(log).is_empty()),
        "AfterYield launches while location bar holds focus"
    );
    assert_eq!(launches(log)[0].0, folder.join("documents"));

    host.runtime().update(|app: &mut App, _: &mut Tx| {
        error_dialog::show(app, Error::new("Busy", "Modal open"));
    });
    while host.turn() {}
    assert!(matches!(current_holds(host), Some(Holds::Modal { .. })));

    press_ctrl_alt_t(host);
    while host.turn() {}
    assert_eq!(
        launches(log).len(),
        1,
        "Reach::Taken swallows Ctrl+Alt+T under a modal dialog"
    );
}

#[test]
fn after_yield_opens_under_location_and_skips_under_menu_tenxer_or_modal() {
    let launcher_dir = tempfile::tempdir().expect("launcher dir");
    let parent_log = launcher_dir.path().join("launches.log");
    write_stub_launcher(launcher_dir.path(), &parent_log);
    hosted_with_env(
        "after_yield_opens_under_location_and_skips_under_menu_tenxer_or_modal",
        &[
            ("PATH", launcher_dir.path().as_os_str()),
            ("TERMINAL", OsStr::new("")),
        ],
        |mut host| {
            let log = PathBuf::from(std::env::var_os("PATH").expect("PATH")).join("launches.log");
            let folder = fixture();
            open_and_list(&mut host, folder.path(), 5);

            let documents = folder.path().join("documents");
            click_row(&mut host, folder.path(), "documents");
            assert!(
                host.run_until(PATIENCE, |host| {
                    host.app().browser().location_at(1).as_ref()
                        == Some(&Location::local(&documents))
                }),
                "documents column open"
            );

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                app.columns.clear_selection();
                shell::publish(app);
                app.columns.hold_header(Some(1));
                menus::open(
                    app,
                    Request {
                        depth: 0,
                        entry: None,
                        anchor: Rect::new(10.0, 10.0, 0.0, 0.0),
                    },
                    tx,
                );
            });
            while host.turn() {}
            assert_eq!(current_holds(&mut host), Some(Holds::Menu));

            press_ctrl_alt_t(&mut host);
            while host.turn() {}
            assert_eq!(
                current_holds(&mut host),
                Some(Holds::Menu),
                "the menu stays open"
            );

            assert_location_and_modal_after_yield(&mut host, folder.path(), &log);
        },
    );
}
