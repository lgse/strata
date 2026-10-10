// SPDX-License-Identifier: MIT

use super::*;
use gtk::subclass::prelude::ObjectSubclassIsExt;

pub(in crate::ui::window) fn focus_dialog_fixture(
    parent: &gtk::Widget,
    entry_focus: bool,
) -> (gtk::Box, gtk::Button) {
    let shell = modal_shell(
        parent,
        crate::assets::icons::HARD_DRIVE,
        "Drive dialog",
        "",
        "Continue",
        false,
    )
    .expect("drive modal host");
    wire_modal_close(&shell);
    if entry_focus {
        let entry = gtk::Entry::new();
        shell.layout.body.append(&entry);
        entry.grab_focus();
    }
    (shell.layer, shell.layout.cancel)
}

fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = Vec::new();
    let mut child = root.first_child();
    while let Some(widget) = child {
        widgets.extend(descendants(&widget));
        child = widget.next_sibling();
        widgets.push(widget);
    }
    widgets
}

fn has_label(root: &gtk::Widget, text: &str) -> bool {
    descendants(root).into_iter().any(|widget| {
        widget
            .downcast::<gtk::Label>()
            .is_ok_and(|label| label.text() == text)
    })
}

fn has_progress_status(root: &gtk::Widget, text: &str) -> bool {
    descendants(root).into_iter().any(|widget| {
        widget.has_css_class("job-status")
            && widget
                .downcast::<gtk::Label>()
                .is_ok_and(|label| label.text() == text)
    })
}

fn wait_until(ready: impl Fn() -> bool) {
    let expired = Rc::new(Cell::new(false));
    let timeout_expired = expired.clone();
    let timeout = glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
        timeout_expired.set(true);
    });
    while !ready() && !expired.get() {
        glib::MainContext::default().iteration(true);
    }
    if !expired.get() {
        timeout.remove();
    }
    assert!(ready(), "format feedback did not settle");
}

#[test]
fn format_feedback_reports_success_failure_and_authorization_cancellation() {
    crate::test_support::gtk_test(
        "ui::window::drive_dialogs::tests::format_feedback_reports_success_failure_and_authorization_cancellation",
        || {
            for (result, expected_title) in [
                (Ok(()), Some("Format complete")),
                (Err(drive_ops::DriveOpError::Cancelled), None),
                (
                    Err(drive_ops::DriveOpError::CommandFailed(
                        "Formatting failed for this test.".to_owned(),
                    )),
                    Some("Unable to update Test drive"),
                ),
            ] {
                let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let origin = gtk::Button::with_label("Browser item");
                parent.append(&origin);
                let overlay = gtk::Overlay::new();
                let blur = crate::ui::blur::BlurBin::new(&parent);
                overlay.set_child(Some(&blur));
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                origin.grab_focus();
                let (finish, pending) = futures_channel::oneshot::channel();
                let started = Rc::new(Cell::new(false));
                let task_started = started.clone();
                run_format_with_feedback(
                    &parent.upcast::<gtk::Widget>(),
                    "Test drive",
                    move |task_parent| async move {
                        assert!(task_parent.root().is_some());
                        task_started.set(true);
                        pending.await.expect("controlled format result")
                    },
                );
                wait_until(|| started.get());
                assert!(has_label(overlay.upcast_ref(), "Formatting drive"));
                assert!(has_progress_status(overlay.upcast_ref(), "…"));
                assert!(
                    !blur.imp().blurred.get(),
                    "format card must not blur browsing"
                );
                assert!(super::super::visible_modal_layer(&window).is_none());
                assert_eq!(
                    gtk::prelude::GtkWindowExt::focus(&window),
                    Some(origin.clone().upcast::<gtk::Widget>())
                );
                assert!(has_label(overlay.upcast_ref(), "Drive: Test drive"));
                let close = descendants(overlay.upcast_ref())
                    .into_iter()
                    .find_map(|widget| {
                        widget
                            .downcast::<gtk::Button>()
                            .ok()
                            .filter(|button| button.has_css_class("progress-cancel"))
                    })
                    .expect("format result close control");
                assert!(!close.is_visible());
                close.emit_clicked();
                assert!(has_label(overlay.upcast_ref(), "Formatting drive"));
                finish.send(result).expect("complete controlled format");
                if let Some(title) = expected_title {
                    wait_until(|| has_label(overlay.upcast_ref(), title));
                    if title == "Format complete" {
                        assert!(close.is_visible());
                        assert!(super::super::visible_modal_layer(&window).is_none());
                        assert!(has_label(
                            overlay.upcast_ref(),
                            "The drive was formatted successfully."
                        ));
                        assert!(!has_label(
                            overlay.upcast_ref(),
                            "Click the drive in the sidebar to mount it."
                        ));
                        assert!(has_progress_status(overlay.upcast_ref(), "100%"));
                        assert!(!has_label(overlay.upcast_ref(), "Done"));
                        let complete = descendants(overlay.upcast_ref())
                            .into_iter()
                            .find_map(|widget| {
                                widget
                                    .downcast::<gtk::Button>()
                                    .ok()
                                    .filter(|button| button.has_css_class("progress-complete"))
                            })
                            .expect("completed notification action");
                        complete.emit_clicked();
                    } else {
                        assert!(!has_label(overlay.upcast_ref(), "Format complete"));
                        assert!(has_label(
                            overlay.upcast_ref(),
                            "Formatting failed for this test."
                        ));
                        let error_close = descendants(overlay.upcast_ref())
                            .into_iter()
                            .find_map(|widget| {
                                widget.downcast::<gtk::Button>().ok().filter(|button| {
                                    button.label().as_deref() == Some("Close")
                                        && button.is_sensitive()
                                })
                            })
                            .expect("close format error");
                        error_close.emit_clicked();
                    }
                }
                wait_until(|| !has_label(overlay.upcast_ref(), "Formatting drive"));
                wait_until(|| {
                    !descendants(overlay.upcast_ref())
                        .iter()
                        .any(|widget| widget.has_css_class("app-modal-layer"))
                });
                assert!(!has_label(overlay.upcast_ref(), "Format complete"));
                assert_eq!(
                    gtk::prelude::GtkWindowExt::focus(&window),
                    Some(origin.clone().upcast::<gtk::Widget>())
                );
                window.close();
            }
        },
    );
}

#[test]
fn docked_format_blocks_window_close_until_the_worker_finishes() {
    crate::test_support::gtk_test(
        "ui::window::drive_dialogs::tests::docked_format_blocks_window_close_until_the_worker_finishes",
        || {
            let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let origin = gtk::Button::with_label("Browser item");
            parent.append(&origin);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&parent));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            origin.grab_focus();
            let (finish, pending) = futures_channel::oneshot::channel();
            run_format_with_feedback(
                parent.upcast_ref(),
                "Test drive",
                move |parent| async move {
                    let result = pending.await.expect("format worker stays alive");
                    assert!(parent.root().is_some());
                    result
                },
            );
            assert!(super::super::visible_modal_layer(&window).is_none());
            assert_eq!(
                gtk::prelude::GtkWindowExt::focus(&window),
                Some(origin.clone().upcast::<gtk::Widget>())
            );
            window.close();
            wait_until(|| has_label(overlay.upcast_ref(), "Drive formatting is still active"));
            assert!(window.is_visible());
            let error_close = descendants(overlay.upcast_ref())
                .into_iter()
                .find_map(|widget| {
                    widget.downcast::<gtk::Button>().ok().filter(|button| {
                        button.label().as_deref() == Some("Close") && button.is_sensitive()
                    })
                })
                .expect("close active-format warning");
            error_close.emit_clicked();
            finish
                .send(Ok(()))
                .expect("finish format without cancellation");
            wait_until(|| has_label(overlay.upcast_ref(), "Format complete"));
            let result_close = descendants(overlay.upcast_ref())
                .into_iter()
                .find_map(|widget| {
                    widget
                        .downcast::<gtk::Button>()
                        .ok()
                        .filter(|button| button.has_css_class("progress-cancel"))
                })
                .expect("close format result");
            assert!(result_close.is_visible());
            result_close.emit_clicked();
            window.close();
        },
    );
}

#[test]
fn capacity_summary_reports_used_bytes_and_fraction() {
    crate::test_support::gtk_test(
        "ui::window::drive_dialogs::tests::capacity_summary_reports_used_bytes_and_fraction",
        || {
            for (total, available, amount, fraction) in [
                (1000, 250, "750 B / 1 kB", 0.75),
                (1000, 0, "1 kB / 1 kB", 1.0),
                (1000, 1200, "0 B / 1 kB", 0.0),
                (0, 0, "0 B / 0 B", 0.0),
            ] {
                let summary = capacity_summary(total, available);
                assert_eq!(summary.amount.text(), amount);
                assert_eq!(summary.progress.fraction(), fraction);
            }
        },
    );
}

#[test]
fn enter_submits_strata_label_through_shared_validation() {
    crate::test_support::gtk_test(
        "ui::window::drive_dialogs::tests::enter_submits_strata_label_through_shared_validation",
        || {
            let field = FormTextField::with_character_limit(255);
            let confirm = gtk::Button::new();
            let error = inline_error();
            let submitted = Rc::new(Cell::new(0));
            let clicked_submitted = submitted.clone();
            confirm.connect_clicked(move |_| {
                clicked_submitted.set(clicked_submitted.get() + 1);
            });
            let changed_confirm = confirm.clone();
            let changed_error = error.clone();
            field.entry.connect_changed(move |entry| {
                refresh_label_validity(entry, "CURRENT", &changed_confirm, &changed_error);
            });
            wire_entry_submission(&field.entry, &confirm);

            field.entry.set_text("BACKUP");
            confirm.emit_clicked();
            assert_eq!(submitted.get(), 1);

            for (text, expected_error) in [
                ("CURRENT", None),
                (" CURRENT ", None),
                (
                    "BAD\tLABEL",
                    Some("Labels cannot contain control characters."),
                ),
            ] {
                field.entry.set_text(text);
                assert!(!confirm.is_sensitive());
                assert_eq!(error.is_visible(), expected_error.is_some());
                assert_eq!(field.entry.has_css_class("error"), expected_error.is_some());
                if let Some(message) = expected_error {
                    assert_eq!(error.text(), message);
                }
                field.entry.emit_activate();
                assert_eq!(
                    submitted.get(),
                    1,
                    "invalid Enter never activates submission"
                );
            }

            for text in ["", "  ", ".", "A/B", "A long Unicode label 📁"] {
                field.entry.set_text(text);
                assert!(confirm.is_sensitive());
                assert!(!error.is_visible());
                assert!(!field.entry.has_css_class("error"));
                field.entry.emit_activate();
            }
            assert_eq!(submitted.get(), 6);
        },
    );
}

#[test]
fn ntfs_install_guidance_uses_the_native_utilities_package() {
    let tool = required_drive_tool(
        FilesystemType::Ntfs,
        FilesystemType::Ntfs.format_tool_name(),
    );
    for (manager, expected) in [
        (PackageManager::Pacman, "sudo pacman -S --needed ntfsprogs"),
        (PackageManager::Apt, "sudo apt install ntfs-3g"),
    ] {
        let package = tool
            .packages
            .iter()
            .find_map(|(candidate, package)| (*candidate == manager).then_some(*package))
            .expect("native package for NTFS utilities");
        assert_eq!(
            manager.install_command(&[package]).as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn strata_labels_allow_clearing_and_do_not_require_filesystem_tools() {
    crate::test_support::gtk_test(
        "ui::window::drive_dialogs::tests::strata_labels_allow_clearing_and_do_not_require_filesystem_tools",
        || {
            let preferences = super::super::super::preferences::PreferenceManager::shared();
            let id = "volume:strata-label-dialog-fixture";
            preferences.set_device_label(id, "Existing label");
            let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&crate::ui::blur::BlurBin::new(&parent)));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            for (text, save, expected) in [
                ("Cancelled label", false, Some("Existing label")),
                ("  Photos / 📁  ", true, Some("Photos / 📁")),
                ("", true, None),
            ] {
                show_label_dialog_for_identity(
                    parent.upcast_ref(),
                    id,
                    "System device name",
                    preferences.clone(),
                );
                let layer =
                    super::super::visible_modal_layer(&window).expect("Strata label dialog");
                let entry = descendants(layer.upcast_ref())
                    .into_iter()
                    .find_map(|widget| widget.downcast::<gtk::Entry>().ok())
                    .expect("label entry");
                assert_eq!(
                    entry.placeholder_text().as_deref(),
                    Some("System device name")
                );
                assert_eq!(
                    entry.text().as_str(),
                    preferences.device_label(id).as_deref().unwrap_or("")
                );
                entry.set_text(text);
                if save {
                    entry.emit_activate();
                } else {
                    let cancel = descendants(layer.upcast_ref())
                        .into_iter()
                        .find_map(|widget| {
                            widget
                                .downcast::<gtk::Button>()
                                .ok()
                                .filter(|button| button.label().as_deref() == Some("Cancel"))
                        })
                        .expect("cancel action");
                    cancel.emit_clicked();
                }
                wait_until(|| super::super::visible_modal_layer(&window).is_none());
                assert_eq!(preferences.device_label(id).as_deref(), expected);
            }
            show_label_dialog_for_identity(
                parent.upcast_ref(),
                id,
                "System device name",
                preferences.clone(),
            );
            let layer = super::super::visible_modal_layer(&window).expect("label editor");
            let widgets = descendants(layer.upcast_ref());
            let entry = widgets
                .iter()
                .find_map(|widget| widget.clone().downcast::<gtk::Entry>().ok())
                .expect("draft entry");
            let save = widgets
                .iter()
                .find_map(|widget| {
                    widget
                        .clone()
                        .downcast::<gtk::Button>()
                        .ok()
                        .filter(|button| button.label().as_deref() == Some("Save label"))
                })
                .expect("save action");
            entry.set_text("Draft label");
            preferences.set_device_label(id, "Elsewhere");
            assert_eq!(entry.text(), "Draft label");
            assert!(save.is_sensitive());
            preferences.set_device_label(id, "Draft label");
            assert!(!save.is_sensitive());
            preferences.set_device_label(id, "Different label");
            assert!(save.is_sensitive());
            entry.emit_activate();
            assert_eq!(preferences.device_label(id).as_deref(), Some("Draft label"));
            preferences.set_device_label(id, "");
            window.close();
        },
    );
}

#[test]
fn storage_properties_separate_strata_labels_from_formatting() {
    crate::test_support::gtk_test(
        "ui::window::drive_dialogs::tests::storage_properties_separate_strata_labels_from_formatting",
        || {
            for (editable, can_label, can_release, expected) in [
                (false, true, true, [1, 1, 0]),
                (false, false, true, [0, 1, 0]),
                (true, true, true, [1, 1, 1]),
                (false, true, false, [1, 0, 0]),
                (false, false, false, [0, 0, 0]),
            ] {
                let invoked: [Rc<Cell<usize>>; 3] = std::array::from_fn(|_| Rc::new(Cell::new(0)));
                for (index, action) in [
                    PropertyAction::Label,
                    PropertyAction::Eject,
                    PropertyAction::Format,
                ]
                .into_iter()
                .enumerate()
                {
                    if let Some(button) =
                        property_action_control(action, editable, can_release, can_label)
                    {
                        let invoked = invoked[index].clone();
                        button.connect_clicked(move |_| invoked.set(invoked.get() + 1));
                        button.emit_clicked();
                    }
                }
                assert_eq!(invoked.map(|count| count.get()), expected);
            }
        },
    );
}

#[test]
fn format_filesystem_selection_updates_label_limit_and_validity() {
    crate::test_support::gtk_test(
        "ui::window::drive_dialogs::tests::format_filesystem_selection_updates_label_limit_and_validity",
        || {
            let field = FormTextField::with_character_limit(11);
            let confirm = gtk::Button::new();
            let available = [
                FilesystemType::Fat32,
                FilesystemType::Ntfs,
                FilesystemType::Exfat,
            ];

            for (fs, expected_text) in [
                (FilesystemType::Fat32, "abcdefghijk"),
                (FilesystemType::Ntfs, "abcdefghijklmnop"),
                (FilesystemType::Exfat, "abcdefghijklmnop"),
            ] {
                let selector = format_filesystem_selector(&available, Some(fs.label()));
                let selected = selector
                    .selected_item()
                    .and_downcast::<gtk::StringObject>()
                    .expect("selected filesystem");
                assert_eq!(selected.string(), format!("{} (current)", fs.label()));
                let selected_fs = available.get(selector.selected() as usize).copied();
                refresh_format_selection(selected_fs, true, &field.entry, &confirm);
                field.entry.set_text("abcdefghijklmnop");
                refresh_format_selection(selected_fs, true, &field.entry, &confirm);
                assert_eq!(field.entry.text(), expected_text);
                assert!(confirm.is_sensitive());

                refresh_format_selection(selected_fs, false, &field.entry, &confirm);
                assert!(!confirm.is_sensitive());
                assert_eq!(field.entry.text(), expected_text);
            }

            let unsupported = format_filesystem_selector(&available, Some("ext4"));
            assert_eq!(
                unsupported
                    .selected_item()
                    .and_downcast::<gtk::StringObject>()
                    .expect("fallback filesystem")
                    .string(),
                "FAT32"
            );
            refresh_format_selection(
                available.get(unsupported.selected() as usize).copied(),
                true,
                &field.entry,
                &confirm,
            );
            assert_eq!(field.entry.text(), "abcdefghijk");
            assert!(confirm.is_sensitive());

            let empty_combo = gtk::DropDown::from_strings(&[]);
            refresh_format_validity(&[], &empty_combo, &field.entry, &confirm);
            assert!(!confirm.is_sensitive());
        },
    );
}
