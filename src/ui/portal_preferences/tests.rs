// SPDX-License-Identifier: MIT

use super::*;

#[test]
#[ignore = "requires a GTK display and isolated XDG directories; run this test alone"]
fn inline_setup_actions_follow_status_and_preserve_errors() {
    gtk::init().expect("GTK display");
    gio::resources_register_include!("strata.gresource").expect("bundled icons");
    crate::ui::prepare_portal_ui();
    let parents = [
        gtk::Box::new(gtk::Orientation::Vertical, 0),
        gtk::Box::new(gtk::Orientation::Vertical, 0),
    ];
    let summaries = parents.each_ref().map(SettingsIntegrationStatus::new);
    for (chooser, folders, reveal, shortcuts, installed) in [
        (false, false, false, None, false),
        (true, false, false, Some(false), true),
        (true, true, true, Some(false), true),
        (true, true, true, Some(true), true),
        (true, true, true, None, true),
        (false, false, false, Some(false), false),
    ] {
        for summary in &summaries {
            summary.message("", false);
            let status = || {
                Ok((
                    portal_setup::PortalStatus {
                        configured: chooser,
                        has_installation: installed,
                    },
                    portal_setup::FileManagerStatus {
                        default: folders,
                        has_service: reveal,
                        has_installation: installed,
                        shortcuts,
                    },
                ))
            };
            summary.show_result(status());
            let complete = summary.complete.upgrade().expect("complete action");
            let restore = summary.restore.upgrade().expect("restore action");
            assert_eq!(
                complete.is_visible(),
                !(chooser && folders && reveal && shortcuts != Some(false))
            );
            assert_eq!(restore.is_visible(), installed);
            assert!(complete.is_sensitive());
            for (indicator, configured) in summary.indicators.iter().zip([
                Some(chooser),
                Some(folders),
                Some(reveal),
                shortcuts,
            ]) {
                let row = indicator.row.upgrade().expect("status row");
                assert_eq!(row.is_visible(), configured.is_some());
                assert!(row.tooltip_text().is_none());
            }
            for enable in [true, false] {
                summary.show_progress(enable);
                assert!(!complete.is_sensitive());
                assert!(!restore.is_sensitive());
                assert!(
                    !summary
                        .message
                        .upgrade()
                        .expect("inline message")
                        .is_visible()
                );
                let action = if enable { &complete } else { &restore };
                assert_eq!(
                    action.label().as_deref(),
                    Some(if enable {
                        "Completing setup…"
                    } else {
                        "Restoring defaults…"
                    })
                );
                summary.show_result(status());
                assert_eq!(complete.label().as_deref(), Some("Complete setup"));
                assert_eq!(restore.label().as_deref(), Some("Restore default"));
                assert!(complete.is_sensitive());
            }
            SETUP_RUNNING.set(true);
            summary.show_result(status());
            assert!(!complete.is_sensitive());
            assert!(!restore.is_sensitive());
            SETUP_RUNNING.set(false);
            summary.message("Could not apply shortcuts", true);
            summary.show_result(status());
            let message = summary.message.upgrade().expect("inline error");
            assert!(message.is_visible());
            assert_eq!(message.text(), "Could not apply shortcuts");
            assert!(complete.is_sensitive());
            summary.show_result(Err("configuration is unreadable".into()));
            assert!(!complete.is_sensitive());
            assert!(!restore.is_sensitive());
            assert!(
                summary.indicators.iter().all(|indicator| !indicator
                    .row
                    .upgrade()
                    .expect("status row")
                    .is_visible())
            );
            summary.show_result(status());
            assert!(complete.is_sensitive());
        }
    }
}

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

#[test]
fn package_builds_show_package_notes_and_refuse_runtime_setup() {
    crate::test_support::gtk_test(
        "ui::portal_preferences::tests::package_builds_show_package_notes_and_refuse_runtime_setup",
        || {
            crate::ui::prepare_portal_ui();
            crate::build_info::disable_runtime_integrations_for_test();
            for (row, note) in [
                (settings_row(), PACKAGE_NOTE),
                (
                    crate::ui::udiskie_preferences::settings_row(),
                    crate::ui::udiskie_preferences::PACKAGE_NOTE,
                ),
            ] {
                assert!(row.is_visible(), "package note rows stay searchable");
                assert!(
                    descendants::<gtk::Button>(row.upcast_ref()).is_empty(),
                    "package builds offer no setup actions"
                );
                assert!(
                    descendants::<gtk::Label>(row.upcast_ref())
                        .iter()
                        .any(|label| label.text() == note)
                );
            }
            let refused = crate::i18n::tr(portal_setup::PACKAGE_PROVIDED);
            for result in [
                portal_setup::install(),
                portal_setup::uninstall(),
                portal_setup::install_file_manager(),
                portal_setup::uninstall_file_manager(),
                portal_setup::dismiss_prompt(),
                portal_setup::udiskie::install(),
                portal_setup::udiskie::uninstall(),
            ] {
                assert_eq!(result, Err(refused.clone()));
            }
            assert_eq!(portal_setup::refresh_stale_portal(), Ok(()));
            assert_eq!(portal_setup::refresh_after_in_place_update(), Ok(()));
            for path in [
                glib::user_config_dir().join("strata/portal-opt-in-v1"),
                glib::user_config_dir().join("xdg-desktop-portal"),
                glib::user_config_dir().join("udiskie"),
                glib::user_data_dir().join("dbus-1"),
                glib::user_data_dir().join("xdg-desktop-portal"),
                glib::user_data_dir().join("strata/portal-install"),
                glib::user_data_dir().join("strata/file-manager-install"),
                glib::user_data_dir().join("strata/udiskie-install"),
            ] {
                assert!(!path.exists(), "refused setup must not write {path:?}");
            }
        },
    );
}
