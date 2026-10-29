// SPDX-License-Identifier: MIT

use super::super::*;
use super::{FakeInstaller, GUARD_REJECTION, offered_request, page_text, wait_until};
use crate::test_support::gtk_test;

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
                release_notes_card(ReleaseNotesKind::Available, "Available release", ""),
                guard.clone(),
                UpdateMethod::InPlace,
                installer.launcher(),
            );
            let button = row.responsive_action.1.clone();
            seed_offer(&row);

            guard.set(true);
            button.emit_clicked();

            assert_eq!(row.status.text(), GUARD_REJECTION);
            assert_eq!(button.label().as_deref(), Some("Install update"));
            assert!(
                button.is_sensitive(),
                "a rejected install must stay re-triable"
            );
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

#[test]
fn release_dates_follow_the_app_language() {
    gtk_test(
        "ui::settings::tests::updates::release_dates_follow_the_app_language",
        || {
            for (locale, expected) in [
                ("en", "Sep 20, 2026"),
                ("de", "20. Sept. 2026"),
                ("hu", "2026. szept. 20."),
                ("ja", "2026年9月20日"),
                ("zh-CN", "2026年9月20日"),
                ("zh-TW", "2026年9月20日"),
                ("ru", "20 сент. 2026 г."),
            ] {
                rust_i18n::set_locale(locale);
                assert_eq!(release_date("2026-09-20T12:00:00Z"), expected, "{locale}");
            }
            assert_eq!(release_date("2026-09-20Tunknown"), "2026-09-20");
        },
    );
}

#[test]
fn release_notes_hide_html_comments_and_show_the_empty_message_without_blocks() {
    gtk_test(
        "ui::settings::tests::updates::release_notes_hide_html_comments_and_show_the_empty_message_without_blocks",
        || {
            let comment = "<!-- Release notes generated using configuration in .github/release.yml at v9.9.9 -->\n";
            for (notes, expected, summary) in [
                (
                    comment.to_owned(),
                    "No release notes were provided for this release.",
                    "Release notes",
                ),
                (
                    format!("{comment}\n## What's Changed\n* one\n"),
                    "What's Changed",
                    "1 change",
                ),
            ] {
                let card = release_notes_card(ReleaseNotesKind::Available, "Available release", "");
                let release = ReleaseMetadata {
                    version: "9.9.9".to_owned(),
                    url: "https://example.test/releases/v9.9.9".to_owned(),
                    note_blocks: crate::services::parse_markdown(&notes).blocks,
                    notes,
                    kind: BuildKind::Stable,
                    tag: "v9.9.9".to_owned(),
                    published_at: None,
                    commit: None,
                };

                show_release_notes(&card, &release);

                let text = page_text(card.notes.upcast_ref());
                assert!(
                    text.contains(expected),
                    "{expected:?} missing from {text:?}"
                );
                assert!(!text.contains("<!--"), "comment shown in {text:?}");
                assert_eq!(card.summary.text(), summary);
            }
        },
    );
}
