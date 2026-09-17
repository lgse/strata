// SPDX-License-Identifier: MIT

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

use super::{UdiskieIntegrationStatus, settings_row};
use crate::portal_setup::{FileManagerStatus, PortalStatus, udiskie::UdiskieStatus};
use crate::test_support::{gtk_test, gtk_test_with_env};
use crate::ui::desktop_integration::SETUP_RUNNING;
use crate::ui::portal_preferences::SettingsIntegrationStatus;

fn descendants<T: IsA<gtk::Widget> + Clone>(root: &gtk::Widget) -> Vec<T> {
    let mut widgets = Vec::new();
    if let Ok(widget) = root.clone().downcast::<T>() {
        widgets.push(widget);
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        widgets.extend(descendants::<T>(&widget));
        child = widget.next_sibling();
    }
    widgets
}

fn summary_on(row: &gtk::Box) -> std::rc::Rc<UdiskieIntegrationStatus> {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.append(&content);
    UdiskieIntegrationStatus::new(&content, row)
}

fn udiskie_paths() -> (PathBuf, PathBuf) {
    (
        glib::user_config_dir().join("udiskie/config.yml"),
        glib::user_data_dir().join("strata/udiskie-install/state.toml"),
    )
}

fn seed_omarchy() {
    let directory = glib::home_dir().join(".local/share/omarchy");
    fs::create_dir_all(&directory).expect("Omarchy version directory");
    fs::write(directory.join("version"), "4.0\n").expect("Omarchy version");
}

fn write_udiskie_stub(bin: &Path) {
    let path = bin.join("udiskie");
    fs::write(&path, "#!/bin/sh\nexit 0\n").expect("udiskie stub");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("executable udiskie");
}

fn wait_until(predicate: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let context = glib::MainContext::default();
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        context.iteration(true);
    }
}

fn has_action_label(row: &gtk::Box, label: &str) -> bool {
    descendants::<gtk::Button>(row.upcast_ref())
        .iter()
        .any(|button| button.label().as_deref() == Some(label))
}

#[test]
fn actions_follow_status_and_hide_unknown_indicator() {
    gtk_test(
        "ui::udiskie_preferences::tests::actions_follow_status_and_hide_unknown_indicator",
        || {
            crate::ui::prepare_portal_ui();
            let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let summary = summary_on(&row);
            let use_strata = summary.use_strata.upgrade().expect("Use Strata action");
            let restore = summary.restore.upgrade().expect("Restore default action");
            let indicator = summary.indicator.row.upgrade().expect("status row");
            summary.show_result(Ok(UdiskieStatus {
                available: true,
                configured: false,
                has_installation: true,
            }));
            assert!(
                row.is_visible(),
                "available status should show the Settings row"
            );
            assert!(use_strata.is_visible());
            assert!(restore.is_visible());
            assert!(indicator.is_visible());
            summary.show_result(Ok(UdiskieStatus {
                available: true,
                configured: true,
                has_installation: false,
            }));
            assert!(row.is_visible());
            assert!(!use_strata.is_visible());
            assert!(!restore.is_visible());
            summary.show_result(Ok(UdiskieStatus {
                available: false,
                configured: false,
                has_installation: false,
            }));
            assert!(
                !row.is_visible(),
                "unavailable status should hide the Settings row"
            );
            summary.show_result(Ok(UdiskieStatus {
                available: true,
                configured: false,
                has_installation: true,
            }));
            assert!(
                row.is_visible(),
                "availability should follow later status polls without rebuilding"
            );
            SETUP_RUNNING.set(true);
            summary.show_result(Ok(UdiskieStatus {
                available: true,
                configured: false,
                has_installation: true,
            }));
            assert!(!use_strata.is_sensitive());
            assert!(!restore.is_sensitive());
            SETUP_RUNNING.set(false);
            summary.show_result(Err("configuration is unreadable".into()));
            assert!(
                !indicator.is_visible(),
                "unknown status should hide the indicator"
            );
            assert!(!use_strata.is_sensitive());
            assert!(!restore.is_sensitive());
            assert!(
                summary
                    .message
                    .upgrade()
                    .expect("inline error")
                    .is_visible()
            );
        },
    );
}

#[test]
fn shared_busy_flag_blocks_portal_and_udiskie() {
    gtk_test(
        "ui::udiskie_preferences::tests::shared_busy_flag_blocks_portal_and_udiskie",
        || {
            crate::ui::prepare_portal_ui();
            let portal_parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let udiskie_row = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let portal = SettingsIntegrationStatus::new(&portal_parent);
            let udiskie = summary_on(&udiskie_row);
            portal.show_result(Ok((
                PortalStatus {
                    configured: true,
                    has_installation: true,
                },
                FileManagerStatus {
                    default: true,
                    has_service: true,
                    has_installation: true,
                    shortcuts: Some(true),
                },
            )));
            udiskie.show_result(Ok(UdiskieStatus {
                available: true,
                configured: false,
                has_installation: false,
            }));
            SETUP_RUNNING.set(true);
            portal
                .complete
                .upgrade()
                .expect("Complete setup")
                .emit_clicked();
            udiskie
                .use_strata
                .upgrade()
                .expect("Use Strata")
                .emit_clicked();
            assert!(
                portal
                    .message
                    .upgrade()
                    .expect("portal message")
                    .is_visible(),
                "busy portal action should surface a refusal"
            );
            assert!(
                udiskie
                    .message
                    .upgrade()
                    .expect("udiskie message")
                    .is_visible(),
                "busy udiskie action should surface a refusal"
            );
            let (config, state) = udiskie_paths();
            assert!(
                !config.exists(),
                "refused Use Strata should not write udiskie config.yml"
            );
            assert!(
                !state.exists(),
                "refused Use Strata should not write udiskie state.toml"
            );
            SETUP_RUNNING.set(false);
        },
    );
}

#[test]
fn settings_row_open_does_not_write_config() {
    gtk_test(
        "ui::udiskie_preferences::tests::settings_row_open_does_not_write_config",
        || {
            crate::ui::prepare_portal_ui();
            let (config, state) = udiskie_paths();
            let _row = settings_row();
            assert!(
                !config.exists(),
                "constructing the row should not write {config:?}"
            );
            assert!(
                !state.exists(),
                "constructing the row should not write {state:?}"
            );
        },
    );
}

#[test]
fn settings_row_polls_availability_while_hidden() {
    let bin = tempfile::tempdir().expect("isolated PATH");
    gtk_test_with_env(
        "ui::udiskie_preferences::tests::settings_row_polls_availability_while_hidden",
        [("PATH", bin.path())],
        || {
            crate::ui::prepare_portal_ui();
            seed_omarchy();
            let bin = PathBuf::from(std::env::var_os("PATH").expect("isolated PATH"));
            let row = settings_row();
            assert!(
                !row.is_visible(),
                "row should start hidden before status returns"
            );
            wait_until(
                || has_action_label(&row, "Use Strata"),
                "construction reload to finish",
            );
            assert!(
                !row.is_visible(),
                "row should stay hidden without udiskie on PATH"
            );
            write_udiskie_stub(&bin);
            wait_until(|| row.is_visible(), "hidden poll to see udiskie on PATH");
            fs::remove_file(bin.join("udiskie")).expect("remove udiskie stub");
            wait_until(
                || !row.is_visible(),
                "poll to hide the row after udiskie leaves PATH",
            );
            let (config, state) = udiskie_paths();
            assert!(
                !config.exists(),
                "availability polling should not write {config:?}"
            );
            assert!(
                !state.exists(),
                "availability polling should not write {state:?}"
            );
        },
    );
}
