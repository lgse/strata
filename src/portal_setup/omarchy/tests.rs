// SPDX-License-Identifier: MIT

use super::*;

fn installed(major: u8) -> String {
    let installer = include_str!("../../../install.sh");
    let prefix = if major == 4 { "--" } else { "#" };
    let start = format!("{prefix} strata-installer: file-manager start");
    let end = format!("{prefix} strata-installer: file-manager end");
    let from = installer.find(&start).expect("valid test fixture");
    let to = installer[from..].find(&end).expect("valid test fixture") + from + end.len();
    let block = installer[from..to]
        .replace("$BIN_PATH", "/home/test/.local/bin/strata")
        .replace("\\$", "$");
    format!("unrelated-before\n{block}\nunrelated-after\n")
}

#[test]
fn restore_replaces_both_installer_shortcuts_and_keeps_unrelated_configuration() {
    for major in [3, 4] {
        assert!(shortcuts_configured_in(&installed(major), major));
        let result = restored_bindings(&installed(major), major)
            .expect("valid test fixture")
            .expect("valid test fixture");
        assert!(!shortcuts_configured_in(&result, major));
        assert!(!shortcuts_configured_in("", major));
        assert!(!shortcuts_configured_in(
            &installed(major).replace("File manager (cwd)", "Custom shortcut"),
            major
        ));
        assert!(result.starts_with("unrelated-before\n"));
        assert!(result.ends_with("\nunrelated-after\n"));
        assert!(!result.contains("/home/test/.local/bin/strata"));
        assert_eq!(result.matches("nautilus --new-window").count(), 2);
        assert!(result.contains("$(omarchy-cmd-terminal-cwd)"));
        assert_eq!(
            restored_bindings(&result, major).expect("valid test fixture"),
            None
        );
        assert!(
            restored_bindings(
                &installed(major).replace("File manager (cwd)", "Custom shortcut"),
                major
            )
            .is_err()
        );
        assert!(
            restored_bindings(&format!("{}{}", installed(major), installed(major)), major).is_err()
        );
    }
}

#[test]
fn complete_setup_installs_repairs_and_reinstalls_shortcuts() {
    for major in [3, 4] {
        let executable = Path::new("/home/test/strata/target/debug/strata");
        for original in [
            String::new(),
            "unrelated-setting\n".into(),
            installed(major),
            restored_bindings(&installed(major), major)
                .expect("restore")
                .expect("installed block"),
        ] {
            let configured =
                installed_bindings(&original, major, executable).expect("complete setup");
            assert!(shortcuts_configured_in(&configured, major));
            assert_eq!(
                configured
                    .matches(executable.to_str().expect("UTF-8 executable"))
                    .count(),
                2
            );
            assert_eq!(
                installed_bindings(&configured, major, executable).expect("retry"),
                configured
            );
            if original.starts_with("unrelated-setting") {
                assert!(configured.starts_with("unrelated-setting\n"));
            }
            let restored = restored_bindings(&configured, major)
                .expect("restore")
                .expect("configured block");
            assert!(!shortcuts_configured_in(&restored, major));
        }
        let customized = installed(major).replace("File manager (cwd)", "Custom shortcut");
        assert!(installed_bindings(&customized, major, executable).is_err());
        for unsafe_path in [
            "/home/test/my strata",
            "/home/test/$strata",
            "/home/test/strata;touch-marker",
            "/home/test/strata\ncommand",
        ] {
            assert!(installed_bindings("", major, Path::new(unsafe_path)).is_err());
        }
    }
}

#[test]
fn failed_reload_rolls_back_and_keeps_a_backup() {
    let fixture = tempfile::tempdir().expect("valid test fixture");
    let path = fixture.path().join("bindings.lua");
    let original = installed(4);
    let updated = restored_bindings(&original, 4)
        .expect("valid test fixture")
        .expect("valid test fixture");
    fs::write(&path, &original).expect("valid test fixture");
    assert!(replace_bindings(&path, &original, &updated, || Err("reload failure".into())).is_err());
    assert_eq!(
        fs::read_to_string(&path).expect("valid test fixture"),
        original
    );
    let backups: Vec<_> = fs::read_dir(fixture.path())
        .expect("valid test fixture")
        .map(|entry| entry.expect("valid test fixture").path())
        .filter(|path| {
            path.file_name()
                .expect("valid test fixture")
                .to_string_lossy()
                .starts_with("strata-keybind-backup-")
        })
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read_to_string(&backups[0]).expect("valid test fixture"),
        original
    );
    replace_bindings(&path, &original, &updated, || Ok(())).expect("valid test fixture");
    assert_eq!(
        fs::read_to_string(path).expect("valid test fixture"),
        updated
    );
}

#[test]
fn only_supported_omarchy_versions_are_selected() {
    for (version, expected) in [
        ("3.1.0", Some(3)),
        ("4.0.0", Some(4)),
        ("Omarchy v4.0.0\n", Some(4)),
        ("5.0", None),
        ("", None),
        ("13.0", None),
    ] {
        assert_eq!(major_version(version), expected);
    }
}
