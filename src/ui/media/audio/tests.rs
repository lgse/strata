// SPDX-License-Identifier: MIT
use super::*;
use std::os::unix::{
    fs::{PermissionsExt, symlink},
    net::UnixListener,
};

#[test]
fn audio_endpoints_require_private_owned_storage_and_real_unix_sockets() {
    let runtime = crate::media_helper::private_tempdir().expect("private runtime");
    let pulse = runtime.path().join("pulse");
    std::fs::create_dir(&pulse).expect("private service directory");
    let path = pulse.join("native");
    assert!(audio_socket(runtime.path()).is_err());
    let _listener = UnixListener::bind(&path).expect("fixture socket, never host audio");
    assert_eq!(audio_socket(runtime.path()).expect("owned socket"), path);
    std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o755))
        .expect("unsafe runtime");
    assert!(audio_socket(runtime.path()).is_err());
    std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private runtime");
    std::fs::rename(&path, pulse.join("real")).expect("move endpoint");
    symlink("real", &path).expect("substituted endpoint");
    assert!(audio_socket(runtime.path()).is_err());
}
