// SPDX-License-Identifier: MIT

use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::Path};

use gtk::gio;

use super::{
    CommandLineAction, LaunchMode, classify_command_line, classify_udiskie_hook,
    encode_daemon_pids, gvfs_daemon_pids, gvfs_probe_marker_is_fresh_at, gvfs_probe_marker_path_in,
    launch_mode, run_preview_helper, should_select_cairo, version_line,
};
use crate::ui::preferences::InterfaceRenderer;

#[test]
fn application_renderer_respects_saved_choice_and_explicit_environment() {
    assert!(should_select_cairo(None, InterfaceRenderer::Cairo));
    assert!(!should_select_cairo(None, InterfaceRenderer::System));
    for supplied in ["cairo", "gl", ""] {
        assert!(!should_select_cairo(
            Some(supplied.as_ref()),
            InterfaceRenderer::Cairo,
        ));
    }
}

#[test]
fn launch_mode_treats_non_utf8_arguments_as_an_ordinary_launch() {
    let program = OsString::from("strata");
    let non_utf8 = OsString::from_vec(b"/tmp/\xff".to_vec());

    assert_eq!(
        launch_mode(&[program.clone(), non_utf8]),
        LaunchMode::Application
    );
    assert_eq!(
        launch_mode(&[program.clone(), OsString::from("--portal")]),
        LaunchMode::Portal
    );
    assert_eq!(launch_mode(&[program]), LaunchMode::Application);
}

#[test]
fn launch_mode_recognizes_only_the_first_argument_as_a_mode() {
    for (flag, mode) in [
        ("--preview-helper", LaunchMode::PreviewHelper),
        ("--gvfs-probe", LaunchMode::GvfsProbe),
        ("--portal", LaunchMode::Portal),
        ("--install-portal", LaunchMode::InstallPortal),
        ("--dismiss-portal-prompt", LaunchMode::DismissPortalPrompt),
        ("--uninstall-portal", LaunchMode::UninstallPortal),
        ("--version", LaunchMode::Version),
        ("--udiskie-hook", LaunchMode::UdiskieHook),
        ("--install-udiskie-unlock", LaunchMode::InstallUdiskie),
        ("--uninstall-udiskie-unlock", LaunchMode::UninstallUdiskie),
        ("--unlock-volume", LaunchMode::Application),
    ] {
        assert_eq!(launch_mode(&["strata".into(), flag.into()]), mode);
        assert_eq!(
            launch_mode(&["strata".into(), "/tmp".into(), flag.into()]),
            LaunchMode::Application
        );
    }
}

#[test]
fn classify_udiskie_hook_encrypted_device_added() {
    let uuid = "6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7";
    let cases: &[(&[&str], Option<&str>)] = &[
        (
            &["device_added", "crypto", "/dev/sdb1", uuid],
            Some("/dev/sdb1"),
        ),
        (&["device_added", "crypto", "", uuid], Some(uuid)),
        (&["device_added", "filesystem", "/dev/sdb1", uuid], None),
    ];
    for (arguments, expected) in cases {
        let arguments: Vec<OsString> = arguments.iter().copied().map(OsString::from).collect();
        assert_eq!(
            classify_udiskie_hook(&arguments),
            *expected,
            "hook arguments {arguments:?} should classify as {expected:?}"
        );
    }
}

#[test]
fn classify_udiskie_hook_non_utf8_fields() {
    let non_utf8 = OsString::from_vec(b"\xff".to_vec());
    assert_eq!(
        classify_udiskie_hook(&[
            "device_added".into(),
            "crypto".into(),
            non_utf8,
            "6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7".into(),
        ]),
        None,
        "a non-UTF-8 device_file should reject rather than panic"
    );
}

#[test]
fn classify_command_line_unlock_volume() {
    match classify_command_line(Some("/dev/sdb1"), &[], false, false) {
        CommandLineAction::Unlock(target) => {
            assert_eq!(target.unix_device.as_deref(), Some(Path::new("/dev/sdb1")));
            assert_eq!(target.uuid, None);
        }
        other => panic!("unix-device should unlock, got {other:?}"),
    }
    match classify_command_line(Some("6e5d75a7e4e24c7d9c1c8e5a5e5d75a7"), &[], false, false) {
        CommandLineAction::Unlock(target) => {
            assert_eq!(target.unix_device, None);
            assert_eq!(
                target.uuid.as_deref(),
                Some("6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7")
            );
        }
        other => panic!("compact UUID should unlock, got {other:?}"),
    }
    match classify_command_line(Some("not-a-valid-uuid"), &[], false, false) {
        CommandLineAction::Usage(_) => {}
        other => panic!("invalid operand should be usage, got {other:?}"),
    }
    let file = gio::File::for_path("/tmp/Documents");
    match classify_command_line(Some("/dev/sdb1"), std::slice::from_ref(&file), false, false) {
        CommandLineAction::Usage(_) => {}
        other => panic!("unlock with files should be usage, got {other:?}"),
    }
}

#[test]
fn version_line_is_the_package_name_and_installed_version() {
    let line = version_line();
    assert!(
        line.starts_with("strata "),
        "the --version line should start with the package name"
    );
    assert!(
        line.contains(&crate::build_info::installed_version().to_string()),
        "the --version line should include the installed version"
    );
    assert_eq!(
        line.lines().count(),
        1,
        "the --version line should be a single line"
    );
}

#[test]
fn preview_helper_rejects_non_utf8_instead_of_changing_paths() {
    let arguments = [
        "thumbnail-image".into(),
        OsString::from_vec(b"/tmp/\xff".to_vec()),
        "/tmp/result.png".into(),
        "128".into(),
        "software".into(),
    ];
    assert_eq!(
        run_preview_helper(&arguments),
        Err("Invalid UTF-8 in preview helper arguments".to_owned())
    );
}

fn fake_proc(label: &str, processes: &[(&str, &str)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "strata-gvfs-proc-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock should be set")
            .as_nanos()
    ));
    for (pid, comm) in processes {
        let dir = root.join(pid);
        std::fs::create_dir_all(&dir).expect("the fake pid dir should exist");
        std::fs::write(dir.join("comm"), comm).expect("the fake comm should exist");
    }
    root
}

#[test]
fn daemon_identity_lists_gvfs_processes_sorted() {
    let root = fake_proc(
        "identity",
        &[
            ("9", "gvfsd-fuse\n"),
            ("2200", "gvfsd\n"),
            ("31", "bash\n"),
            ("400", "gvfsd-trash\n"),
            ("self", "test\n"),
        ],
    );
    assert_eq!(gvfs_daemon_pids(&root), vec![9, 400, 2200]);
    assert_eq!(encode_daemon_pids(&[9, 2200]), "9,2200");
}

#[test]
fn daemon_restart_changes_the_identity() {
    let before = fake_proc("restart-before", &[("100", "gvfsd\n")]);
    let after = fake_proc("restart-after", &[("8400", "gvfsd\n")]);
    assert_ne!(
        encode_daemon_pids(&gvfs_daemon_pids(&before)),
        encode_daemon_pids(&gvfs_daemon_pids(&after))
    );
}

#[test]
fn missing_proc_root_is_an_empty_identity() {
    let missing = std::env::temp_dir().join("strata-gvfs-proc-definitely-missing");
    assert_eq!(gvfs_daemon_pids(&missing), Vec::<u32>::new());
}

#[test]
fn marker_path_needs_a_runtime_dir() {
    assert_eq!(gvfs_probe_marker_path_in(None), None);
    assert_eq!(gvfs_probe_marker_path_in(Some(OsString::from(""))), None);
    assert_eq!(
        gvfs_probe_marker_path_in(Some(OsString::from("/run/user/1000"))),
        Some(std::path::PathBuf::from(
            "/run/user/1000/strata-gvfs-probe-ok"
        ))
    );
}

#[test]
fn only_a_readable_matching_marker_is_fresh() {
    let proc_root = fake_proc("marker", &[("31", "bash\n")]);
    let marker = proc_root.join("marker");
    assert!(!gvfs_probe_marker_is_fresh_at(&marker, &proc_root));

    std::fs::write(&marker, "").expect("the empty identity marker should be written");
    assert!(gvfs_probe_marker_is_fresh_at(&marker, &proc_root));

    std::fs::write(&marker, [0xff]).expect("the unreadable marker should be written");
    assert!(!gvfs_probe_marker_is_fresh_at(&marker, &proc_root));
}
