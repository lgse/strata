// SPDX-License-Identifier: MIT
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn only_fixed_protected_library_aliases_enter_the_sandbox() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = fixture.path();
    let owner = rustix::process::geteuid().as_raw();
    let libraries = root.join("usr/lib/vendor");
    let alternatives = root.join("etc/alternatives");
    fs::create_dir_all(&libraries).expect("system library fixture");
    fs::create_dir_all(&alternatives).expect("aliases");
    let source = libraries.join("libblas.so.3");
    let mut elf = [0; 20];
    elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    let machine: u16 = if cfg!(target_arch = "aarch64") {
        183
    } else {
        62
    };
    elf[18..20].copy_from_slice(&machine.to_le_bytes());
    fs::write(&source, elf).expect("ELF");
    let abi = if cfg!(target_arch = "aarch64") {
        "aarch64-linux-gnu"
    } else {
        "x86_64-linux-gnu"
    };
    let name = format!("libblas.so.3-{abi}");
    let alias = alternatives.join(&name);
    symlink(&source, &alias).expect("known alias");
    symlink(&source, alternatives.join("editor")).expect("unrelated alias");
    assert_eq!(
        aliases_at(root, owner),
        [(source.clone(), Path::new("/etc/alternatives").join(&name))]
    );
    assert!(aliases_at(root, owner + 1).is_empty());
    fs::set_permissions(&libraries, fs::Permissions::from_mode(0o777)).expect("unsafe ancestor");
    assert!(aliases_at(root, owner).is_empty());
    fs::set_permissions(&libraries, fs::Permissions::from_mode(0o755)).expect("repair ancestor");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o666)).expect("unsafe file");
    assert!(aliases_at(root, owner).is_empty());
    fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).expect("repair file");
    fs::write(&source, b"not a library").expect("malformed library");
    assert!(aliases_at(root, owner).is_empty());
    let outside = root.join("private-document");
    fs::write(&outside, elf).expect("outside library prefix");
    fs::remove_file(&alias).expect("remove alias");
    symlink(outside, &alias).expect("redirect outside prefix");
    assert!(aliases_at(root, owner).is_empty());
}
