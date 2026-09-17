// SPDX-License-Identifier: MIT

use super::*;

fn empty_device_dialog() -> (gtk::Overlay, gtk::Window) {
    use gtk::prelude::*;

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    (overlay, window)
}

fn button_with_label(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    use gtk::prelude::*;

    if let Ok(button) = root.clone().downcast::<gtk::Button>()
        && button.label().as_deref() == Some(label)
    {
        return Some(button);
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(button) = button_with_label(&widget, label) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn volume_release_prefers_eject_and_hides_fixed_disks() {
    assert_eq!(
        volume_release_action(true, false, false),
        Some(MediaRelease::EjectVolume)
    );
    assert_eq!(
        volume_release_action(true, true, true),
        Some(MediaRelease::EjectVolume)
    );
    assert_eq!(
        volume_release_action(false, true, false),
        Some(MediaRelease::EjectMount)
    );
    assert_eq!(
        volume_release_action(false, true, true),
        Some(MediaRelease::EjectMount)
    );
    assert_eq!(
        volume_release_action(false, false, true),
        Some(MediaRelease::UnmountMount)
    );
    assert_eq!(volume_release_action(false, false, false), None);
}

#[test]
fn encrypted_device_actions_share_lock_and_release() {
    let locked = device_row_actions(true, false, false, false, false);
    assert_eq!(locked.encrypted, Some(EncryptedMediaAction::Unlock));
    assert_eq!(locked.release, None);
    assert_eq!(
        device_row_actions(true, false, true, false, false).release,
        Some(MediaRelease::EjectVolume)
    );

    let unlocked = device_row_actions(true, true, false, false, true);
    assert_eq!(unlocked.encrypted, Some(EncryptedMediaAction::Lock));
    assert_eq!(unlocked.release, Some(MediaRelease::UnmountMount));

    let usb = device_row_actions(false, true, false, false, true);
    assert_eq!(usb.encrypted, None);
    assert_eq!(usb.release, Some(MediaRelease::UnmountMount));

    let shutdown = device_row_actions(false, false, false, false, false);
    assert_eq!(shutdown.encrypted, None);
    assert_eq!(shutdown.release, None);
}

#[test]
fn forget_password_prompt_routes() {
    gtk_test(
        "ui::window::tests::devices::forget_password_prompt_routes",
        || {
            use gtk::prelude::*;

            let (overlay, window) = empty_device_dialog();

            let locked = Rc::new(Cell::new(false));
            let cancelled = Rc::new(Cell::new(false));
            continue_encrypted_lock(
                overlay.upcast_ref(),
                "STRATA-537",
                false,
                {
                    let locked = locked.clone();
                    move || locked.set(true)
                },
                {
                    let cancelled = cancelled.clone();
                    move || cancelled.set(true)
                },
            );
            assert!(locked.get(), "uncached lock should proceed immediately");
            assert!(!cancelled.get(), "uncached lock should not cancel");
            assert!(
                button_with_label(overlay.upcast_ref(), "Forget and lock").is_none(),
                "uncached lock should not open a confirmation"
            );
            window.destroy();

            for confirm in [true, false] {
                let (overlay, window) = empty_device_dialog();
                let locked = Rc::new(Cell::new(false));
                let cancelled = Rc::new(Cell::new(false));
                confirm_forget_cached_password(
                    overlay.upcast_ref(),
                    "STRATA-537",
                    {
                        let locked = locked.clone();
                        move || locked.set(true)
                    },
                    {
                        let cancelled = cancelled.clone();
                        move || cancelled.set(true)
                    },
                );
                let label = if confirm { "Forget and lock" } else { "Cancel" };
                button_with_label(overlay.upcast_ref(), label)
                    .unwrap_or_else(|| panic!("confirmation should offer {label}"))
                    .emit_clicked();
                assert_eq!(locked.get(), confirm, "{label} should lock only on confirm");
                assert_eq!(
                    cancelled.get(),
                    !confirm,
                    "{label} should cancel only on cancel"
                );
                window.destroy();
            }
        },
    );
}

#[test]
fn emblemed_padlock_icon_names_include_emblem() {
    gtk_test(
        "ui::window::tests::devices::emblemed_padlock_icon_names_include_emblem",
        || {
            use gtk::gio;
            use gtk::prelude::*;
            let base = gio::ThemedIcon::new("drive-harddisk-usb");
            let padlock = gio::ThemedIcon::new("changes-prevent");
            let emblem = gio::Emblem::new(&padlock);
            let emblemed = gio::EmblemedIcon::new(&base, Some(&emblem));
            let names = super::gio_icon_names(emblemed.upcast_ref());
            assert!(
                names.iter().any(|name| name == "drive-harddisk-usb"),
                "base drive icon should remain"
            );
            assert!(
                names.iter().any(|name| name == "changes-prevent"),
                "GVfs padlock emblem should be collected"
            );
        },
    );
}

#[test]
fn device_controls_dispatch_independently() {
    gtk_test(
        "ui::window::tests::devices::device_controls_dispatch_independently",
        || {
            use gtk::prelude::*;

            let opened = Rc::new(Cell::new(0));
            let unlocked = Rc::new(Cell::new(0));
            let ejected = Rc::new(Cell::new(0));
            let row = super::sidebar_button(crate::assets::icons::HARD_DRIVE, "USB Backup");
            row.connect_clicked({
                let opened = opened.clone();
                move |_| opened.set(opened.get() + 1)
            });
            let lock = super::sidebar_lock_button(EncryptedMediaAction::Unlock, {
                let unlocked = unlocked.clone();
                move || unlocked.set(unlocked.get() + 1)
            });
            let eject = super::sidebar_eject_button(MediaRelease::EjectVolume, {
                let ejected = ejected.clone();
                move || ejected.set(ejected.get() + 1)
            });
            let _shell = super::sidebar_device_row(&row, Some(&lock), Some(&eject));
            lock.emit_clicked();
            assert_eq!((opened.get(), unlocked.get(), ejected.get()), (0, 1, 0));
            eject.emit_clicked();
            assert_eq!((opened.get(), unlocked.get(), ejected.get()), (0, 1, 1));
            row.emit_clicked();
            assert_eq!((opened.get(), unlocked.get(), ejected.get()), (1, 1, 1));
        },
    );
}

#[test]
fn unsupported_unmounted_lock_fails_before_password_lookup() {
    gtk_test(
        "ui::window::tests::devices::unsupported_unmounted_lock_fails_before_password_lookup",
        || {
            use gtk::prelude::*;

            let view = browser_for_window();
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&view.widget()));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            let in_flight = Rc::new(Cell::new(false));
            super::request_encrypted_lock(
                overlay.upcast_ref(),
                "USB Backup",
                Some("6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7".into()),
                None,
                None,
                &view.browser(),
                &in_flight,
            );
            assert!(!in_flight.get());
            assert!(button_with_label(overlay.upcast_ref(), "Forget and lock").is_none());
            button_with_label(overlay.upcast_ref(), "Close")
                .expect("unsupported lock reports an error immediately")
                .emit_clicked();
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn device_selection_marks_the_full_row_shell() {
    gtk_test(
        "ui::window::tests::devices::device_selection_marks_the_full_row_shell",
        || {
            use gtk::prelude::*;

            let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let home = super::sidebar_button(crate::assets::icons::HOME, "Home");
            let row = super::sidebar_button(crate::assets::icons::HARD_DRIVE, "STRATA-537");
            let lock = super::sidebar_lock_button(EncryptedMediaAction::Unlock, || {});
            let eject = super::sidebar_eject_button(MediaRelease::EjectVolume, || {});
            let shell = super::sidebar_device_row(&row, Some(&lock), Some(&eject));
            sidebar.append(&home);
            sidebar.append(&shell);

            super::select_sidebar_row(&sidebar, &row);
            assert!(row.has_css_class("active"));
            assert!(shell.has_css_class("active"));
            assert!(!home.has_css_class("active"));

            super::select_sidebar_row(&sidebar, &home);
            assert!(home.has_css_class("active"));
            assert!(!row.has_css_class("active"));
            assert!(!shell.has_css_class("active"));
        },
    );
}

#[test]
fn device_focus_marks_the_full_row_shell() {
    gtk_test(
        "ui::window::tests::devices::device_focus_marks_the_full_row_shell",
        || {
            use std::time::Duration;

            use gtk::prelude::*;

            let row = super::sidebar_button(crate::assets::icons::HARD_DRIVE, "STRATA-537");
            let lock = super::sidebar_lock_button(EncryptedMediaAction::Unlock, || {});
            let eject = super::sidebar_eject_button(MediaRelease::EjectVolume, || {});
            let shell = super::sidebar_device_row(&row, Some(&lock), Some(&eject));
            let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 2);
            sidebar.append(&shell);
            let window = gtk::Window::builder()
                .default_width(SIDEBAR_WIDTH)
                .default_height(80)
                .child(&sidebar)
                .build();
            window.present();
            let main_loop = glib::MainLoop::new(None, false);
            let stop = main_loop.clone();
            glib::timeout_add_local_once(Duration::from_millis(100), move || stop.quit());
            main_loop.run();

            assert!(row.grab_focus(), "device name should take focus");
            assert!(
                shell.has_css_class("focused"),
                "focus outline should use the full device shell"
            );

            assert!(lock.grab_focus(), "lock action should take focus");
            assert!(
                !shell.has_css_class("focused"),
                "lock focus should not outline the full device row"
            );

            window.destroy();
        },
    );
}

#[test]
fn media_release_labels_match_nautilus_wording() {
    assert_eq!(media_release_label(MediaRelease::EjectVolume), "Eject");
    assert_eq!(media_release_label(MediaRelease::EjectMount), "Eject");
    assert_eq!(media_release_label(MediaRelease::UnmountMount), "Unmount");
}

#[test]
fn media_release_guard_rejects_repeated_actions_until_completion() {
    let in_flight = Cell::new(false);

    assert!(begin_media_release(&in_flight));
    assert!(!begin_media_release(&in_flight));

    in_flight.set(false);
    assert!(begin_media_release(&in_flight));
}
