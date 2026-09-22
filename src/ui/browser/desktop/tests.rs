// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{FileEntry, Location};
use std::path::Path;

#[test]
fn regular_executable_requires_regular_file_and_execute_bit()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempfile::tempdir()?;
    let program = fixture.path().join("program");
    std::fs::write(&program, b"#!/bin/sh\n")?;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))?;

    assert!(is_regular_executable(&program));
    assert!(!is_regular_executable(fixture.path()));

    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644))?;
    assert!(!is_regular_executable(&program));
    Ok(())
}

#[test]
fn entry_executable_policy_accepts_regular_files_and_file_links() {
    let entry = |kind, mode| FileEntry {
        location: Location::local("/fixture/program"),
        native_name: "program".into(),
        thumbnail_path: None,
        display_name: "program".into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Known(mode),
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    };

    assert!(entry_is_regular_executable(&entry(
        crate::model::EntryKind::File,
        0o755,
    )));
    assert!(entry_is_regular_executable(&entry(
        crate::model::EntryKind::FileSymbolicLink,
        0o755,
    )));
    assert!(!entry_is_regular_executable(&entry(
        crate::model::EntryKind::File,
        0o644,
    )));
    assert!(!entry_is_regular_executable(&entry(
        crate::model::EntryKind::Directory,
        0o755,
    )));

    let mut remote = entry(crate::model::EntryKind::File, 0o755);
    remote.location = Location::uri("smb://server/program");
    assert!(!entry_is_regular_executable(&remote));

    let mut unknown_mode = entry(crate::model::EntryKind::File, 0o755);
    unknown_mode.mode = crate::model::MetadataValue::Unknown;
    assert!(!entry_is_regular_executable(&unknown_mode));
}

#[test]
fn program_command_runs_from_program_directory() {
    let path = Path::new("/tmp/tools/program");
    let command = program_command(path, || panic!("non-script must launch directly"))
        .expect("direct launch command");

    assert_eq!(command.get_program(), path.as_os_str());
    assert_eq!(command.get_current_dir(), Some(Path::new("/tmp/tools")));
}

#[test]
fn shell_script_requires_a_terminal() {
    let error = program_command(Path::new("/fixture/script.sh"), || None)
        .expect_err("scripts require a terminal");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(error.to_string().contains("No terminal emulator"));
}

#[test]
fn shell_script_terminal_preserves_input_output_status_and_literal_path()
-> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempfile::tempdir()?;
    let launcher = fixture.path().join("xdg-terminal-exec");
    std::fs::write(&launcher, "#!/bin/sh\nshift\nexec \"$@\"\n")?;
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755))?;
    let path = fixture.path().join("script ' $(touch injected).sh");
    std::fs::write(
        &path,
        "#!/bin/sh\nIFS= read -r value\nprintf 'input=%s\\n' \"$value\"\nprintf 'script error\\n' >&2\npwd\nexit 7\n",
    )?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    let terminal = terminal::Terminal::resolve_with(Some(fixture.path().as_os_str()), None);
    let mut command = program_command(&path, || terminal)?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().expect("piped stdin");
    input.write_all(b"hello\n")?;
    // Read through the completion prompt before allowing the terminal to close.
    use std::io::Read;
    let mut output = child.stdout.take().expect("piped stdout");
    let mut bytes = Vec::new();
    while !bytes.ends_with("Press Enter to close…".as_bytes()) {
        let mut byte = [0];
        assert_eq!(output.read(&mut byte)?, 1);
        bytes.push(byte[0]);
    }
    assert!(child.try_wait()?.is_none());
    input.write_all(b"\n")?;
    drop(input);
    let result = child.wait_with_output()?;
    assert_eq!(result.status.code(), Some(7));
    let stdout = String::from_utf8(bytes)?;
    assert!(stdout.contains("input=hello\n"));
    assert!(stdout.contains(fixture.path().to_str().expect("UTF-8 fixture path")));
    assert!(stdout.contains("Process exited with status 7."));
    assert_eq!(String::from_utf8(result.stderr)?, "script error\n");
    assert!(!fixture.path().join("injected").exists());
    Ok(())
}

#[test]
fn terminal_shortcut_prefers_one_selected_directory() {
    let entry = |name: &str, kind| FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        thumbnail_path: None,
        display_name: name.into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    };
    let directory = entry("selected", crate::model::EntryKind::Directory);
    let file = entry("notes.txt", crate::model::EntryKind::File);

    assert_eq!(
        selected_terminal_location(std::slice::from_ref(&directory)),
        Some(directory.location.clone())
    );
    assert_eq!(selected_terminal_location(&[directory, file.clone()]), None);
    assert_eq!(selected_terminal_location(&[file]), None);
    assert_eq!(selected_terminal_location(&[]), None);
}

#[test]
fn recent_root_is_not_a_terminal_working_directory() {
    assert!(!can_open_terminal(&Location::uri("recent:///")));
}

#[test]
fn delayed_open_with_respects_selection_focus_navigation_and_window_lifetime() {
    crate::test_support::gtk_test(
        "ui::browser::desktop::tests::delayed_open_with_respects_selection_focus_navigation_and_window_lifetime",
        || {
            use crate::ui::{
                browser::{BrowserView, PeekBehavior},
                open_with::ApplicationChoices,
            };
            use std::{
                cell::Cell,
                time::{Duration, Instant},
            };
            crate::ui::preferences::PreferenceManager::shared().set_minimal_mode(true);
            let context = glib::MainContext::default();
            let wait = |condition: &dyn Fn() -> bool| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !condition() {
                    assert!(
                        Instant::now() < deadline,
                        "controlled Open With request did not settle"
                    );
                    context.iteration(false);
                    std::thread::sleep(Duration::from_millis(1));
                }
            };
            for action in [
                "deliver",
                "error",
                "selection",
                "navigation",
                "focus",
                "replace",
                "cancel",
                "close",
                "drop",
            ] {
                let fixture = tempfile::tempdir().expect("files");
                std::fs::write(fixture.path().join("alpha.txt"), "alpha").expect("alpha");
                std::fs::write(fixture.path().join("beta.txt"), "beta").expect("beta");
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let input = gtk::Entry::new();
                content.append(&input);
                content.append(&view.widget());
                let overlay = gtk::Overlay::builder().child(&content).build();
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                view.browser().navigate(Location::local(fixture.path()));
                wait(&|| {
                    view.browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
                });
                view.browser().select(0, 0);
                view.browser().focus_active();
                wait(&|| view.item_view_has_focus());
                let entries = view.open_with_entries();
                let (send, receive) = futures_channel::oneshot::channel();
                let started = Rc::new(Cell::new(false));
                let started_task = started.clone();
                let returned = Rc::new(Cell::new(false));
                let returned_task = returned.clone();
                show_open_with_when_ready(&view.state, entries.clone(), async move {
                    started_task.set(true);
                    let result = receive.await.expect("controlled response");
                    returned_task.set(true);
                    result
                });
                wait(&|| started.get());
                assert_eq!(
                    overlay.last_child(),
                    overlay.child(),
                    "pending metadata must not block input behind a modal"
                );
                match action {
                    "selection" => {
                        view.browser().select(0, 1);
                    }
                    "navigation" => {
                        view.browser().bump_navigation_generation();
                    }
                    "focus" => {
                        input.grab_focus();
                    }
                    "replace" => {
                        show_open_with_when_ready(&view.state, entries, std::future::pending())
                    }
                    "cancel" => view.cancel_pending_open_with(),
                    "close" => window.close(),
                    "drop" => {
                        let weak = view.downgrade();
                        view.browser().clear_observer();
                        window.close();
                        drop(view);
                        wait(&|| weak.upgrade().is_none());
                        assert!(
                            send.is_canceled(),
                            "pending lookup cannot retain the browser"
                        );
                        continue;
                    }
                    _ => {}
                }
                if matches!(action, "replace" | "cancel") {
                    wait(&|| send.is_canceled());
                    assert_eq!(overlay.last_child(), overlay.child());
                    view.cancel_pending_open_with();
                } else {
                    let result = if action == "error" {
                        Err(glib::Error::new(
                            gio::IOErrorEnum::NotFound,
                            "Selected file disappeared",
                        ))
                    } else {
                        Ok(ApplicationChoices::default())
                    };
                    assert!(send.send(result).is_ok());
                    wait(&|| returned.get());
                    assert_eq!(
                        overlay.last_child() != overlay.child(),
                        matches!(action, "deliver" | "error"),
                        "late response after {action}"
                    );
                }
                view.browser().clear_observer();
                window.close();
            }
        },
    );
}

#[test]
fn open_location_rejects_trash_locations() {
    crate::test_support::gtk_test(
        "ui::browser::desktop::tests::open_location_rejects_trash_locations",
        || {
            let overlay = gtk::Overlay::new();
            let location = Location::uri("trash:///test.png");
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            open_location(&location, &overlay, &browser);
            assert!(overlay.last_child().is_some());
        },
    );
}
