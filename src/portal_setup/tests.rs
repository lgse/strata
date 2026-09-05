// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fs, os::unix::fs::PermissionsExt as _, path::PathBuf};

use super::{
    SetupContext, disable_config, enable_config, install_at, secure_executable, trusted_owner,
    uninstall_at,
};

const FILE_CHOOSER: &str = "org.freedesktop.impl.portal.FileChooser";

#[test]
fn chooser_preference_uses_existing_backends_and_round_trips() {
    let original = "[preferred]\ndefault=hyprland;gtk;\norg.example.Other=gtk;\n";
    let enabled = enable_config(original).expect("enable Strata");

    assert!(enabled.contains(&format!("{FILE_CHOOSER}=strata;hyprland;gtk;")));
    assert_eq!(enable_config(&enabled).expect("enable again"), enabled);
    assert_eq!(disable_config(&enabled).expect("disable Strata"), original);
}

#[test]
fn chooser_preference_preserves_explicit_fallbacks() {
    let original = format!("[preferred]\ndefault=gnome;gtk;\n{FILE_CHOOSER}=kde;gtk;\n");
    let enabled = enable_config(&original).expect("enable Strata");

    assert!(enabled.contains(&format!("{FILE_CHOOSER}=strata;kde;gtk;")));
    assert_eq!(disable_config(&enabled).expect("disable Strata"), original);
}

#[test]
fn install_and_uninstall_restore_an_existing_user_configuration() {
    let fixture = fixture();
    let context = context(fixture.path());
    let config = context.portal_directory().join("portals.conf");
    fs::create_dir_all(config.parent().expect("config parent")).expect("config directory");
    let original = "[preferred]\ndefault=gtk;\n";
    fs::write(&config, original).expect("portal config");

    let executable = executable(fixture.path());
    let installed_path = install_at(&context, &executable).expect("install portal");

    assert_eq!(installed_path, config);
    assert!(
        fs::read_to_string(&config)
            .expect("installed config")
            .contains("strata;gtk;")
    );
    assert!(
        context
            .data_home
            .join("xdg-desktop-portal/portals/strata.portal")
            .is_file()
    );
    assert!(
        fs::read_to_string(
            context
                .data_home
                .join("dbus-1/services/org.freedesktop.impl.portal.desktop.strata.service")
        )
        .expect("D-Bus service")
        .contains(&format!(
            "Exec={}/bin/strata --portal",
            fixture.path().display()
        ))
    );

    install_at(&context, &executable).expect("install portal again");

    assert!(!uninstall_at(&context).expect("uninstall portal"));
    assert_eq!(
        fs::read_to_string(config).expect("restored config"),
        original
    );
}

#[test]
fn uninstall_keeps_later_configuration_edits() {
    let fixture = fixture();
    let context = context(fixture.path());
    let config = context.portal_directory().join("portals.conf");
    fs::create_dir_all(config.parent().expect("config parent")).expect("config directory");
    fs::write(&config, "[preferred]\ndefault=gtk;\n").expect("portal config");
    install_at(&context, &executable(fixture.path())).expect("install portal");
    fs::write(
        &config,
        format!(
            "[preferred]\ndefault=gtk;\n{FILE_CHOOSER}=strata;gtk;\norg.example.Other=custom;\n"
        ),
    )
    .expect("edit portal config");

    assert!(uninstall_at(&context).expect("uninstall portal"));
    let remaining = fs::read_to_string(config).expect("preserved config");
    assert!(!remaining.contains("strata"));
    assert!(remaining.contains("org.example.Other=custom;"));
}

#[test]
fn generated_override_is_removed_on_uninstall() {
    let fixture = fixture();
    let context = context(fixture.path());
    let system_config = context.search_roots[1].join("xdg-desktop-portal/hyprland-portals.conf");
    fs::create_dir_all(system_config.parent().expect("system config parent"))
        .expect("system config directory");
    fs::write(&system_config, "[preferred]\ndefault=hyprland;gtk;\n").expect("system config");

    let generated = install_at(&context, &executable(fixture.path())).expect("install portal");

    assert!(generated.ends_with("hyprland-portals.conf"));
    assert!(
        fs::read_to_string(&generated)
            .expect("generated config")
            .contains("strata;")
    );
    assert!(!uninstall_at(&context).expect("uninstall portal"));
    assert!(!generated.exists());
}

#[test]
fn portal_activation_rejects_replaceable_executables() {
    let fixture = fixture();
    let executable = executable(fixture.path());
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o775))
        .expect("group-writable executable");
    assert!(secure_executable(&executable).is_err());

    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).expect("safe executable");
    fs::set_permissions(
        executable.parent().expect("executable parent"),
        fs::Permissions::from_mode(0o777),
    )
    .expect("world-writable executable directory");
    assert!(secure_executable(&executable).is_err());

    fs::set_permissions(
        executable.parent().expect("executable parent"),
        fs::Permissions::from_mode(0o755),
    )
    .expect("safe executable directory");
    fs::set_permissions(fixture.path(), fs::Permissions::from_mode(0o777))
        .expect("world-writable executable ancestor");
    assert!(secure_executable(&executable).is_err());
}

#[test]
fn portal_activation_accepts_only_the_effective_user_or_root_as_owners() {
    assert!(trusted_owner(0, 1_000));
    assert!(trusted_owner(1_000, 1_000));
    assert!(!trusted_owner(1_001, 1_000));
}

#[test]
fn the_one_time_offer_never_changes_the_current_chooser() {
    let fixture = fixture();
    let context = context(fixture.path());
    let config = context.portal_directory().join("portals.conf");
    fs::create_dir_all(config.parent().expect("config parent")).expect("config directory");
    let original = "[preferred]\ndefault=gtk;\n";
    fs::write(&config, original).expect("portal config");

    assert!(super::take_prompt_offer_at(&context).expect("first offer"));
    assert!(!super::take_prompt_offer_at(&context).expect("later launch"));
    assert_eq!(
        fs::read_to_string(config).expect("unchanged config"),
        original
    );
    assert!(!context.data_home.exists());
    assert!(!context.state_directory().exists());
}

#[test]
fn dismissing_the_offer_does_not_prevent_later_explicit_installation() {
    let fixture = fixture();
    let context = context(fixture.path());
    super::dismiss_prompt_at(&context).expect("installer declines offer");
    assert!(!super::take_prompt_offer_at(&context).expect("no duplicate app offer"));
    install_at(&context, &executable(fixture.path())).expect("enable from Settings");
    let status = super::status_at(&context).expect("configured status");
    assert!(status.configured);
    assert!(status.has_installation);
    uninstall_at(&context).expect("restore previous chooser");
    assert!(
        !super::status_at(&context)
            .expect("restored status")
            .configured
    );
    assert!(!super::take_prompt_offer_at(&context).expect("still dismissed"));
}

#[test]
fn an_existing_configured_chooser_is_not_offered_again() {
    let fixture = fixture();
    let context = context(fixture.path());
    install_at(&context, &executable(fixture.path())).expect("existing CLI integration");
    assert!(!super::take_prompt_offer_at(&context).expect("already configured"));
    uninstall_at(&context).expect("uninstall");
    assert!(!super::take_prompt_offer_at(&context).expect("remember prior opt-in"));
}

#[test]
fn status_uses_the_active_configuration_and_requires_activation_files() {
    let fixture = fixture();
    let context = context(fixture.path());
    let config = install_at(&context, &executable(fixture.path())).expect("install");
    fs::write(&config, "[preferred]\ndefault=gtk;\n").expect("change chooser externally");
    let status = super::status_at(&context).expect("external preference");
    assert!(!status.configured);
    assert!(status.has_installation);
    fs::write(&config, "[preferred]\ndefault=strata;gtk;\n").expect("prefer Strata");
    assert!(
        super::status_at(&context)
            .expect("default preference")
            .configured
    );
    fs::remove_file(
        context
            .data_home
            .join("dbus-1/services")
            .join(super::SERVICE_FILE),
    )
    .expect("remove activation");
    assert!(
        !super::status_at(&context)
            .expect("incomplete integration")
            .configured
    );
}

#[test]
fn concurrent_launches_claim_only_one_offer() {
    let fixture = fixture();
    let workers = (0..8)
        .map(|_| {
            let root = fixture.path().to_owned();
            std::thread::spawn(move || {
                super::take_prompt_offer_at(&context(&root)).expect("claim offer")
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| usize::from(worker.join().expect("offer worker")))
            .sum::<usize>(),
        1
    );
}

fn context(root: &std::path::Path) -> SetupContext {
    let data_home = root.join("data");
    let config_home = root.join("config");
    SetupContext {
        search_roots: vec![config_home.clone(), root.join("system")],
        data_home,
        config_home,
        config_names: vec![
            "hyprland-portals.conf".to_owned(),
            "portals.conf".to_owned(),
        ],
    }
}

fn executable(root: &std::path::Path) -> PathBuf {
    let path = root.join("bin/strata");
    fs::create_dir_all(path.parent().expect("executable parent")).expect("executable directory");
    fs::write(&path, b"binary").expect("executable file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("executable permissions");
    path
}

fn fixture() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("strata-portal-")
        .tempdir_in("target")
        .expect("fixture directory")
}
