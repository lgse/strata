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

#[test]
fn archive_collisions_use_the_final_name() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let destination = Location::local(root.path());
    assert!(!archive_has_collision(&destination, "archive.zip"));
    std::fs::write(root.path().join("archive.zip"), b"existing")?;
    assert!(archive_has_collision(&destination, "archive.zip"));
    Ok(())
}

/// Text of every visible label and button in the browser overlay.
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

#[test]
fn only_a_password_failure_kind_opens_the_extract_password_dialog() {
    crate::test_support::gtk_test(
        "ui::browser::archive::tests::only_a_password_failure_kind_opens_the_extract_password_dialog",
        || {
            // The same wording for every kind: only the kind may decide.
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
