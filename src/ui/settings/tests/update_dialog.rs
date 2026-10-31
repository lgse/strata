// SPDX-License-Identifier: MIT

use gtk::gdk;

use super::super::*;
use super::{FakeInstaller, GUARD_REJECTION, offered_request, page_text, wait_until};
use crate::test_support::{gtk_test, gtk_test_with_env};

#[derive(Clone, Copy, Debug)]
enum Dismisser {
    Cancel,
    Close,
    Escape,
    Backdrop,
}

const DISMISSERS: [Dismisser; 4] = [
    Dismisser::Cancel,
    Dismisser::Close,
    Dismisser::Escape,
    Dismisser::Backdrop,
];

fn press_escape(dialog: &UpdateDialog) -> bool {
    dialog.escape.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::Escape, &0u32, &gdk::ModifierType::empty()],
    )
}

fn press_backdrop(dialog: &UpdateDialog) {
    wait_until("the dialog to be laid out", || dialog.action.width() > 0);
    let controllers = dialog.layer.observe_controllers();
    let click = (0..controllers.n_items())
        .find_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .expect("backdrop click gesture");
    click.emit_by_name::<()>("pressed", &[&1i32, &2.0f64, &2.0f64]);
}

fn trigger(dialog: &UpdateDialog, dismisser: Dismisser) {
    match dismisser {
        Dismisser::Cancel => dialog.cancel.emit_clicked(),
        Dismisser::Close => dialog.close.emit_clicked(),
        Dismisser::Escape => assert!(press_escape(dialog), "Escape must be handled"),
        Dismisser::Backdrop => press_backdrop(dialog),
    }
}

fn dismiss_with(dialog: &UpdateDialog, dismisser: Dismisser) {
    match dismisser {
        Dismisser::Cancel => assert!(dialog.cancel.is_sensitive(), "Cancel must be enabled"),
        Dismisser::Close => assert!(dialog.close.is_sensitive(), "X must be enabled"),
        Dismisser::Escape | Dismisser::Backdrop => {}
    }
    trigger(dialog, dismisser);
}

fn is_dismissing(dialog: &UpdateDialog) -> bool {
    dialog.layer.has_css_class("dismissing") || dialog.layer.parent().is_none()
}

fn assert_finalizing(dialog: &UpdateDialog, dismisser: Dismisser) {
    assert_eq!(dialog.status.text(), FINALIZING_STATUS, "{dismisser:?}");
    assert!(
        !dialog.cancel.is_sensitive(),
        "{dismisser:?}: Cancel while finalizing"
    );
    assert!(
        !dialog.close.is_sensitive(),
        "{dismisser:?}: X while finalizing"
    );
    assert!(
        !is_dismissing(dialog),
        "{dismisser:?}: the dialog stays open"
    );
}

fn wait_until_dismissed(dialog: &UpdateDialog) {
    wait_until("the dialog to dismiss", || dialog.layer.parent().is_none());
}

pub(super) fn release() -> ReleaseMetadata {
    ReleaseMetadata {
        version: "99.0.0".to_owned(),
        url: "https://github.com/lgse/strata/releases/tag/v99.0.0".to_owned(),
        notes: String::new(),
        note_blocks: Vec::new(),
        kind: BuildKind::Stable,
        tag: "v99.0.0".to_owned(),
        published_at: None,
        commit: None,
    }
}

fn open_dialog(installer: &FakeInstaller) -> (gtk::Window, UpdateDialog) {
    open_dialog_for(installer, &release(), UpdateMethod::InPlace)
}

fn open_dialog_for(
    installer: &FakeInstaller,
    release: &ReleaseMetadata,
    update_method: UpdateMethod,
) -> (gtk::Window, UpdateDialog) {
    PreferenceManager::shared().set_release_channel(Channel::Stable);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&BlurBin::new(&gtk::Box::new(
        gtk::Orientation::Vertical,
        0,
    ))));
    let window = gtk::Window::builder()
        .default_width(1200)
        .default_height(900)
        .child(&overlay)
        .build();
    window.present();
    let dialog = build_update_dialog(
        &window,
        release,
        offered_request(),
        install_guard(),
        update_method,
        UpdateOffer::Available,
        installer.launcher(),
    )
    .expect("update dialog");
    (window, dialog)
}

fn fail_download(dialog: &UpdateDialog, installer: &FakeInstaller, outcome: Option<&str>) {
    dialog.action.emit_clicked();
    match outcome {
        Some(message) => installer.report(UpdateInstall::Failed(message.to_owned())),
        None => installer.disconnect(),
    }
    wait_until("the download to fail", || {
        dialog.status.text().starts_with("Couldn’t install update")
    });
}

#[test]
fn closing_the_dialog_during_a_download_cancels_it() {
    gtk_test(
        "ui::settings::tests::update_dialog::closing_the_dialog_during_a_download_cancels_it",
        || {
            let guard = install_guard();
            for dismisser in DISMISSERS {
                guard.set(false);
                let installer = FakeInstaller::default();
                let (window, dialog) = open_dialog(&installer);

                dialog.action.emit_clicked();

                assert_eq!(installer.requests(), [offered_request()], "{dismisser:?}");
                assert!(guard.get(), "{dismisser:?}: the download holds the guard");
                assert!(!dialog.action.is_sensitive(), "{dismisser:?}");
                dismiss_with(&dialog, dismisser);
                assert!(installer.cancel_requested(), "{dismisser:?} must cancel");
                wait_until_dismissed(&dialog);

                installer.report(UpdateInstall::Cancelled);
                wait_until("the cancelled install to release the guard", || {
                    !guard.get()
                });

                window.destroy();
            }
        },
    );
}

#[test]
fn guard_rejection_leaves_every_dismisser_working() {
    gtk_test(
        "ui::settings::tests::update_dialog::guard_rejection_leaves_every_dismisser_working",
        || {
            let guard = install_guard();
            for dismisser in DISMISSERS {
                guard.set(true);
                let installer = FakeInstaller::default();
                let (window, dialog) = open_dialog(&installer);

                dialog.action.emit_clicked();

                assert_eq!(dialog.status.text(), GUARD_REJECTION, "{dismisser:?}");
                assert!(installer.requests().is_empty(), "{dismisser:?}");
                assert!(
                    dialog.action.is_sensitive(),
                    "{dismisser:?}: action retries"
                );
                dismiss_with(&dialog, dismisser);
                wait_until_dismissed(&dialog);
                assert!(
                    guard.get(),
                    "{dismisser:?}: the other install keeps the guard"
                );

                guard.set(false);
                window.destroy();
            }
        },
    );
}

#[test]
fn failed_download_leaves_every_dismisser_working() {
    gtk_test(
        "ui::settings::tests::update_dialog::failed_download_leaves_every_dismisser_working",
        || {
            let guard = install_guard();
            for dismisser in DISMISSERS {
                guard.set(false);
                let installer = FakeInstaller::default();
                let (window, dialog) = open_dialog(&installer);

                fail_download(&dialog, &installer, Some("checksum"));

                assert_eq!(
                    dialog.status.text(),
                    "Couldn’t install update: checksum",
                    "{dismisser:?}"
                );
                assert!(
                    !guard.get(),
                    "{dismisser:?}: a failed install releases the guard"
                );
                assert_eq!(
                    dialog.action.label().as_deref(),
                    Some("Close"),
                    "{dismisser:?}"
                );
                assert!(dialog.action.is_sensitive(), "{dismisser:?}");
                dismiss_with(&dialog, dismisser);
                wait_until_dismissed(&dialog);
                assert!(
                    !installer.cancel_requested(),
                    "{dismisser:?}: nothing to cancel"
                );

                window.destroy();
            }
        },
    );
}

#[test]
fn close_action_after_a_failure_dismisses_instead_of_retrying() {
    gtk_test(
        "ui::settings::tests::update_dialog::close_action_after_a_failure_dismisses_instead_of_retrying",
        || {
            let guard = install_guard();
            guard.set(false);
            let installer = FakeInstaller::default();
            let (window, dialog) = open_dialog(&installer);
            fail_download(&dialog, &installer, Some("checksum"));

            dialog.action.emit_clicked();

            wait_until_dismissed(&dialog);
            assert_eq!(
                installer.requests(),
                [offered_request()],
                "no second install"
            );
            assert!(!guard.get());
            window.destroy();
        },
    );
}

#[test]
fn disconnected_installer_counts_as_a_failure() {
    gtk_test(
        "ui::settings::tests::update_dialog::disconnected_installer_counts_as_a_failure",
        || {
            let guard = install_guard();
            guard.set(false);
            let installer = FakeInstaller::default();
            let (window, dialog) = open_dialog(&installer);

            fail_download(&dialog, &installer, None);

            assert_eq!(dialog.status.text(), "Couldn’t install update");
            assert!(!guard.get(), "a disconnected installer releases the guard");
            dismiss_with(&dialog, Dismisser::Close);
            wait_until_dismissed(&dialog);
            window.destroy();
        },
    );
}

#[test]
fn finalizing_blocks_every_dismisser_until_the_install_ends() {
    gtk_test(
        "ui::settings::tests::update_dialog::finalizing_blocks_every_dismisser_until_the_install_ends",
        || {
            let guard = install_guard();
            for dismisser in DISMISSERS {
                guard.set(false);
                let installer = FakeInstaller::default();
                let (window, dialog) = open_dialog(&installer);
                dialog.action.emit_clicked();
                installer.commit();
                installer.report(UpdateInstall::Finalizing);
                wait_until("the finalizing report", || {
                    dialog.status.text() == FINALIZING_STATUS
                });

                trigger(&dialog, dismisser);

                assert_finalizing(&dialog, dismisser);
                assert!(guard.get(), "{dismisser:?}: the install keeps the guard");

                installer.report(UpdateInstall::Failed("disk full".to_owned()));
                wait_until("the install to fail", || {
                    dialog.status.text().contains("disk full")
                });
                assert_eq!(
                    dialog.action.label().as_deref(),
                    Some("Close"),
                    "{dismisser:?}"
                );
                dismiss_with(&dialog, dismisser);
                wait_until_dismissed(&dialog);
                window.destroy();
            }
        },
    );
}

#[test]
fn cancel_refused_by_a_committed_install_switches_to_finalizing() {
    gtk_test(
        "ui::settings::tests::update_dialog::cancel_refused_by_a_committed_install_switches_to_finalizing",
        || {
            let guard = install_guard();
            for dismisser in DISMISSERS {
                guard.set(false);
                let installer = FakeInstaller::default();
                let (window, dialog) = open_dialog(&installer);
                dialog.action.emit_clicked();
                // Committed, but the Finalizing report has not arrived yet.
                installer.commit();

                dismiss_with(&dialog, dismisser);

                assert_finalizing(&dialog, dismisser);
                assert!(!installer.cancel_requested(), "{dismisser:?}");

                installer.report(UpdateInstall::Failed("disk full".to_owned()));
                wait_until("the install to fail", || !guard.get());
                window.destroy();
            }
        },
    );
}

#[test]
fn installed_update_leaves_every_dismisser_working() {
    gtk_test(
        "ui::settings::tests::update_dialog::installed_update_leaves_every_dismisser_working",
        || {
            let guard = install_guard();
            for dismisser in DISMISSERS {
                guard.set(false);
                let installer = FakeInstaller::default();
                let (window, dialog) = open_dialog(&installer);
                dialog.action.emit_clicked();
                installer.commit();
                installer.report(UpdateInstall::Finalizing);

                installer.report(UpdateInstall::Installed);
                wait_until("the install to finish", || {
                    dialog.action.label().as_deref() == Some("Restart now")
                });

                assert!(
                    !guard.get(),
                    "{dismisser:?}: an installed update releases the guard"
                );
                dismiss_with(&dialog, dismisser);
                wait_until_dismissed(&dialog);
                window.destroy();
            }
        },
    );
}

#[test]
fn update_dialog_notes_hide_html_comments() {
    gtk_test(
        "ui::settings::tests::update_dialog::update_dialog_notes_hide_html_comments",
        || {
            let comment = "<!-- Release notes generated using configuration in .github/release.yml at v99.0.0 -->\n";
            for (notes, expected) in [
                (
                    comment.to_owned(),
                    "No release notes were provided. Review this release on GitHub before continuing.",
                ),
                (
                    format!("{comment}\n## What's Changed\n* one\n"),
                    "What's Changed",
                ),
            ] {
                let release = ReleaseMetadata {
                    note_blocks: crate::services::parse_markdown(&notes).blocks,
                    notes,
                    ..release()
                };
                let installer = FakeInstaller::default();
                let (window, dialog) = open_dialog_for(&installer, &release, UpdateMethod::InPlace);

                let text = page_text(dialog.layer.upcast_ref());
                assert!(
                    text.contains(expected),
                    "{expected:?} missing from {text:?}"
                );
                assert!(!text.contains("<!--"), "comment shown in {text:?}");
                window.destroy();
            }
        },
    );
}

const NO_TERMINAL: [(&str, &str); 2] =
    [("PATH", "/nonexistent/strata-test-path"), ("TERMINAL", "")];

#[test]
fn omarchy_update_without_a_terminal_reports_and_reenables_the_action() {
    gtk_test_with_env(
        "ui::settings::tests::update_dialog::omarchy_update_without_a_terminal_reports_and_reenables_the_action",
        NO_TERMINAL,
        || {
            let installer = FakeInstaller::default();
            let (window, dialog) = open_dialog_for(&installer, &release(), UpdateMethod::Omarchy);

            dialog.action.emit_clicked();

            assert!(!dialog.action.is_sensitive(), "one launch at a time");
            wait_until("the launch to fail", || {
                dialog.status.text().contains("No terminal emulator")
            });
            let status = dialog.status.text();
            assert!(
                status.starts_with("Couldn’t open Omarchy Update"),
                "{status}"
            );
            assert!(dialog.action.is_sensitive(), "the action retries");
            assert!(dialog.layer.parent().is_some(), "the dialog stays open");
            assert!(installer.requests().is_empty());
            window.destroy();
        },
    );
}

#[test]
fn aur_helpers_are_found_off_the_ui_thread_and_the_update_actions_use_that_answer() {
    let bin = tempfile::tempdir().expect("isolated PATH");
    gtk_test_with_env(
        "ui::settings::tests::update_dialog::aur_helpers_are_found_off_the_ui_thread_and_the_update_actions_use_that_answer",
        [
            ("PATH", bin.path().as_os_str()),
            ("TERMINAL", std::ffi::OsStr::new("")),
        ],
        || {
            use std::os::unix::fs::PermissionsExt;
            let bin = std::path::PathBuf::from(std::env::var_os("PATH").expect("isolated PATH"));
            let helper = bin.join("paru");
            std::fs::write(&helper, "#!/bin/sh\n").expect("AUR helper stub");
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755))
                .expect("executable AUR helper stub");
            InstallSource::seed_for_test(
                "manager = \"pacman\"\npackage = \"strata-bin\"\naur_helpers = [\"yay\", \"paru\"]\n",
            );
            let managed = InstallSource::detect()
                .managed()
                .expect("a managed install");
            assert_eq!(aur_update_action_label(), "View on AUR");

            let resolved = Rc::new(std::cell::Cell::new(None));
            let answer = resolved.clone();
            resolve_update_method_async(move |method| answer.set(Some(method)));
            wait_until("the update method", || resolved.get().is_some());
            assert_eq!(resolved.get(), Some(UpdateMethod::Aur));
            assert_eq!(aur_update_action_label(), "Open AUR Update");
            let summary = managed_install_summary(managed);
            assert!(
                summary.contains("Update Strata with: paru -Syu strata-bin"),
                "{summary}"
            );

            let answer = Rc::new(RefCell::new(None));
            let launched = answer.clone();
            glib::MainContext::default().spawn_local(async move {
                launched.replace(Some(launch_aur_update().await));
            });
            wait_until("the AUR launch to answer", || answer.borrow().is_some());
            let error = answer
                .take()
                .expect("an answer")
                .expect_err("no terminal to open");
            assert!(error.contains("No terminal emulator"), "{error}");
        },
    );
}
