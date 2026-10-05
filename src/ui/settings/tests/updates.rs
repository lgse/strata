// SPDX-License-Identifier: MIT

use super::super::*;
use super::{FakeInstaller, GUARD_REJECTION, offered_request, wait_until};
use crate::test_support::gtk_test;

/// What a completed check leaves behind when it offers a stable update.
fn seed_offer(row: &UpdateCheckRow) {
    row.pending_download.replace(Some(PendingInstall {
        kind: BuildKind::Stable,
        returns_to_stable: false,
        request: offered_request(),
    }));
    row.responsive_action.1.set_label("Install update");
}

#[test]
fn install_click_rejected_by_the_guard_keeps_the_row_usable() {
    gtk_test(
        "ui::settings::tests::updates::install_click_rejected_by_the_guard_keeps_the_row_usable",
        || {
            let manager = PreferenceManager::shared();
            manager.set_release_channel(Channel::Stable);
            let installer = FakeInstaller::default();
            let guard = install_guard();
            let row = update_check_row_with(
                manager,
                release_notes_card("Available release", ""),
                guard.clone(),
                UpdateMethod::InPlace,
                installer.launcher(),
            );
            let button = row.responsive_action.1.clone();
            seed_offer(&row);

            // Another window's row or an update dialog holds the install.
            guard.set(true);
            button.emit_clicked();

            assert_eq!(row.status.text(), GUARD_REJECTION);
            assert_eq!(button.label().as_deref(), Some("Install update"));
            assert!(button.is_sensitive(), "a rejected install must stay re-triable");
            assert!(row.pending_download.borrow().is_some(), "the offer is kept");
            assert!(installer.requests().is_empty());
            assert!(guard.get(), "the running install keeps the guard");

            guard.set(false);
            button.emit_clicked();

            assert_eq!(installer.requests(), [offered_request()]);
            assert!(guard.get(), "the retried install holds the guard");
            assert!(row.pending_download.borrow().is_none());
            assert_eq!(row.status.text(), "Downloading update…");
            assert_eq!(button.label().as_deref(), Some("Cancel"));

            installer.report(UpdateInstall::Failed("boom".to_owned()));
            wait_until("the install failure", || row.status.text().contains("boom"));

            assert_eq!(row.status.text(), "Couldn't install update: boom");
            assert_eq!(button.label().as_deref(), Some("Check now"));
            assert!(button.is_sensitive());
            assert!(!guard.get(), "a failed install releases the guard");
        },
    );
}
