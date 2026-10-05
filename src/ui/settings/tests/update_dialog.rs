// SPDX-License-Identifier: MIT

use super::super::*;
use super::{FakeInstaller, GUARD_REJECTION, offered_request, wait_until};
use crate::test_support::gtk_test;

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

/// Presses the backdrop's top-left corner, well outside the dialog card.
fn press_backdrop(dialog: &UpdateDialog) {
    wait_until("the dialog to be laid out", || dialog.action.width() > 0);
    let controllers = dialog.layer.observe_controllers();
    let click = (0..controllers.n_items())
        .find_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .expect("backdrop click gesture");
    click.emit_by_name::<()>("pressed", &[&1i32, &2.0f64, &2.0f64]);
}

/// Triggers `dismisser` whether or not its button is enabled.
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

fn release() -> ReleaseMetadata {
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
    PreferenceManager::shared().set_release_channel(Channel::Stable);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&BlurBin::new(&gtk::Box::new(
        gtk::Orientation::Vertical,
        0,
    ))));
    // Large enough that the backdrop surrounds the dialog card.
    let window = gtk::Window::builder()
        .default_width(1200)
        .default_height(900)
        .child(&overlay)
        .build();
    window.present();
    let dialog = build_update_dialog(
        &window,
        &release(),
        offered_request(),
        install_guard(),
        UpdateMethod::InPlace,
        installer.launcher(),
    )
    .expect("update dialog");
    (window, dialog)
}

/// Starts the download and waits for the installer to report `outcome`, or
/// to disconnect without one when `None`.
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
                // An install from the update row or another window is running.
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

                // The test window has no application, so nothing restarts.
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
