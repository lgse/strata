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
fn startup_results_initialize_and_update_existing_settings_rows() {
    gtk_test(
        "ui::settings::tests::updates::startup_results_initialize_and_update_existing_settings_rows",
        || {
            let manager = PreferenceManager::shared();
            manager.set_checks_for_updates(true);
            manager.set_release_channel(Channel::Stable);
            clear_cached_update_notice();
            let finish = |result| {
                let generation = next_check_generation();
                complete_due_update_check(
                    &Rc::downgrade(&manager),
                    Channel::Stable,
                    result,
                    UpdateMethod::InPlace,
                    generation,
                );
            };
            finish(UpdateCheck::UpToDate);
            let installer = FakeInstaller::default();
            let row = || {
                update_check_row_with(
                    manager.clone(),
                    release_notes_card(ReleaseNotesKind::Available, "Available release", ""),
                    install_guard(),
                    UpdateMethod::InPlace,
                    installer.launcher(),
                )
            };
            let first = row();
            assert!(
                first.status.text().starts_with("Up to date"),
                "a page opened after startup reuses the finished check"
            );
            let second = row();
            finish(UpdateCheck::Failed("test connection failure".into()));
            for row in [&first, &second] {
                assert!(row.status.text().contains("test connection failure"));
                assert!(row.responsive_action.1.is_sensitive());
                assert!(row.pending_download.borrow().is_none());
            }
            let generation = next_check_generation();
            let newer = next_check_generation();
            complete_due_update_check(
                &Rc::downgrade(&manager),
                Channel::Stable,
                UpdateCheck::UpToDate,
                UpdateMethod::InPlace,
                generation,
            );
            assert!(
                first.status.text().contains("test connection failure"),
                "an older completion cannot replace the current result"
            );
            complete_due_update_check(
                &Rc::downgrade(&manager),
                Channel::Stable,
                UpdateCheck::UpToDate,
                UpdateMethod::InPlace,
                newer,
            );
            assert!(first.status.text().starts_with("Up to date"));
            finish(UpdateCheck::Available {
                release: super::update_dialog::release(),
                install: offered_request(),
            });
            let third = row();
            assert!(
                third.pending_download.borrow().is_some(),
                "opening a new row also restores the install action"
            );
            first.responsive_action.1.emit_clicked();
            assert_eq!(installer.requests(), [offered_request()]);
            finish(UpdateCheck::Failed("later check".into()));
            assert_eq!(
                first.status.text(),
                "Downloading update…",
                "a background check cannot overwrite a running install"
            );
            assert!(second.status.text().contains("later check"));
            installer.report(UpdateInstall::Cancelled);
            wait_until("the install to cancel", || {
                first.status.text() == "Update cancelled"
            });
            let weak = second.row.downgrade();
            drop(second);
            assert!(
                weak.upgrade().is_none(),
                "a result subscription must not retain its Settings row"
            );
            clear_cached_update_notice();
            assert!(third.pending_download.borrow().is_none());
            assert_eq!(
                third.responsive_action.1.label().as_deref(),
                Some("Check now")
            );
        },
    );
}

#[cfg(feature = "qa")]
fn link_names(label: &gtk::Label) -> Vec<Option<String>> {
    std::iter::successors(label.first_accessible_child(), |link| {
        link.next_accessible_sibling()
    })
    .map(|link| crate::ui::accessibility::qa::naming(&link).and_then(|naming| naming.label))
    .collect()
}

#[cfg(feature = "qa")]
#[test]
fn update_status_links_are_named_after_their_text() {
    gtk_test(
        "ui::settings::tests::updates::update_status_links_are_named_after_their_text",
        || {
            let manager = PreferenceManager::shared();
            manager.set_checks_for_updates(true);
            manager.set_release_channel(Channel::Stable);
            clear_cached_update_notice();
            let row = update_check_row_with(
                manager.clone(),
                release_notes_card(ReleaseNotesKind::Available, "Available release", ""),
                install_guard(),
                UpdateMethod::InPlace,
                FakeInstaller::default().launcher(),
            );
            for (result, link) in [
                (
                    UpdateCheck::Failed("test connection failure".into()),
                    "View releases on GitHub",
                ),
                (
                    UpdateCheck::Available {
                        release: super::update_dialog::release(),
                        install: offered_request(),
                    },
                    "v99.0.0",
                ),
            ] {
                let generation = next_check_generation();
                complete_due_update_check(
                    &Rc::downgrade(&manager),
                    Channel::Stable,
                    result,
                    UpdateMethod::InPlace,
                    generation,
                );
                assert_eq!(link_names(&row.status), [Some(link.to_owned())]);
            }
            clear_cached_update_notice();
        },
    );
}

#[cfg(feature = "qa")]
fn channel_choice(manager: &Rc<PreferenceManager>) -> (gtk::Widget, gtk::MenuButton) {
    let (page, _) = updates_page(
        manager.clone(),
        Rc::new(|_| {}),
        install_guard(),
        UpdateMethod::InPlace,
    );
    let choice = super::descendants(&page)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::MenuButton>().ok())
        .find(|button| {
            crate::ui::accessibility::qa::naming(button)
                .and_then(|naming| naming.label)
                .as_deref()
                == Some("Release channel")
        })
        .expect("the release channel choice");
    (page, choice)
}

#[cfg(feature = "qa")]
fn assert_channel_choice_sensitive(
    (_page, choice): &(gtk::Widget, gtk::MenuButton),
    sensitive: bool,
    case: &str,
) {
    let toggle = choice.first_child().expect("the choice's toggle");
    assert_eq!(choice.get_sensitive(), sensitive, "{case}: the choice");
    assert_eq!(toggle.get_sensitive(), sensitive, "{case}: its toggle");
}

#[cfg(feature = "qa")]
#[test]
fn the_release_channel_choice_is_disabled_for_assistive_technology_without_automatic_checks() {
    gtk_test(
        "ui::settings::tests::updates::the_release_channel_choice_is_disabled_for_assistive_technology_without_automatic_checks",
        || {
            crate::ui::prepare_portal_ui();
            let manager = PreferenceManager::shared();
            // Turning checks on runs a check, so the page only sees them turned off.
            manager.set_checks_for_updates(true);
            let opened_checking = channel_choice(&manager);
            assert_channel_choice_sensitive(&opened_checking, true, "opened with checks on");
            manager.set_checks_for_updates(false);
            assert_channel_choice_sensitive(&opened_checking, false, "checks turned off");
            let opened_without = channel_choice(&manager);
            assert_channel_choice_sensitive(&opened_without, false, "opened with checks off");
        },
    );
}

#[test]
fn a_check_started_in_one_row_keeps_other_rows_offers_until_its_result() {
    gtk_test(
        "ui::settings::tests::updates::a_check_started_in_one_row_keeps_other_rows_offers_until_its_result",
        || {
            let manager = PreferenceManager::shared();
            manager.set_checks_for_updates(true);
            manager.set_release_channel(Channel::Stable);
            clear_cached_update_notice();
            let notices = Rc::new(RefCell::new(Vec::new()));
            let notice: UpdateNoticeHandler = Rc::new({
                let notices = notices.clone();
                move |result| notices.borrow_mut().push(result.is_some())
            });
            register_update_notice(&notice);
            let generation = next_check_generation();
            complete_due_update_check(
                &Rc::downgrade(&manager),
                Channel::Stable,
                UpdateCheck::Available {
                    release: super::update_dialog::release(),
                    install: offered_request(),
                },
                UpdateMethod::InPlace,
                generation,
            );
            let row = || {
                update_check_row_with(
                    manager.clone(),
                    release_notes_card(ReleaseNotesKind::Available, "Available release", ""),
                    install_guard(),
                    UpdateMethod::InPlace,
                    FakeInstaller::default().launcher(),
                )
            };
            let (first, second) = (row(), row());
            assert_eq!(notices.borrow().as_slice(), [true]);

            (second.run_check)(true);

            assert!(second.pending_download.borrow().is_none());
            assert!(
                first.pending_download.borrow().is_some(),
                "a check elsewhere must not drop this row's install offer"
            );
            assert_eq!(
                first.responsive_action.1.label().as_deref(),
                Some("Install update")
            );
            assert!(first.responsive_action.1.is_sensitive());
            assert_eq!(
                notices.borrow().as_slice(),
                [true, false],
                "the sidebar notice clears as soon as a check starts"
            );
            assert!(update_status::available().is_none());
            // Orphan the started check so its result cannot land in later assertions.
            next_check_generation();
            clear_cached_update_notice();
            assert!(first.pending_download.borrow().is_none());
        },
    );
}

#[test]
fn startup_offers_keep_package_managed_actions_and_reject_obsolete_channels() {
    gtk_test(
        "ui::settings::tests::updates::startup_offers_keep_package_managed_actions_and_reject_obsolete_channels",
        || {
            let manager = PreferenceManager::shared();
            manager.set_checks_for_updates(true);
            // Package-managed rows follow Stable whatever the preference says.
            manager.set_release_channel(Channel::Preview);
            for method in [
                UpdateMethod::Aur,
                UpdateMethod::Omarchy,
                UpdateMethod::Pacman,
            ] {
                clear_cached_update_notice();
                let generation = next_check_generation();
                complete_due_update_check(
                    &Rc::downgrade(&manager),
                    Channel::Preview,
                    UpdateCheck::Available {
                        release: super::update_dialog::release(),
                        install: offered_request(),
                    },
                    method,
                    generation,
                );
                let row = update_check_row_with(
                    manager.clone(),
                    release_notes_card(ReleaseNotesKind::Available, "Available release", ""),
                    install_guard(),
                    method,
                    FakeInstaller::default().launcher(),
                );
                assert!(page_text(row.row.upcast_ref()).contains("99.0.0"));
                assert!(
                    row.pending_download.borrow().is_none(),
                    "{method:?} must not offer an in-place installation"
                );
                assert_eq!(
                    row.responsive_action.1.label().as_deref(),
                    Some(match method {
                        UpdateMethod::Aur => aur_update_action_label(),
                        UpdateMethod::Omarchy => "Open Omarchy Update",
                        UpdateMethod::Pacman => "Check again",
                        UpdateMethod::InPlace => unreachable!(),
                    })
                );
                let generation = next_check_generation();
                complete_due_update_check(
                    &Rc::downgrade(&manager),
                    Channel::Stable,
                    UpdateCheck::Failed("wrong channel".into()),
                    method,
                    generation,
                );
                assert!(!row.status.text().contains("wrong channel"));
            }
            clear_cached_update_notice();
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
