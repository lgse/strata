// SPDX-License-Identifier: MIT
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

fn executable(path: &Path) {
    let mut bytes = [0; 64];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    let arch: u16 = if cfg!(target_arch = "aarch64") {
        183
    } else {
        62
    };
    bytes[18..20].copy_from_slice(&arch.to_le_bytes());
    fs::write(path, bytes).expect("ELF fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable");
}

#[test]
fn discovery_rejects_absent_nonexecutable_symlink_and_wrong_architecture() {
    let dir = tempfile::tempdir().expect("private directory");
    let path = dir.path().join(NAME);
    assert!(open_helper(&path).expect_err("absent").contains("missing"));
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
    assert!(
        fs::read(snapshot)
            .expect("old bytes")
            .starts_with(b"\x7fELF")
    );
}

#[test]
fn unsafe_temporary_and_installation_ancestors_are_rejected_before_copying() {
    let root = tempfile::tempdir().expect("private fixture");
    let unsafe_parent = root.path().join("shared");
    fs::create_dir(&unsafe_parent).expect("shared directory");
    fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o777)).expect("unsafe parent");
    let private = unsafe_parent.join("private");
    fs::create_dir(&private).expect("private child");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).expect("private permissions");
    executable(&private.join(NAME));
    assert!(trusted_directory(&private).is_err());
    assert!(open_helper(&private.join(NAME)).is_err());
    fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o700)).expect("repair root");
    assert!(trusted_directory(&private).is_ok());
    assert!(open_helper(&private.join(NAME)).is_ok());
    assert!(trusted_directory(Path::new(".")).is_err());
}

#[test]
fn offline_recovery_installs_only_the_exact_embedded_bytes_and_rejects_corruption() {
    use std::io::Write;
    let root = tempfile::tempdir().expect("private fixture");
    let source = root.path().join(NAME);
    executable(&source);
    let expected = hash_file(&File::open(&source).expect("helper")).expect("digest");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&fs::read(&source).expect("helper bytes"))
        .expect("compress");
    let compressed = encoder.finish().expect("gzip");
    let cache = root.path().join("cache");
    let recovered = recover_at(&cache, &compressed, &expected).expect("offline recovery");
    assert_eq!(
        fs::read(&recovered).expect("recovered"),
        fs::read(&source).expect("matching helper")
    );
    assert_eq!(
        recover_at(&cache, &compressed, &expected).expect("reuse matching helper"),
        recovered
    );
    fs::write(&recovered, b"damaged").expect("simulate damaged installation");
    assert!(recover_at(&cache, &compressed, &expected).is_err());
    assert!(
        recover_at(
            &root.path().join("truncated"),
            &compressed[..compressed.len() - 4],
            &expected
        )
        .is_err()
    );
    assert!(
        recover_at(
            &root.path().join("wrong-hash"),
            &compressed,
            &"0".repeat(64)
        )
        .is_err()
    );
    assert!(recover_at(&root.path().join("traversal"), &compressed, "../helper").is_err());
}

#[test]
fn startup_diagnostics_do_not_echo_untrusted_paths_or_recommend_from_arbitrary_stderr() {
    assert!(classify_stderr(b"secret /home/private install evil-command gstreamer").is_none());
    let libraries = classify_stderr(
        b"/private/helper: error while loading shared libraries: libgstapp-1.0.so.0",
    )
    .expect("loader diagnosis");
    assert!(libraries.contains("runtime libraries"));
    assert!(!libraries.contains("/private"));
    assert!(
        classify_stderr(b"STRATA_MEDIA:version")
            .expect("mismatch")
            .contains("matching Strata bundle")
    );
    assert!(
        classify_stderr(b"STRATA_MEDIA:audio-plugin")
            .expect("plugin")
            .contains("plugins")
    );
    assert!(
        classify_stderr(b"STRATA_MEDIA:audio-server")
            .expect("server")
            .contains("audio service")
    );
    assert!(
        classify_stderr(b"bwrap: namespace failed /private/input")
            .expect("sandbox")
            .contains("No unsandboxed fallback")
    );
}
