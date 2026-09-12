// SPDX-License-Identifier: MIT
use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};

fn executable(path: &Path) {
    let mut bytes = [0; 64];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    let arch: u16 = if cfg!(target_arch = "aarch64") { 183 } else { 62 };
    bytes[18..20].copy_from_slice(&arch.to_le_bytes());
    fs::write(path, bytes).expect("ELF fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable");
}

#[test]
fn discovery_rejects_absent_nonexecutable_symlink_and_wrong_architecture() {
    let dir = tempfile::tempdir().expect("private directory");
    let path = dir.path().join(NAME);
    assert!(open_helper(&path).err().expect("absent").contains("missing"));
    executable(&path);
    assert!(open_helper(&path).is_ok());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("nonexecutable");
    assert!(open_helper(&path).is_err());
    executable(&path);
    let link = dir.path().join("link");
    symlink(&path, &link).expect("symlink");
    assert!(open_helper(&link).is_err());
    fs::write(&path, b"not an ELF executable").expect("corrupt");
    assert!(open_helper(&path).is_err());
}

#[test]
fn pinned_inode_survives_replacement_and_unlink() {
    let dir = tempfile::tempdir().expect("private directory");
    let path = dir.path().join(NAME);
    executable(&path);
    let file = open_helper(&path).expect("pin helper");
    let replacement = dir.path().join("new");
    fs::write(&replacement, b"unrelated new helper").expect("new executable");
    fs::rename(&replacement, &path).expect("atomic replacement");
    fs::remove_file(&path).expect("remove installation");
    let snapshot = dir.path().join("snapshot");
    fs::copy(format!("/proc/self/fd/{}", file.as_raw_fd()), &snapshot).expect("preserve old inode");
    assert!(fs::read(snapshot).expect("old bytes").starts_with(b"\x7fELF"));
}

#[test]
fn startup_diagnostics_do_not_echo_untrusted_paths_or_recommend_from_arbitrary_stderr() {
    assert!(classify_stderr(b"secret /home/private install evil-command gstreamer").is_none());
    let libraries = classify_stderr(b"/private/helper: error while loading shared libraries: libgstapp-1.0.so.0").expect("loader diagnosis");
    assert!(libraries.contains("runtime libraries"));
    assert!(!libraries.contains("/private"));
    assert!(classify_stderr(b"STRATA_MEDIA:version").expect("mismatch").contains("matching Strata bundle"));
    assert!(classify_stderr(b"STRATA_MEDIA:audio-plugin").expect("plugin").contains("plugins"));
    assert!(classify_stderr(b"STRATA_MEDIA:audio-server").expect("server").contains("audio service"));
    assert!(classify_stderr(b"bwrap: namespace failed /private/input").expect("sandbox").contains("No unsandboxed fallback"));
}
