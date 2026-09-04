// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{
    OmarchyGeneration, backup_path, config_errors_reported, detect_generation, has_managed_block,
    managed_block, with_managed_block, without_managed_block,
};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn quattro_block_is_appended_after_existing_bindings() {
    let existing =
        "-- personal overrides\no.bind(\"SUPER + SHIFT + M\", \"Fix monitors\", \"x\")\n";

    let updated = with_managed_block(existing, OmarchyGeneration::Quattro);

    assert!(updated.starts_with(existing));
    assert!(updated.contains("hl.unbind(\"SUPER + SHIFT + F\")"));
    assert!(updated.contains("{ launch = \"strata\" }"));
    assert!(has_managed_block(&updated, OmarchyGeneration::Quattro));
}

#[test]
fn legacy_block_uses_hyprland_binding_syntax() {
    let updated = with_managed_block(
        "bindd = SUPER, RETURN, Terminal, exec, foot\n",
        OmarchyGeneration::Legacy,
    );

    assert!(updated.contains("unbind = SUPER SHIFT, F"));
    assert!(updated.contains("bindd = SUPER SHIFT, F, File manager, exec, uwsm-app -- strata"));
    assert!(has_managed_block(&updated, OmarchyGeneration::Legacy));
}

#[test]
fn reapplying_replaces_the_block_instead_of_stacking_copies() {
    let existing = "-- keep me\n";

    let once = with_managed_block(existing, OmarchyGeneration::Quattro);
    let twice = with_managed_block(&once, OmarchyGeneration::Quattro);

    assert_eq!(once, twice);
    assert_eq!(twice.matches("hl.unbind(\"SUPER + SHIFT + F\")").count(), 1);
}

#[test]
fn removing_the_block_restores_surrounding_bindings() {
    let existing = "-- keep me\no.bind(\"SUPER + K\", \"Thing\", \"thing\")\n";

    let applied = with_managed_block(existing, OmarchyGeneration::Quattro);
    let reverted = without_managed_block(&applied, OmarchyGeneration::Quattro);

    assert_eq!(reverted.trim_end(), existing.trim_end());
    assert!(!has_managed_block(&reverted, OmarchyGeneration::Quattro));
}

#[test]
fn an_empty_bindings_file_receives_only_the_block() {
    assert_eq!(
        with_managed_block("\n \n", OmarchyGeneration::Legacy),
        managed_block(OmarchyGeneration::Legacy)
    );
}

#[test]
fn a_users_own_strata_binding_is_not_mistaken_for_the_managed_block() {
    let hand_written =
        "o.bind(\"SUPER + ALT + SHIFT + F\", \"File manager\", { launch = \"strata\" })\n";

    assert!(!has_managed_block(hand_written, OmarchyGeneration::Quattro));
}

#[test]
fn quattro_is_detected_from_the_installed_version() -> io::Result<()> {
    let directory = test_directory()?;
    let share = directory.join("share");
    let hypr = directory.join("hypr");
    fs::create_dir_all(&share)?;
    fs::create_dir_all(&hypr)?;
    fs::write(share.join("version"), "4.0.0.alpha\n")?;
    fs::write(hypr.join("bindings.lua"), "")?;
    fs::write(hypr.join("bindings.conf"), "")?;

    assert_eq!(
        detect_generation(&share, &hypr),
        Some(OmarchyGeneration::Quattro)
    );
    fs::remove_dir_all(directory)
}

#[test]
fn a_three_x_install_keeps_using_the_conf_bindings() -> io::Result<()> {
    let directory = test_directory()?;
    let share = directory.join("share");
    let hypr = directory.join("hypr");
    fs::create_dir_all(&share)?;
    fs::create_dir_all(&hypr)?;
    fs::write(share.join("version"), "3.1.2\n")?;
    fs::write(hypr.join("bindings.conf"), "")?;

    assert_eq!(
        detect_generation(&share, &hypr),
        Some(OmarchyGeneration::Legacy)
    );
    fs::remove_dir_all(directory)
}

#[test]
fn detection_fails_without_the_generations_bindings_file() -> io::Result<()> {
    let directory = test_directory()?;
    let share = directory.join("share");
    let hypr = directory.join("hypr");
    fs::create_dir_all(&share)?;
    fs::create_dir_all(&hypr)?;
    fs::write(share.join("version"), "4.0.0")?;
    fs::write(hypr.join("bindings.conf"), "")?;

    assert_eq!(detect_generation(&share, &hypr), None);
    fs::remove_dir_all(directory)
}

#[test]
fn detection_fails_outside_omarchy() -> io::Result<()> {
    let directory = test_directory()?;
    let share = directory.join("share");
    let hypr = directory.join("hypr");
    fs::create_dir_all(&share)?;
    fs::create_dir_all(&hypr)?;

    assert_eq!(detect_generation(&share, &hypr), None);
    fs::remove_dir_all(directory)
}

#[test]
fn only_reported_configuration_errors_count_as_failure() {
    assert!(!config_errors_reported(""));
    assert!(!config_errors_reported("\n  \n"));
    assert!(!config_errors_reported("no errors"));
    assert!(config_errors_reported(
        "Config error in line 42: unknown keyword"
    ));
}

#[test]
fn the_backup_sits_beside_the_bindings_file() {
    let backup = backup_path(&PathBuf::from("/home/user/.config/hypr/bindings.lua"));

    assert_eq!(backup.parent(), Some(Path::new("/home/user/.config/hypr")));
    assert!(
        backup
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("bindings.lua.strata.bak."))
    );
}

fn test_directory() -> io::Result<PathBuf> {
    loop {
        let path = std::env::temp_dir().join(format!(
            "strata-omarchy-setup-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}
