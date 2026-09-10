// SPDX-License-Identifier: MIT

use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::Path};

use gtk::{gio, glib};

use super::{
    GIO_FALLBACK_BACKENDS, LaunchMode, encode_daemon_pids, gvfs_daemon_pids,
    gvfs_probe_marker_is_fresh_at, gvfs_probe_marker_path_in, launch_mode, open_requests,
    run_preview_helper,
};

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
    ] {
        assert_eq!(launch_mode(&["strata".into(), flag.into()]), mode);
        assert_eq!(
            launch_mode(&["strata".into(), "/tmp".into(), flag.into()]),
            LaunchMode::Application
        );
    }
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

#[test]
fn open_requests_preserve_directory_and_remote_arguments() {
    // Real smb/sftp hosts aren't reachable in tests; trash:// is a
    // network-free non-native backend and is enough to prove classification
    // isn't gated on `is_native()` any more.
    let non_utf8 = OsString::from_vec(b"/tmp/\xff".to_vec());
    let files = [
        gio::File::for_path("/tmp/first"),
        gio::File::for_path("/tmp/second"),
        gio::File::for_path(&non_utf8),
        gio::File::for_uri("trash:///"),
    ];

    let requests = glib::MainContext::new().block_on(open_requests(&files));
    assert!(requests.iter().all(|request| request.selection.is_empty()));
    let locations: Vec<_> = requests.iter().map(|request| &request.directory).collect();

    assert_eq!(locations.len(), files.len());
    assert_eq!(locations[0].native_path(), Some(Path::new("/tmp/first")));
    assert_eq!(locations[1].native_path(), Some(Path::new("/tmp/second")));
    assert_eq!(locations[2].native_path(), Some(Path::new(&non_utf8)));
    assert!(
        locations[3]
            .uri_value()
            .is_some_and(|uri| uri.starts_with("trash:")),
        "{:?}",
        locations[3]
    );
    assert!(
        glib::MainContext::new()
            .block_on(open_requests(&[]))
            .is_empty()
    );
}

#[test]
fn open_requests_reveal_local_files_but_open_directories() {
    use gtk::prelude::*;

    let root = tempfile::tempdir().expect("temporary directory");
    let directory = root
        .path()
        .join(OsString::from_vec(b"parent-\xff".to_vec()));
    std::fs::create_dir(&directory).expect("create parent directory");
    let file = directory.join("open me.txt");
    std::fs::write(&file, "hello").expect("create file");
    let link = directory.join("linked.txt");
    std::os::unix::fs::symlink(&file, &link).expect("create symlink");
    let missing = directory.join("missing.txt");
    let broken_link = directory.join("broken.txt");
    std::os::unix::fs::symlink(&missing, &broken_link).expect("create broken symlink");
    let directory_link = root.path().join("linked-directory");
    std::os::unix::fs::symlink(&directory, &directory_link).expect("create directory symlink");
    let files = [
        gio::File::for_path(&directory),
        gio::File::for_path(&file),
        gio::File::for_uri(gio::File::for_path(&file).uri().as_str()),
        gio::File::for_path(&link),
        gio::File::for_path(&broken_link),
    ];

    let requests = glib::MainContext::new().block_on(open_requests(&files));

    assert_eq!(requests.len(), files.len());
    for (request, selection) in requests.iter().zip([
        vec![],
        vec!["open me.txt"],
        vec!["open me.txt"],
        vec!["linked.txt"],
        vec!["broken.txt"],
    ]) {
        assert_eq!(request.directory.native_path(), Some(directory.as_path()));
        assert_eq!(request.selection, selection);
        assert!(!request.properties);
    }
    for path in [&directory_link, &missing] {
        let requests =
            glib::MainContext::new().block_on(open_requests(&[gio::File::for_path(path)]));
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].directory.native_path(), Some(path.as_path()));
        assert!(requests[0].selection.is_empty());
    }
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

#[test]
fn gvfs_fallback_covers_files_and_volumes() {
    assert_eq!(
        GIO_FALLBACK_BACKENDS,
        [("GIO_USE_VFS", "local"), ("GIO_USE_VOLUME_MONITOR", "unix"),]
    );
}
