// SPDX-License-Identifier: MIT

use super::*;
use crate::model::Location;
use crate::services::{ArchiveFormat, validate_basename};

#[test]
fn archive_names_strip_only_the_selected_dotted_extension() {
    assert_eq!(
        normalized_archive_name("backup.zip", ArchiveFormat::Zip),
        "backup"
    );
    assert_eq!(
        normalized_archive_name("backupzip", ArchiveFormat::Zip),
        "backupzip"
    );
    assert_eq!(
        normalized_archive_name("backup.tar.gz", ArchiveFormat::TarGz),
        "backup"
    );
    assert_eq!(
        normalized_archive_name("backup.rar", ArchiveFormat::Rar),
        "backup"
    );
    assert!(
        validate_basename(&normalized_archive_name(
            "../outside.zip",
            ArchiveFormat::Zip
        ))
        .is_err()
    );
}

fn visible_texts(overlay: &gtk::Overlay) -> Vec<String> {
    let mut texts = Vec::new();
    let mut stack = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        stack.push(widget.clone());
        child = widget.next_sibling();
    }
    while let Some(widget) = stack.pop() {
        if !widget.is_visible() {
            continue;
        }
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            texts.push(label.label().to_string());
        }
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && let Some(label) = button.label()
        {
            texts.push(label.to_string());
        }
        let mut descendant = widget.first_child();
        while let Some(next) = descendant {
            stack.push(next.clone());
            descendant = next.next_sibling();
        }
    }
    texts
}

fn wait_until(condition: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[derive(Clone, Copy)]
enum AtTheName {
    Nothing,
    File(&'static str),
    DanglingLink,
    FailedCheck,
    SlowCheck,
}

#[test]
fn compress_checks_the_final_name_off_the_ui_thread_and_asks_only_when_it_is_taken() {
    crate::test_support::gtk_test(
        "ui::browser::archive::tests::compress_checks_the_final_name_off_the_ui_thread_and_asks_only_when_it_is_taken",
        || {
            for (case, at_the_name, asks) in [
                ("a free name", AtTheName::Nothing, false),
                (
                    "a file named without the extension",
                    AtTheName::File("archive"),
                    false,
                ),
                (
                    "an archive of that name",
                    AtTheName::File("archive.zip"),
                    true,
                ),
                (
                    "a dangling link of that name",
                    AtTheName::DanglingLink,
                    true,
                ),
                (
                    "a name that cannot be checked",
                    AtTheName::FailedCheck,
                    false,
                ),
                ("a name slow to check", AtTheName::SlowCheck, false),
            ] {
                let fixture = tempfile::tempdir().expect("compress fixture");
                let source = fixture.path().join("notes.txt");
                std::fs::write(&source, b"notes").expect("source file");
                let archive = fixture.path().join("archive.zip");
                match at_the_name {
                    AtTheName::File(name) => {
                        std::fs::write(fixture.path().join(name), b"existing").expect("existing")
                    }
                    AtTheName::DanglingLink => {
                        std::os::unix::fs::symlink("nowhere", &archive).expect("dangling link")
                    }
                    _ => {}
                }
                let (release, held) = std::sync::mpsc::channel::<()>();
                let checked = std::sync::Arc::new(std::sync::Mutex::new(None));
                let seen = checked.clone();
                let slow = matches!(at_the_name, AtTheName::SlowCheck);
                let _scope = crate::adapters::FsQuirks::install(
                    fixture.path(),
                    crate::adapters::Quirks {
                        before: vec![(
                            crate::adapters::FsOp::Stat,
                            "archive.zip",
                            0,
                            Box::new(move || {
                                *seen.lock().expect("check thread") =
                                    Some(crate::util::ui_thread::is_current());
                                if slow {
                                    let _released = held.recv();
                                }
                            }),
                        )],
                        fail: if matches!(at_the_name, AtTheName::FailedCheck) {
                            vec![(
                                crate::adapters::FsOp::Stat,
                                "archive.zip",
                                0,
                                rustix::io::Errno::IO,
                            )]
                        } else {
                            Vec::new()
                        },
                        ..crate::adapters::Quirks::ext4()
                    },
                );
                let view = crate::ui::browser::BrowserView::new(
                    std::rc::Rc::new(crate::adapters::LocalFileSource),
                    crate::ui::browser::PeekBehavior::default(),
                );
                view.set_operation_provider(std::rc::Rc::new(
                    crate::adapters::LocalOperationProvider,
                ));
                let overlay = view.overlay();
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                let started = view.browser().last_started_operation();

                view.state.start_compression(
                    vec![crate::test_support::operations::entry(Location::local(
                        &source,
                    ))],
                    Location::local(fixture.path()),
                    "archive".to_owned(),
                    ArchiveFormat::Zip,
                    None,
                );

                wait_until(
                    || checked.lock().expect("check thread").is_some(),
                    &format!("{case}: the archive name to be checked"),
                );
                assert_eq!(
                    *checked.lock().expect("check thread"),
                    Some(false),
                    "{case}: the name is checked off the UI thread"
                );
                if slow {
                    for _ in 0..20 {
                        glib::MainContext::default().iteration(false);
                    }
                    assert_eq!(
                        view.browser().last_started_operation(),
                        started,
                        "{case}: nothing is compressed before the name answers"
                    );
                    release.send(()).expect("held check");
                }
                if asks {
                    wait_until(
                        || {
                            let texts = visible_texts(&overlay);
                            texts.iter().any(|text| text == "File already exists")
                                && texts.iter().any(|text| text == "Keep Both")
                        },
                        &format!("{case}: the Replace or Keep Both prompt"),
                    );
                    assert_eq!(
                        view.browser().last_started_operation(),
                        started,
                        "{case}: nothing is compressed before the prompt is answered"
                    );
                } else {
                    wait_until(
                        || {
                            std::fs::symlink_metadata(&archive)
                                .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
                                && view.browser().last_started_operation().is_some_and(
                                    |operation| !view.browser().is_current_operation(operation),
                                )
                        },
                        &format!("{case}: the archive to be written"),
                    );
                    assert!(
                        !visible_texts(&overlay)
                            .iter()
                            .any(|text| text == "File already exists"),
                        "{case}: a free name is not asked about"
                    );
                }
                window.destroy();
            }
        },
    );
}

#[test]
fn only_a_password_failure_kind_opens_the_extract_password_dialog() {
    crate::test_support::gtk_test(
        "ui::browser::archive::tests::only_a_password_failure_kind_opens_the_extract_password_dialog",
        || {
            let message = "The password may be incorrect. Could not open `passwords.zip`.";
            for (password_failure, password_dialog, invalid_password) in [
                (None, false, false),
                (Some(PasswordFailure::Required), true, false),
                (Some(PasswordFailure::Incorrect), true, true),
            ] {
                let fixture = tempfile::tempdir().expect("archive fixture");
                let view = crate::ui::browser::BrowserView::new(
                    std::rc::Rc::new(crate::adapters::LocalFileSource),
                    crate::ui::browser::PeekBehavior::default(),
                );
                let overlay = view.overlay();
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                let archive = Location::local(fixture.path().join("passwords.zip"));
                view.state.pending_extract_retry.replace(Some((
                    crate::test_support::operations::entry(archive),
                    Location::local(fixture.path()),
                )));

                view.state
                    .handle(&crate::app::BrowserEvent::OperationFailed {
                        message: message.to_owned(),
                        password_failure,
                        report: Default::default(),
                    });
                let context = glib::MainContext::default();
                while context.pending() {
                    context.iteration(false);
                }

                let texts = visible_texts(&overlay);
                let label = format!("{password_failure:?}: {texts:?}");
                assert_eq!(
                    texts.iter().any(|text| text == "Password"),
                    password_dialog,
                    "{label}"
                );
                assert_eq!(
                    texts.iter().any(|text| text == "Invalid password"),
                    invalid_password,
                    "{label}"
                );
                assert_eq!(
                    texts
                        .iter()
                        .any(|text| text == "Unable to complete operation"),
                    !password_dialog,
                    "{label}"
                );
                assert_eq!(
                    texts.iter().any(|text| text == message),
                    !password_dialog,
                    "{label}"
                );
                window.destroy();
            }
        },
    );
}
