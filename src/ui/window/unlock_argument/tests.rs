// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use super::*;
use crate::adapters::{LocalFileSource, LocalOperationProvider};
use crate::model::Location;
use crate::ui::browser::PeekBehavior;
use crate::ui::preferences::PreferenceManager;
use crate::ui::window::home_directory;
use crate::ui::window::open_argument::status_widget;

const HYPHENATED_UUID: &str = "6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7";
const COMPACT_UUID: &str = "6e5d75a7e4e24c7d9c1c8e5a5e5d75a7";

fn view() -> BrowserView {
    let view = BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default());
    view.set_operation_provider(Rc::new(LocalOperationProvider));
    let window = gtk::Window::builder().child(&view.widget()).build();
    view.connect_navigation_cleanup(&window);
    let browser = view.browser();
    window.connect_destroy(move |_| {
        browser.bump_navigation_generation();
        browser.clear_observer();
    });
    window.present();
    view
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "operation timed out");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn button_with_label(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Ok(button) = widget.clone().downcast::<gtk::Button>()
        && button.label().as_deref() == Some(label)
    {
        return Some(button);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(button) = button_with_label(&widget, label) {
            return Some(button);
        }
    }
    None
}

fn missing_target() -> UnlockTarget {
    UnlockTarget::parse("/dev/strata-missing-unlock-volume").expect("unix device operand")
}

fn encrypted_volume(
    unix_device: &str,
    uuid: Option<&str>,
    crypto_uuid: Option<&str>,
) -> VolumeUnlockIdentity {
    VolumeUnlockIdentity {
        unix_device: Some(unix_device.to_owned()),
        uuid: uuid.map(str::to_owned),
        crypto_uuid: crypto_uuid.map(str::to_owned),
        encrypted: true,
    }
}

fn unencrypted_volume(unix_device: &str, uuid: Option<&str>) -> VolumeUnlockIdentity {
    VolumeUnlockIdentity {
        unix_device: Some(unix_device.to_owned()),
        uuid: uuid.map(str::to_owned),
        crypto_uuid: None,
        encrypted: false,
    }
}

#[test]
fn identities_unix_device_uuid_and_not_encrypted() {
    let target = |operand: &str| UnlockTarget::parse(operand).expect("unlock operand");
    let password_drive = [PasswordDriveIdentity {
        unix_device: Some("/dev/sdb".to_owned()),
        uuid: Some(COMPACT_UUID.to_owned()),
    }];
    assert_eq!(
        classify_unlock_identities(
            &target("/dev/sdb1"),
            &[
                unencrypted_volume("/dev/sda1", None),
                encrypted_volume("/dev/sdb1", Some(HYPHENATED_UUID), None),
            ],
            &[],
        ),
        UnlockResolution::EncryptedVolume(1),
        "unix-device should match the encrypted volume"
    );
    assert_eq!(
        classify_unlock_identities(
            &target(COMPACT_UUID),
            &[encrypted_volume("/dev/sdb1", Some(HYPHENATED_UUID), None)],
            &[],
        ),
        UnlockResolution::EncryptedVolume(0),
        "compact UUID should match a hyphenated volume UUID"
    );
    assert_eq!(
        classify_unlock_identities(
            &target(COMPACT_UUID),
            &[encrypted_volume("/dev/sdb1", None, Some(HYPHENATED_UUID))],
            &[],
        ),
        UnlockResolution::EncryptedVolume(0),
        "compact UUID should match crypto_uuid"
    );
    assert_eq!(
        classify_unlock_identities(&target("/dev/sdb"), &[], &password_drive),
        UnlockResolution::PasswordDrive(0),
        "unix-device should match a password drive"
    );
    assert_eq!(
        classify_unlock_identities(&target(HYPHENATED_UUID), &[], &password_drive),
        UnlockResolution::PasswordDrive(0),
        "UUID should match a password drive"
    );
    assert_eq!(
        classify_unlock_identities(
            &target("/dev/sdb1"),
            &[unencrypted_volume("/dev/sdb1", Some(HYPHENATED_UUID))],
            &[],
        ),
        UnlockResolution::NotEncrypted,
        "matching unencrypted volume should not unlock"
    );
    assert_eq!(
        classify_unlock_identities(
            &target("/dev/sdb1"),
            &[
                encrypted_volume("/dev/sdc1", Some(HYPHENATED_UUID), None),
                unencrypted_volume("/dev/sdb1", None),
            ],
            &[],
        ),
        UnlockResolution::NotEncrypted,
        "unix-device match should win over a different volume's UUID"
    );
}

#[test]
fn match_hides_connecting_overlay() {
    crate::test_support::gtk_test(
        "ui::window::unlock_argument::tests::match_hides_connecting_overlay",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let browser = view();
            show_connecting_overlay(&browser, CONNECTING_MESSAGE, || {});
            assert!(
                status_widget(&browser.overlay()).is_some(),
                "connecting overlay should be visible before a match"
            );

            let request = UnlockRequest::new(gio::VolumeMonitor::get());
            complete_matched_unlock(&browser, &request, ResolvedUnlock::Unmatched);

            assert!(
                status_widget(&browser.overlay()).is_none(),
                "match success should hide waiting chrome without painting an error overlay"
            );
            assert!(
                !request.active.get(),
                "match success should finish the appearance wait"
            );
        },
    );
}

#[test]
fn wait_timeout_not_found() {
    crate::test_support::gtk_test(
        "ui::window::unlock_argument::tests::wait_timeout_not_found",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let browser = view();
            start_unlock_wait(
                browser.clone(),
                missing_target(),
                Duration::from_millis(5),
                Duration::from_millis(40),
            );
            wait_until(|| status_widget(&browser.overlay()).is_some());
            wait_until(|| {
                status_widget(&browser.overlay())
                    .is_some_and(|status| button_with_label(&status, "Cancel").is_none())
            });
        },
    );
}

#[test]
fn wait_cancel_leaves_default_directory() {
    crate::test_support::gtk_test(
        "ui::window::unlock_argument::tests::wait_cancel_leaves_default_directory",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let browser = view();
            let home = Location::local(home_directory());
            browser.navigate_location(home.clone());
            wait_until(|| {
                browser
                    .browser()
                    .active_location()
                    .is_some_and(|location| location.native_path() == home.native_path())
            });

            start_unlock_wait(
                browser.clone(),
                missing_target(),
                Duration::from_millis(10),
                Duration::from_secs(5),
            );
            wait_until(|| {
                status_widget(&browser.overlay())
                    .as_ref()
                    .and_then(|status| button_with_label(status, "Cancel"))
                    .is_some()
            });
            let status = status_widget(&browser.overlay()).expect("connecting status");
            button_with_label(&status, "Cancel")
                .expect("cancel button")
                .emit_clicked();

            pump_for(Duration::from_millis(50));
            assert!(status_widget(&browser.overlay()).is_none());
            let active = browser
                .browser()
                .active_location()
                .expect("default directory");
            assert_eq!(active.native_path(), home.native_path());
        },
    );
}
