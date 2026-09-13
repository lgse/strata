// SPDX-License-Identifier: MIT

use super::*;
use crate::model::Location;
use gtk::{gio, glib};
use std::time::{Duration, Instant};

#[test]
fn password_storage_selection_maps_to_gio_values() {
    assert_eq!(password_save_for_selection(0), gio::PasswordSave::Never);
    assert_eq!(
        password_save_for_selection(1),
        gio::PasswordSave::ForSession
    );
    assert_eq!(
        password_save_for_selection(2),
        gio::PasswordSave::Permanently
    );
    assert_eq!(password_save_for_selection(99), gio::PasswordSave::Never);
}

#[test]
fn location_input_credentials_are_one_shot_and_never_saved() {
    let (location, credentials) = credentials_from_location_input("smb://alice:secret@host/share")
        .expect("credential URI should parse");
    let credentials = credentials.expect("credentials should be separated");

    assert_eq!(location, "smb://alice@host/share");
    assert_eq!(credentials.username, "alice");
    assert_eq!(credentials.password, "secret");
    assert_eq!(credentials.save, gio::PasswordSave::Never);
}

#[test]
fn remote_permission_denials_are_treated_as_authentication_failures() {
    let denied = glib::Error::new(gio::IOErrorEnum::PermissionDenied, "Permission denied");
    let smb_denied = glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Failed to mount Windows share: Permission denied",
    );
    let remote = Location::uri("smb://host/share");
    assert!(mount_error_is_authentication_failure(&remote, &denied));
    assert!(mount_error_is_authentication_failure(&remote, &smb_denied,));
    assert!(!mount_error_is_authentication_failure(
        &Location::local("/root"),
        &denied,
    ));
}

#[test]
fn cancelling_the_credential_prompt_produces_no_error_message() {
    let location = Location::uri("smb://host/share");
    for kind in [gio::IOErrorEnum::Cancelled, gio::IOErrorEnum::FailedHandled] {
        let error = glib::Error::new(kind, "cancelled by the user");
        assert_eq!(mount_failure_message(&location, &error), None);
    }
}

#[test]
fn a_missing_backend_reports_which_package_to_install() {
    let location = Location::uri("smb://host/share");
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "no handler for smb");
    let message = mount_failure_message(&location, &error).expect("should report a message");
    assert!(message.contains("gvfs-smb"));
}

#[test]
fn a_genuine_mount_failure_still_reports_an_error() {
    let location = Location::uri("smb://host/share");
    let error = glib::Error::new(gio::IOErrorEnum::HostNotFound, "no route to host");
    let message = mount_failure_message(&location, &error).expect("should report a message");
    assert!(message.contains("no route to host"));
}

#[test]
fn authentication_failure_without_a_backend_prompt_gets_login_fields() {
    let location = Location::uri("smb://host/share");
    let details = MountPromptDetails::fallback(&location);
    assert!(details.message.contains("smb://host/share"));
    assert!(details.flags.contains(gio::AskPasswordFlags::NEED_USERNAME));
    assert!(details.flags.contains(gio::AskPasswordFlags::NEED_DOMAIN));
    assert!(details.flags.contains(gio::AskPasswordFlags::NEED_PASSWORD));
}

#[test]
fn volume_cancellation_is_quiet_and_terminal_errors_are_preserved() {
    let volume = Location::local("/mnt/USB");
    for kind in [gio::IOErrorEnum::Cancelled, gio::IOErrorEnum::FailedHandled] {
        assert_eq!(
            mount_failure_message(&volume, &glib::Error::new(kind, "Password dialog aborted")),
            None
        );
    }
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "Filesystem not supported");
    assert_eq!(
        mount_failure_message(&volume, &error),
        Some(error.to_string())
    );
    assert!(!volume_error_is_authentication_failure(&error));
    let error = glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Error unlocking: No key available with this passphrase.",
    );
    assert!(volume_error_is_authentication_failure(&error));
}

#[test]
fn pending_or_busy_volume_mount_is_not_a_terminal_error() {
    let pending = glib::Error::new(
        gio::IOErrorEnum::Pending,
        "A mount operation is already in progress",
    );
    let busy = glib::Error::new(gio::IOErrorEnum::Busy, "Volume is busy");
    let unlocking = glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Error unlocking /dev/loop0: Already unlocking",
    );
    let rejected = glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Error unlocking: No key available with this passphrase.",
    );

    for error in [&pending, &busy, &unlocking] {
        assert!(volume_error_is_in_flight_mount(error));
        assert!(!mount_error_is_cancelled(error));
    }
    assert!(!volume_error_is_in_flight_mount(&rejected));
    assert!(volume_error_is_authentication_failure(&rejected));
}

#[test]
fn external_volume_mount_completion_is_success() {
    let already_mounted = Err(glib::Error::new(
        gio::IOErrorEnum::AlreadyMounted,
        "Volume is already mounted",
    ));
    let failed = Err(glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Error unlocking /dev/loop0: Failed to activate device",
    ));
    let pending = Err(glib::Error::new(
        gio::IOErrorEnum::Pending,
        "A mount operation is already in progress",
    ));

    assert!(device_volume_mount_is_ready(&Ok(()), false));
    assert!(device_volume_mount_is_ready(&already_mounted, false));
    assert!(device_volume_mount_is_ready(&failed, true));
    assert!(!device_volume_mount_is_ready(&failed, false));
    assert!(!device_volume_mount_is_ready(&pending, false));
    assert!(device_volume_mount_is_ready(&pending, true));
}

#[test]
fn successor_identity_matches_across_crypto_replacement() {
    let locked = DeviceKeys::new(
        [Some("/dev/loop0".into()), Some("luks-uuid".into())],
        [Some("/dev/loop0".into())],
    );
    let unlocked = DeviceKeys::new(
        [Some("/dev/dm-0".into()), Some("fs-uuid".into())],
        [Some("/dev/loop0".into())],
    );
    let password_drive = DeviceKeys::new([], [Some("/dev/loop0".into())]);
    let other = DeviceKeys::new(
        [Some("/dev/sdb1".into()), Some("other-uuid".into())],
        [Some("/dev/sdb".into())],
    );
    assert!(identity_matches_mount(&locked, &unlocked, false));
    assert!(identity_matches_mount(&password_drive, &unlocked, false));
    assert!(!identity_matches_mount(&locked, &other, false));
    assert!(!identity_matches_mount(&password_drive, &other, false));
}

#[test]
fn sibling_partition_is_not_this_volume() {
    let luks = DeviceKeys::new(
        [Some("/dev/sdb2".into()), Some("luks-uuid".into())],
        [Some("/dev/sdb".into())],
    );
    let efi = DeviceKeys::new(
        [Some("/dev/sdb1".into()), Some("efi-uuid".into())],
        [Some("/dev/sdb".into())],
    );
    let nvme_luks = DeviceKeys::new(
        [Some("/dev/nvme0n1p3".into())],
        [Some("/dev/nvme0n1".into())],
    );
    let nvme_efi = DeviceKeys::new(
        [Some("/dev/nvme0n1p1".into())],
        [Some("/dev/nvme0n1".into())],
    );
    let password_drive = DeviceKeys::new([], [Some("/dev/sdb".into())]);
    let mapper = DeviceKeys::new(
        [Some("/dev/dm-0".into()), Some("fs-uuid".into())],
        [Some("/dev/sdb".into())],
    );
    assert!(
        !identity_matches_mount(&luks, &efi, true),
        "EFI sibling should not count as the LUKS volume being mounted"
    );
    assert!(
        !identity_matches_mount(&luks, &efi, false),
        "EFI sibling is not the LUKS successor after the locked volume is gone"
    );
    assert!(!identity_matches_volume(&luks, &efi));
    assert!(!device_volume_mount_is_ready(
        &Err(glib::Error::new(
            gio::IOErrorEnum::Failed,
            "Error unlocking /dev/sdb2: Failed to activate device",
        )),
        identity_matches_mount(&luks, &efi, true),
    ));
    assert!(!identity_matches_mount(&nvme_luks, &nvme_efi, true));
    assert!(!identity_matches_mount(&nvme_luks, &nvme_efi, false));
    assert!(
        identity_matches_mount(&luks, &mapper, false),
        "crypto replacement on the same drive should still match"
    );
    assert!(
        !identity_matches_mount(&password_drive, &efi, false),
        "an already-mounted EFI partition is not the password-drive unlock"
    );
    assert!(identity_matches_mount(
        &password_drive,
        &DeviceKeys::new([Some("/dev/dm-0".into())], [Some("/dev/sdb".into())]),
        false
    ));
}

#[test]
fn foreign_changed_without_mount_does_not_complete_wait() {
    assert!(!foreign_wait_changed_is_complete(false));
    assert!(foreign_wait_changed_is_complete(true));
    assert!(!foreign_drive_wait_changed_is_complete(
        VolumeSuccessorKind::Locked
    ));
    assert!(!foreign_drive_wait_changed_is_complete(
        VolumeSuccessorKind::Absent
    ));
    assert!(foreign_drive_wait_changed_is_complete(
        VolumeSuccessorKind::Mounted
    ));
    assert_eq!(
        foreign_volume_wait_follow_up(
            ForeignVolumeWaitOutcome::StillLocked,
            false,
            VolumeSuccessorKind::Locked,
        ),
        ForeignVolumeWaitFollowUp::StartOwnedMount
    );
}

#[test]
fn unlock_chrome_is_only_for_encrypted_targets() {
    assert_eq!(unlock_chrome_for_device(false), UnlockChrome::Connecting);
    assert_eq!(unlock_chrome_for_device(true), UnlockChrome::Unlocking);
}

#[test]
fn same_device_unlock_is_rejected_while_in_flight() {
    let mut slots = Vec::new();
    let luks = DeviceKeys::new(
        [Some("/dev/sdb2".into()), Some("luks-uuid".into())],
        [Some("/dev/sdb".into())],
    );
    let same_row = DeviceKeys::new([Some("/dev/sdb2".into())], [Some("/dev/sdb".into())]);
    let other = DeviceKeys::new([Some("/dev/sdc1".into())], [Some("/dev/sdc".into())]);
    assert!(begin_unlock_slot(&mut slots, &luks));
    assert!(!begin_unlock_slot(&mut slots, &luks));
    assert!(
        !begin_unlock_slot(&mut slots, &same_row),
        "row and padlock of the same volume should share in-flight"
    );
    assert!(begin_unlock_slot(&mut slots, &other));
    assert!(!unlock_progress_dismissed_for(&slots, &luks));
    slots
        .iter_mut()
        .find(|slot| unlock_target_matches(&slot.keys, &luks))
        .expect("should keep a slot for the hidden volume")
        .dismissed = true;
    assert!(unlock_progress_dismissed_for(&slots, &luks));
    assert!(!unlock_progress_dismissed_for(&slots, &other));
}

#[test]
fn unlock_retry_cancel_releases_in_flight() {
    crate::test_support::gtk_test(
        "ui::browser::location::tests::unlock_retry_cancel_releases_in_flight",
        || {
            let (view, window, overlay) = hosted_browser();
            let keys = DeviceKeys::new([Some("/dev/sdb2".into())], [Some("/dev/sdb".into())]);
            assert!(view.state.begin_unlock_progress(&keys));
            assert!(!view.state.begin_unlock_progress(&keys));
            view.state.show_unlock_retry_prompt(
                keys.clone(),
                None,
                MountPromptDetails {
                    message: "Enter a passphrase to unlock USB Backup".into(),
                    default_user: String::new(),
                    default_domain: String::new(),
                    flags: gio::AskPasswordFlags::NEED_PASSWORD,
                },
                |_| {},
            );
            descendants(&overlay.clone().upcast())
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Button>())
                .find(|button| button.label().as_deref() == Some("Cancel"))
                .expect("retry Cancel")
                .emit_clicked();
            assert!(
                view.state.begin_unlock_progress(&keys),
                "cancelling an incorrect-passphrase retry should allow another unlock"
            );
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn foreign_volume_wait_follow_up_covers_successor_states() {
    let cases = [
        (
            ForeignVolumeWaitOutcome::Mounted,
            false,
            VolumeSuccessorKind::Absent,
            ForeignVolumeWaitFollowUp::Navigate,
        ),
        (
            ForeignVolumeWaitOutcome::Mounted,
            true,
            VolumeSuccessorKind::Absent,
            ForeignVolumeWaitFollowUp::Navigate,
        ),
        (
            ForeignVolumeWaitOutcome::StillLocked,
            false,
            VolumeSuccessorKind::Locked,
            ForeignVolumeWaitFollowUp::StartOwnedMount,
        ),
        (
            ForeignVolumeWaitOutcome::StillLocked,
            true,
            VolumeSuccessorKind::Locked,
            ForeignVolumeWaitFollowUp::Quiet,
        ),
        (
            ForeignVolumeWaitOutcome::Gone,
            false,
            VolumeSuccessorKind::Mounted,
            ForeignVolumeWaitFollowUp::Navigate,
        ),
        (
            ForeignVolumeWaitOutcome::Gone,
            false,
            VolumeSuccessorKind::Locked,
            ForeignVolumeWaitFollowUp::StartOwnedMount,
        ),
        (
            ForeignVolumeWaitOutcome::Gone,
            false,
            VolumeSuccessorKind::Absent,
            ForeignVolumeWaitFollowUp::Quiet,
        ),
    ];
    for (outcome, already_waited, successor, expected) in cases {
        assert_eq!(
            foreign_volume_wait_follow_up(outcome, already_waited, successor),
            expected,
            "{outcome:?} waited={already_waited} successor={successor:?}"
        );
    }
}

#[test]
fn unlock_reloads_the_current_folder_without_stealing_another() {
    let mount = Location::local("/run/media/me/USB");
    let nested = Location::local("/run/media/me/USB/docs");
    let home = Location::local("/home/me");
    assert_eq!(
        unlock_view_follow_up(Some(&mount), &mount, true, false),
        UnlockViewFollowUp::Reload
    );
    assert_eq!(
        unlock_view_follow_up(Some(&nested), &mount, false, false),
        UnlockViewFollowUp::Reload
    );
    assert_eq!(
        unlock_view_follow_up(Some(&home), &mount, true, false),
        UnlockViewFollowUp::Navigate
    );
    assert_eq!(
        unlock_view_follow_up(Some(&home), &mount, false, false),
        UnlockViewFollowUp::None
    );
    assert_eq!(
        unlock_view_follow_up(None, &mount, true, false),
        UnlockViewFollowUp::Navigate
    );
    assert_eq!(
        unlock_view_follow_up(Some(&home), &mount, true, true),
        UnlockViewFollowUp::None
    );
    assert_eq!(
        unlock_view_follow_up(Some(&mount), &mount, true, true),
        UnlockViewFollowUp::Reload
    );
}

/// Hide dismisses the unlocking modal without aborting; a later present is a no-op.
#[test]
fn unlock_progress_dismiss_skips_navigation() {
    crate::test_support::gtk_test(
        "ui::browser::location::tests::unlock_progress_dismiss_skips_navigation",
        || {
            let (view, window, overlay) = hosted_browser();
            let keys = DeviceKeys::new([Some("/dev/sdb1".into())], [Some("/dev/sdb".into())]);
            assert!(view.state.begin_unlock_progress(&keys));
            view.state.present_unlock_progress(&keys, "USB Backup");
            let layer = modal_layer_on(&overlay).expect("unlock progress modal");
            descendants(&layer.clone().upcast())
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Button>())
                .find(|button| button.label().as_deref() == Some("Hide"))
                .expect("Hide")
                .emit_clicked();
            assert!(
                view.state
                    .unlock_slots
                    .borrow()
                    .iter()
                    .find(|slot| unlock_target_matches(&slot.keys, &keys))
                    .is_some_and(|slot| slot.view.is_none()),
            );
            assert!(
                unlock_progress_dismissed_for(&view.state.unlock_slots.borrow(), &keys),
                "Hide should keep unlock from jumping to the volume"
            );
            wait_until(
                || layer.parent().is_none(),
                "unlock progress modal did not dismiss",
            );
            view.state.present_unlock_progress(&keys, "USB Backup");
            assert!(
                modal_layer_on(&overlay).is_none(),
                "a dismissed unlock should not bring the unlocking modal back"
            );
            view.state.dismiss_unlock_progress(&keys);
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

/// Cancelling the delayed progress timer must not pop a modal after unlock
/// already finished.
#[test]
fn unlock_progress_schedule_cancelled_before_delay() {
    crate::test_support::gtk_test(
        "ui::browser::location::tests::unlock_progress_schedule_cancelled_before_delay",
        || {
            let (view, window, overlay) = hosted_browser();
            let keys = DeviceKeys::new([Some("/dev/sdb1".into())], [Some("/dev/sdb".into())]);
            assert!(view.state.begin_unlock_progress(&keys));
            view.state.schedule_unlock_progress(&keys, "USB Backup");
            assert!(modal_layer_on(&overlay).is_none());
            view.state.dismiss_unlock_progress(&keys);
            assert!(
                !unlock_progress_dismissed_for(&view.state.unlock_slots.borrow(), &keys),
                "finishing unlock before the modal appears is not a user dismiss"
            );
            let deadline = Instant::now() + UNLOCK_PROGRESS_DELAY + Duration::from_millis(150);
            while Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(
                modal_layer_on(&overlay).is_none(),
                "cancelled unlock progress should not appear after the delay"
            );
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn unlock_progress_hide_is_isolated_per_device() {
    crate::test_support::gtk_test(
        "ui::browser::location::tests::unlock_progress_hide_is_isolated_per_device",
        || {
            let (view, window, overlay) = hosted_browser();
            let keys_a = DeviceKeys::new([Some("/dev/sdb2".into())], [Some("/dev/sdb".into())]);
            let keys_b = DeviceKeys::new([Some("/dev/sdc1".into())], [Some("/dev/sdc".into())]);
            assert!(view.state.begin_unlock_progress(&keys_a));
            view.state.present_unlock_progress(&keys_a, "Volume A");
            let layer_a = modal_layer_on(&overlay).expect("A unlocking modal");
            descendants(&layer_a.clone().upcast())
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Button>())
                .find(|button| button.label().as_deref() == Some("Hide"))
                .expect("Hide")
                .emit_clicked();
            wait_until(
                || layer_a.parent().is_none(),
                "A unlocking modal did not hide",
            );
            assert!(unlock_progress_dismissed_for(
                &view.state.unlock_slots.borrow(),
                &keys_a
            ));
            assert!(view.state.begin_unlock_progress(&keys_b));
            view.state.present_unlock_progress(&keys_b, "Volume B");
            let layer_b = modal_layer_on(&overlay).expect("B unlocking modal");
            assert!(
                descendants(&layer_b.clone().upcast()).iter().any(|widget| {
                    widget
                        .downcast_ref::<gtk::Label>()
                        .is_some_and(|label| label.text().as_str().contains("Volume B"))
                }),
                "B should own the visible unlocking modal"
            );
            view.state.finish_unlock_slot(&keys_a);
            assert!(
                modal_layer_on(&overlay).is_some(),
                "finishing hidden A must not dismiss B's modal"
            );
            assert!(!unlock_progress_dismissed_for(
                &view.state.unlock_slots.borrow(),
                &keys_b
            ));
            view.state.finish_unlock_slot(&keys_b);
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn plain_volume_remount_does_not_present_unlock_chrome() {
    crate::test_support::gtk_test(
        "ui::browser::location::tests::plain_volume_remount_does_not_present_unlock_chrome",
        || {
            let (view, window, overlay) = hosted_browser();
            let keys = DeviceKeys::new([Some("/dev/sdb1".into())], [Some("/dev/sdb".into())]);
            assert!(view.state.begin_unlock_progress(&keys));
            view.state
                .schedule_device_mount_chrome(&keys, "USB Backup", false);
            let deadline = Instant::now() + UNLOCK_PROGRESS_DELAY + Duration::from_millis(150);
            while Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(
                modal_layer_on(&overlay).is_none(),
                "a plain USB remount should stay on Connecting, not Unlocking volume"
            );
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn encrypted_volume_presents_unlocking_chrome() {
    crate::test_support::gtk_test(
        "ui::browser::location::tests::encrypted_volume_presents_unlocking_chrome",
        || {
            let (view, window, overlay) = hosted_browser();
            let keys = DeviceKeys::new([Some("/dev/sdb2".into())], [Some("/dev/sdb".into())]);
            assert!(view.state.begin_unlock_progress(&keys));
            view.state
                .schedule_device_mount_chrome(&keys, "LUKS Backup", true);
            wait_until(
                || modal_layer_on(&overlay).is_some(),
                "encrypted unlock should present after the delay",
            );
            let layer = modal_layer_on(&overlay).expect("unlocking modal");
            assert!(
                descendants(&layer.upcast()).iter().any(|widget| {
                    widget
                        .downcast_ref::<gtk::Label>()
                        .is_some_and(|label| label.text().as_str() == "Unlocking volume")
                }),
                "encrypted unlock chrome should use the lock-icon unlocking title"
            );
            view.state.finish_unlock_slot(&keys);
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn password_only_volume_prompt_submits_and_cancels_the_original_operation() {
    crate::test_support::gtk_test(
        "ui::browser::location::tests::password_only_volume_prompt_submits_and_cancels_the_original_operation",
        || {
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            for submit in [true, false] {
                let operation = gio::MountOperation::new();
                let replies = Rc::new(RefCell::new(Vec::new()));
                let observed = replies.clone();
                operation.connect_reply(move |_, reply| observed.borrow_mut().push(reply));
                let prompt = show_authentication_dialog(
                    &overlay,
                    Some(&operation),
                    "Enter a passphrase to unlock USB Backup",
                    ("", ""),
                    gio::AskPasswordFlags::NEED_PASSWORD,
                    false,
                    MountDialogHandlers {
                        submitted: None,
                        cancelled: None,
                    },
                )
                .expect("themed volume prompt");
                let widgets = descendants(&prompt.clone().upcast());
                let password = widgets
                    .iter()
                    .find_map(|widget| widget.clone().downcast::<gtk::PasswordEntry>().ok())
                    .expect("password field");
                assert!(password.is_visible());
                assert!(!widgets.iter().any(|widget| widget.is::<gtk::Entry>()));
                password.set_text("fixture-passphrase");
                let button = widgets
                    .iter()
                    .filter_map(|widget| widget.downcast_ref::<gtk::Button>())
                    .find(|button| {
                        button.label().as_deref() == Some(if submit { "Connect" } else { "Cancel" })
                    })
                    .expect("prompt action");
                button.emit_clicked();
                assert_eq!(
                    *replies.borrow(),
                    vec![if submit {
                        gio::MountOperationResult::Handled
                    } else {
                        gio::MountOperationResult::Aborted
                    }]
                );
                if submit {
                    assert_eq!(operation.password().as_deref(), Some("fixture-passphrase"));
                    assert_eq!(operation.password_save(), gio::PasswordSave::Never);
                }
                dismiss_authentication_prompt(&overlay, &prompt);
            }
            window.destroy();
        },
    );
}

#[test]
fn breadcrumbs_render_full_labels_with_external_scroller() {
    crate::test_support::gtk_test(
        "ui::browser::location::tests::breadcrumbs_render_full_labels_with_external_scroller",
        || {
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let state = &view.state;
            state.set_location(&Location::local(
                "/usr/local/share/doc/very-long-project-folder-name-here",
            ));

            let breadcrumb_buttons: Vec<gtk::Button> =
                descendants(&state.breadcrumbs.clone().upcast())
                    .into_iter()
                    .filter_map(|w| w.downcast::<gtk::Button>().ok())
                    .collect();

            let ancestor_buttons: Vec<_> = breadcrumb_buttons
                .iter()
                .filter(|b| b.has_css_class("breadcrumb") && !b.has_css_class("copy-path"))
                .collect();
            assert!(
                !ancestor_buttons.is_empty(),
                "ancestor buttons should exist"
            );

            let current_labels: Vec<gtk::Label> = descendants(&state.breadcrumbs.clone().upcast())
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .filter(|l| l.has_css_class("current"))
                .collect();
            assert_eq!(current_labels.len(), 1);
            assert_eq!(
                current_labels[0].text(),
                "very-long-project-folder-name-here"
            );

            assert_eq!(
                state.breadcrumb_scroller.hscrollbar_policy(),
                gtk::PolicyType::External
            );
            assert_eq!(
                state.breadcrumb_scroller.vscrollbar_policy(),
                gtk::PolicyType::Never
            );
            assert!(
                state
                    .breadcrumb_scroller
                    .has_css_class("breadcrumb-scroller"),
                "scroller should have breadcrumb-scroller CSS class"
            );
        },
    );
}

#[test]
fn breadcrumb_adjustment_does_not_retain_widgets() {
    crate::test_support::gtk_test(
        "ui::browser::location::tests::breadcrumb_adjustment_does_not_retain_widgets",
        || {
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let adjustment = view.state.breadcrumb_scroller.hadjustment();
            let container = view
                .state
                .location_stack
                .child_by_name("breadcrumbs")
                .expect("breadcrumb container");
            let scrollbar = descendants(&container)
                .into_iter()
                .filter(|widget| widget.has_css_class("breadcrumb-scrollbar"))
                .find_map(|widget| widget.downcast::<gtk::Scrollbar>().ok())
                .expect("external scrollbar");
            let weak_scrollbar = scrollbar.downgrade();
            container
                .downcast::<gtk::Box>()
                .expect("vertical breadcrumb container")
                .remove(&scrollbar);
            drop(scrollbar);
            assert!(weak_scrollbar.upgrade().is_none());
            adjustment.set_value(1.0);
        },
    );
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        widgets.extend(descendants(&current));
        child = current.next_sibling();
    }
    widgets
}

fn hosted_browser() -> (BrowserView, gtk::Window, gtk::Overlay) {
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        crate::ui::browser::PeekBehavior::default(),
    );
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view.widget()));
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    (view, window, overlay)
}

fn modal_layer_on(overlay: &gtk::Overlay) -> Option<gtk::Box> {
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.is_visible() && widget.has_css_class("app-modal-layer") {
            return widget.downcast().ok();
        }
    }
    None
}

fn wait_until(condition: impl Fn() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}
