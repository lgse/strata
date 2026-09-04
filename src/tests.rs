// SPDX-License-Identifier: GPL-3.0-or-later

use std::ffi::OsString;

use super::{
    GIO_FALLBACK_BACKENDS, encode_daemon_pids, gvfs_daemon_pids, gvfs_probe_marker_is_fresh_at,
    gvfs_probe_marker_path_in,
};

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
