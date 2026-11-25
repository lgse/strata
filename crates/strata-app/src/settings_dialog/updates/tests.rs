// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
};

use rune::{Key, Mods};
use serde_json::Value;
use strata_core::i18n::tr;
use strata_desktop::services::{
    BuildKind, Channel, InstallCancel, InstallRequest, ManagedInstall, ReleaseMetadata,
    UpdateCheck, UpdateInstall, UpdateMethod, Version,
};

use super::{
    UpdatePhase,
    messages::{
        effective_update_channel, installed_version_status, offer_still_eligible, split_anchor,
    },
    view::{
        ACTION_BUTTON_KEY, MANAGED_BANNER_KEY, NOTES_CARD_KEY, PROGRESS_BAR_KEY, STATUS_LINK_KEY,
        STATUS_TEXT_KEY, STATUS_TITLE_KEY,
    },
};
use crate::{
    host::offscreen::Offscreen,
    settings_dialog::tests::{click, has_state, press},
    shell,
    test_support::{PATIENCE, hosted, view},
};

const MODULE: &str = "settings_dialog::updates::tests";
type CheckCalls = Rc<RefCell<Vec<(Channel, bool, mpsc::Sender<UpdateCheck>)>>>;
type InstallSessions = Rc<RefCell<Vec<(InstallCancel, mpsc::Sender<UpdateInstall>)>>>;

fn find_by_key<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    view::all(tree, &|node| node["key"] == key)
        .into_iter()
        .next()
}

fn open_updates(host: &mut Offscreen) {
    host.app_mut().settings_dialog.updates.method = Some(UpdateMethod::InPlace);
    click(host, "header-settings");
    click(host, "settings-nav-updates");
}

fn sample_release(
    version: &str,
    kind: BuildKind,
    notes: &str,
) -> (ReleaseMetadata, InstallRequest) {
    let tag = format!("v{version}");
    let release = ReleaseMetadata {
        version: version.to_owned(),
        url: format!("https://github.com/lgse/strata/releases/tag/{tag}"),
        notes: notes.to_owned(),
        note_blocks: Vec::new(),
        kind,
        tag: tag.clone(),
        published_at: None,
        commit: None,
    };
    let install = InstallRequest {
        tag,
        asset_name: format!("strata-{version}-x86_64.tar.gz"),
        advertised_url: format!(
            "https://github.com/lgse/strata/releases/download/v{version}/strata-{version}-x86_64.tar.gz"
        ),
    };
    (release, install)
}

#[test]
fn channel_and_status_helpers_match_expected_rules() {
    assert_eq!(
        effective_update_channel(Channel::Nightly, UpdateMethod::InPlace),
        Channel::Nightly
    );
    assert_eq!(
        effective_update_channel(Channel::Preview, UpdateMethod::Aur),
        Channel::Preview
    );
    assert_eq!(
        effective_update_channel(Channel::Nightly, UpdateMethod::Omarchy),
        Channel::Stable
    );
    assert_eq!(
        effective_update_channel(Channel::Preview, UpdateMethod::Pacman),
        Channel::Stable
    );

    assert!(offer_still_eligible(Channel::Stable, BuildKind::Stable));
    assert!(!offer_still_eligible(Channel::Stable, BuildKind::Rc));
    assert!(!offer_still_eligible(Channel::Stable, BuildKind::Nightly));
    assert!(offer_still_eligible(Channel::Preview, BuildKind::Rc));
    assert!(!offer_still_eligible(Channel::Preview, BuildKind::Nightly));
    assert!(offer_still_eligible(Channel::Nightly, BuildKind::Nightly));

    let v = Version::parse("2.0.0").expect("version");
    assert_eq!(
        installed_version_status(&v, BuildKind::Stable, UpdateMethod::InPlace, None),
        "Version 2.0.0"
    );
    assert!(
        installed_version_status(&v, BuildKind::Rc, UpdateMethod::Pacman, None)
            .contains("Managed by pacman")
    );

    let split = split_anchor(
        "Update available: <a href=\"https://example.com\">v2.1.0</a> · tail",
        "https://example.com",
    );
    assert_eq!(split, Some(("Update available: ", "v2.1.0", " · tail")));
}

#[test]
fn check_now_transitions_through_checking_to_up_to_date_available_and_failed() {
    hosted(
        MODULE,
        "check_now_transitions_through_checking_to_up_to_date_available_and_failed",
        |mut host| {
            open_updates(&mut host);

            let tree = host.tree();
            assert_eq!(
                view::text(find_by_key(&tree, STATUS_TITLE_KEY).expect("title"), "name"),
                tr("Check for updates")
            );
            assert_eq!(
                view::text(find_by_key(&tree, ACTION_BUTTON_KEY).expect("btn"), "name"),
                tr("Check now")
            );

            // Queue senders per check call so we control exact timing.
            let senders: Rc<RefCell<Vec<mpsc::Sender<UpdateCheck>>>> =
                Rc::new(RefCell::new(Vec::new()));
            let senders_hook = Rc::clone(&senders);
            let opened_uris: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
            let uris_hook = Rc::clone(&opened_uris);
            host.app_mut().settings_dialog.updates.check_hook = Some(Rc::new(move |_, _, _, _| {
                let (tx, rx) = mpsc::channel();
                senders_hook.borrow_mut().push(tx);
                rx
            }));
            host.app_mut().settings_dialog.updates.open_uri_hook = Some(Rc::new(move |uri| {
                uris_hook.borrow_mut().push(uri.to_owned());
            }));

            // 1. Checking -> UpToDate
            click(&mut host, ACTION_BUTTON_KEY);
            let tree = host.tree();
            assert_eq!(
                view::text(find_by_key(&tree, STATUS_TITLE_KEY).expect("title"), "name"),
                tr("Checking for updates…")
            );
            assert!(has_state(
                find_by_key(&tree, ACTION_BUTTON_KEY).expect("btn"),
                "disabled"
            ));

            senders
                .borrow()
                .last()
                .expect("sender")
                .send(UpdateCheck::UpToDate)
                .unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                matches!(h.app().settings_dialog.updates.phase, UpdatePhase::UpToDate)
            }));
            let tree = host.tree();
            assert_eq!(
                view::text(find_by_key(&tree, STATUS_TITLE_KEY).expect("title"), "name"),
                tr("Strata is up to date")
            );
            assert!(!has_state(
                find_by_key(&tree, ACTION_BUTTON_KEY).expect("btn"),
                "disabled"
            ));

            // Closing and reopening preserves finished check result.
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().settings_dialog.is_open());
            click(&mut host, "header-settings");
            assert!(matches!(
                host.app().settings_dialog.updates.phase,
                UpdatePhase::UpToDate
            ));

            // 2. Checking -> Failed with clickable GitHub link
            click(&mut host, ACTION_BUTTON_KEY);
            senders
                .borrow()
                .last()
                .expect("sender")
                .send(UpdateCheck::Failed("offline".to_owned()))
                .unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                matches!(
                    h.app().settings_dialog.updates.phase,
                    UpdatePhase::CheckFailed { .. }
                )
            }));
            let tree = host.tree();
            assert_eq!(
                view::text(find_by_key(&tree, STATUS_TITLE_KEY).expect("title"), "name"),
                tr("Couldn't check for updates")
            );
            click(&mut host, STATUS_LINK_KEY);
            assert_eq!(
                opened_uris.borrow().last().map(String::as_str),
                Some("https://github.com/lgse/strata/releases/latest")
            );

            // 3. Checking -> Available with release notes preview and clickable release link
            click(&mut host, ACTION_BUTTON_KEY);
            let (release, install) =
                sample_release("9.9.9", BuildKind::Stable, "- Faster indexing\n- Bug fixes");
            let expected_url = release.url.clone();
            senders
                .borrow()
                .last()
                .expect("sender")
                .send(UpdateCheck::Available { release, install })
                .unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                matches!(
                    h.app().settings_dialog.updates.phase,
                    UpdatePhase::Available { .. }
                )
            }));
            let tree = host.tree();
            assert_eq!(
                view::text(find_by_key(&tree, STATUS_TITLE_KEY).expect("title"), "name"),
                tr("An update is available")
            );
            assert_eq!(
                view::text(find_by_key(&tree, ACTION_BUTTON_KEY).expect("btn"), "name"),
                tr("Install update")
            );
            assert!(find_by_key(&tree, NOTES_CARD_KEY).is_some());
            click(&mut host, STATUS_LINK_KEY);
            assert_eq!(
                opened_uris.borrow().last().map(String::as_str),
                Some(expected_url.as_str())
            );
        },
    );
}

#[test]
fn superseded_check_is_discarded_on_channel_switch_and_auto_toggle_controls_checks() {
    hosted(
        MODULE,
        "superseded_check_is_discarded_on_channel_switch_and_auto_toggle_controls_checks",
        |mut host| {
            open_updates(&mut host);

            let calls: CheckCalls = Rc::new(RefCell::new(Vec::new()));
            let calls_hook = Rc::clone(&calls);
            host.app_mut().settings_dialog.updates.check_hook =
                Some(Rc::new(move |channel, _, _, force| {
                    let (tx, rx) = mpsc::channel();
                    calls_hook.borrow_mut().push((channel, force, tx));
                    rx
                }));

            // Start check #1 on Stable (force = true).
            click(&mut host, ACTION_BUTTON_KEY);
            assert_eq!(calls.borrow().len(), 1);
            assert_eq!(calls.borrow()[0].0, Channel::Stable);
            assert!(calls.borrow()[0].1);

            // While check #1 is still in flight, switch release channel to Preview -> check #2 (force = false).
            click(&mut host, "settings-choice-release-channel-toggle");
            click(&mut host, "settings-choice-release-channel-opt-1");
            assert_eq!(calls.borrow().len(), 2);
            assert_eq!(calls.borrow()[1].0, Channel::Preview);
            assert!(!calls.borrow()[1].1);

            // Stale check #1 resolves with Available; it must be discarded.
            let (stale_release, stale_install) = sample_release("8.0.0", BuildKind::Stable, "");
            let _ = calls.borrow()[0].2.send(UpdateCheck::Available {
                release: stale_release,
                install: stale_install,
            });
            host.runtime().frame(0.1);
            assert!(matches!(
                host.app().settings_dialog.updates.phase,
                UpdatePhase::Checking { .. }
            ));

            // Current check #2 resolves with UpToDate.
            calls.borrow()[1].2.send(UpdateCheck::UpToDate).unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                matches!(h.app().settings_dialog.updates.phase, UpdatePhase::UpToDate)
            }));

            // Toggling auto-updates OFF clears result to Idle; toggling ON runs a fresh non-forced check.
            click(&mut host, "settings-switch-auto-updates");
            assert!(!shell::preferences(host.app()).check_for_updates);
            assert!(matches!(
                host.app().settings_dialog.updates.phase,
                UpdatePhase::Idle
            ));

            click(&mut host, "settings-switch-auto-updates");
            assert!(shell::preferences(host.app()).check_for_updates);
            assert_eq!(calls.borrow().len(), 3);
            assert!(!calls.borrow()[2].1);

            // Closing Settings while Checking stops polling and resets to Idle.
            press(&mut host, Key::Escape, Mods::default());
            assert!(matches!(
                host.app().settings_dialog.updates.phase,
                UpdatePhase::Idle
            ));
        },
    );
}

#[test]
fn in_place_install_progress_cancel_failure_and_restart_work_end_to_end() {
    hosted(
        MODULE,
        "in_place_install_progress_cancel_failure_and_restart_work_end_to_end",
        |mut host| {
            open_updates(&mut host);

            let check_senders: Rc<RefCell<Vec<mpsc::Sender<UpdateCheck>>>> =
                Rc::new(RefCell::new(Vec::new()));
            let check_hook_senders = Rc::clone(&check_senders);
            host.app_mut().settings_dialog.updates.check_hook = Some(Rc::new(move |_, _, _, _| {
                let (tx, rx) = mpsc::channel();
                check_hook_senders.borrow_mut().push(tx);
                rx
            }));

            let install_sessions: InstallSessions = Rc::new(RefCell::new(Vec::new()));
            let install_hook_sessions = Rc::clone(&install_sessions);
            host.app_mut().settings_dialog.updates.install_hook =
                Some(Rc::new(move |_, cancel| {
                    let (tx, rx) = mpsc::channel();
                    install_hook_sessions.borrow_mut().push((cancel, tx));
                    rx
                }));

            let restarted = Arc::new(AtomicBool::new(false));
            let restarted_hook = Arc::clone(&restarted);
            host.app_mut().settings_dialog.updates.restart_hook = Some(Rc::new(move || {
                restarted_hook.store(true, Ordering::SeqCst);
            }));

            let offer = |host: &mut Offscreen| {
                click(host, ACTION_BUTTON_KEY);
                let (release, install) = sample_release("9.1.0", BuildKind::Stable, "");
                check_senders
                    .borrow()
                    .last()
                    .unwrap()
                    .send(UpdateCheck::Available { release, install })
                    .unwrap();
                assert!(host.run_until(PATIENCE, |h| {
                    matches!(
                        h.app().settings_dialog.updates.phase,
                        UpdatePhase::Available { .. }
                    )
                }));
            };

            // 1. Progress -> Cancel -> Cancelled
            offer(&mut host);
            click(&mut host, ACTION_BUTTON_KEY);
            assert_eq!(install_sessions.borrow().len(), 1);
            assert_eq!(
                view::text(
                    find_by_key(&host.tree(), ACTION_BUTTON_KEY).expect("cancel btn"),
                    "name"
                ),
                tr("Cancel")
            );
            assert!(find_by_key(&host.tree(), PROGRESS_BAR_KEY).is_some());

            install_sessions.borrow()[0]
                .1
                .send(UpdateInstall::Downloading {
                    downloaded: 50,
                    total: Some(100),
                })
                .unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), STATUS_TEXT_KEY)
                    .is_some_and(|n| view::text(n, "name").contains("50%"))
            }));

            click(&mut host, ACTION_BUTTON_KEY);
            assert!(install_sessions.borrow()[0].0.is_cancelled());
            install_sessions.borrow()[0]
                .1
                .send(UpdateInstall::Cancelled)
                .unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                matches!(
                    h.app().settings_dialog.updates.phase,
                    UpdatePhase::InstallCancelled
                )
            }));
            assert_eq!(
                view::text(
                    find_by_key(&host.tree(), STATUS_TEXT_KEY).expect("status"),
                    "name"
                ),
                tr("Update cancelled")
            );

            // 2. Install failure surfaces error and resets action to Check now.
            offer(&mut host);
            click(&mut host, ACTION_BUTTON_KEY);
            install_sessions.borrow()[1]
                .1
                .send(UpdateInstall::Failed("disk full".to_owned()))
                .unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                matches!(
                    h.app().settings_dialog.updates.phase,
                    UpdatePhase::InstallFailed { .. }
                )
            }));
            assert!(
                view::text(
                    find_by_key(&host.tree(), STATUS_TEXT_KEY).expect("status"),
                    "name"
                )
                .contains("disk full")
            );
            assert_eq!(
                view::text(
                    find_by_key(&host.tree(), ACTION_BUTTON_KEY).expect("btn"),
                    "name"
                ),
                tr("Check now")
            );

            // Cached prerelease offer becomes ineligible when channel reverts to Stable externally:
            // clicking Install drops the offer and re-checks instead of installing.
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| {
                    let _ = s.set_release_channel(Channel::Preview);
                });
            });
            click(&mut host, ACTION_BUTTON_KEY);
            let (rc_release, rc_install) = sample_release("9.3.0-rc.1", BuildKind::Rc, "");
            check_senders
                .borrow()
                .last()
                .unwrap()
                .send(UpdateCheck::Available {
                    release: rc_release,
                    install: rc_install,
                })
                .unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                matches!(
                    h.app().settings_dialog.updates.phase,
                    UpdatePhase::Available { .. }
                )
            }));
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| {
                    let _ = s.set_release_channel(Channel::Stable);
                });
            });
            let installs_before = install_sessions.borrow().len();
            let checks_before = check_senders.borrow().len();
            click(&mut host, ACTION_BUTTON_KEY);
            assert_eq!(
                install_sessions.borrow().len(),
                installs_before,
                "ineligible prerelease offer must not start an install"
            );
            assert_eq!(
                check_senders.borrow().len(),
                checks_before + 1,
                "ineligible prerelease offer triggers a fresh check"
            );

            // 3. Full install -> Verifying -> Installing -> Finalizing -> Installed -> Restart
            offer(&mut host);
            click(&mut host, ACTION_BUTTON_KEY);
            let tx = &install_sessions.borrow()[2].1;
            tx.send(UpdateInstall::Verifying).unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), STATUS_TEXT_KEY)
                    .is_some_and(|n| view::text(n, "name") == tr("Verifying update…"))
            }));
            tx.send(UpdateInstall::Finalizing).unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), ACTION_BUTTON_KEY).is_some_and(|n| has_state(n, "disabled"))
            }));
            tx.send(UpdateInstall::Installed).unwrap();
            assert!(host.run_until(PATIENCE, |h| {
                matches!(
                    h.app().settings_dialog.updates.phase,
                    UpdatePhase::Installed { .. }
                )
            }));
            assert_eq!(
                view::text(
                    find_by_key(&host.tree(), ACTION_BUTTON_KEY).expect("restart btn"),
                    "name"
                ),
                tr("Restart now")
            );
            click(&mut host, ACTION_BUTTON_KEY);
            assert!(restarted.load(Ordering::SeqCst));
        },
    );
}

#[test]
fn managed_install_shows_banner_disables_channel_and_launches_managed_updater() {
    hosted(
        MODULE,
        "managed_install_shows_banner_disables_channel_and_launches_managed_updater",
        |mut host| {
            let managed: ManagedInstall = toml::from_str(
                "manager = \"Omarchy\"\npackage = \"strata\"\nchannel = \"stable\"\n",
            )
            .expect("managed install");
            {
                let updates = &mut host.app_mut().settings_dialog.updates;
                updates.method = Some(UpdateMethod::Omarchy);
                updates.managed_override = Some(Some(managed));
            }
            let launched = Arc::new(AtomicUsize::new(0));
            let launched_hook = Arc::clone(&launched);
            host.app_mut().settings_dialog.updates.managed_launch_hook =
                Some(Rc::new(move |method| {
                    assert_eq!(method, UpdateMethod::Omarchy);
                    launched_hook.fetch_add(1, Ordering::SeqCst);
                    Ok(tr("Omarchy Update opened in your terminal."))
                }));
            host.app_mut().settings_dialog.updates.check_hook = Some(Rc::new(|_, _, _, _| {
                let (tx, rx) = mpsc::channel();
                let (release, install) = sample_release("9.2.0", BuildKind::Stable, "");
                tx.send(UpdateCheck::Available { release, install })
                    .unwrap();
                rx
            }));

            click(&mut host, "header-settings");
            click(&mut host, "settings-nav-updates");

            let tree = host.tree();
            assert!(find_by_key(&tree, MANAGED_BANNER_KEY).is_some());
            let channel_choice =
                find_by_key(&tree, "settings-choice-release-channel").expect("channel choice");
            assert!(has_state(channel_choice, "disabled"));

            click(&mut host, ACTION_BUTTON_KEY);
            assert!(host.run_until(PATIENCE, |h| {
                matches!(
                    h.app().settings_dialog.updates.phase,
                    UpdatePhase::Available { .. }
                )
            }));
            assert_eq!(
                view::text(
                    find_by_key(&host.tree(), ACTION_BUTTON_KEY).expect("btn"),
                    "name"
                ),
                tr("Open Omarchy Update")
            );
            click(&mut host, ACTION_BUTTON_KEY);
            assert_eq!(launched.load(Ordering::SeqCst), 1);
            assert!(matches!(
                &host.app().settings_dialog.updates.phase,
                UpdatePhase::Available {
                    managed_status: Some(status),
                    ..
                } if status == &tr("Omarchy Update opened in your terminal.")
            ));
        },
    );
}
