// SPDX-License-Identifier: MIT

//! `--apply`'s guards: nothing written outside the root, nothing read in from outside it.

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use super::apply;

fn fixture(name: &str) -> (PathBuf, PathBuf) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp/x2-fsys")
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    let (root, outside) = (base.join("root"), base.join("outside"));
    for d in [root.join("a"), root.join("b"), outside.clone()] {
        std::fs::create_dir_all(d).expect("fixture folder");
    }
    std::fs::write(root.join("a/notes.txt"), "mine").unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    (root, outside)
}

fn is_link(p: &Path) -> bool {
    p.symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
}

#[test]
fn a_dangling_link_in_the_destination_is_never_written_through() {
    let (root, outside) = fixture("dangling");
    let gone = outside.join("planted.txt");
    symlink(&gone, root.join("b/notes.txt")).unwrap();
    for moving in [false, true] {
        let got = apply(&root, &[root.join("a/notes.txt")], &root.join("b"), moving);
        assert!(got.is_ok(), "{got:?}");
        assert!(
            !gone.exists(),
            "written outside the root (moving: {moving})"
        );
        assert!(is_link(&root.join("b/notes.txt")), "the link is kept");
        if !moving {
            std::fs::remove_file(root.join("b/notes.txt (1)")).unwrap();
        }
    }
    assert_eq!(
        std::fs::read_to_string(root.join("b/notes.txt (1)")).unwrap(),
        "mine"
    );
}

#[test]
fn links_in_a_copied_folder_stay_links() {
    let (root, outside) = fixture("links-in");
    symlink(outside.join("secret.txt"), root.join("a/to-file")).unwrap();
    symlink(&outside, root.join("a/to-dir")).unwrap();
    let got = apply(&root, &[root.join("a")], &root.join("b"), false);
    assert!(got.is_ok(), "{got:?}");
    for name in ["to-file", "to-dir"] {
        let p = root.join("b/a").join(name);
        assert!(is_link(&p), "{name} copied as what it points at");
    }
    assert_eq!(
        std::fs::read_to_string(root.join("b/a/notes.txt")).unwrap(),
        "mine"
    );
}

/// A dragged link is the link: moved or copied, it stays one, and its target is untouched.
#[test]
fn a_dragged_link_is_moved_or_copied_as_a_link() {
    let (root, outside) = fixture("link-source");
    symlink(outside.join("secret.txt"), root.join("a/link")).unwrap();
    let got = apply(&root, &[root.join("a/link")], &root.join("b"), false);
    assert_eq!(got.as_deref(), Ok(&["link".to_owned()][..]));
    assert!(is_link(&root.join("b/link")));
    let got = apply(&root, &[root.join("a/link")], &root.join("b"), true);
    assert_eq!(got.as_deref(), Ok(&["link (1)".to_owned()][..]));
    assert!(is_link(&root.join("b/link (1)")) && !is_link(&root.join("a/link")));
    assert_eq!(
        std::fs::read_to_string(outside.join("secret.txt")).unwrap(),
        "secret"
    );
}

#[test]
fn a_source_outside_the_root_refuses_the_whole_drop() {
    let (root, outside) = fixture("outside-source");
    let got = apply(
        &root,
        &[root.join("a/notes.txt"), outside.join("secret.txt")],
        &root.join("b"),
        true,
    );
    assert!(got.is_err(), "{got:?}");
    assert!(
        root.join("a/notes.txt").exists(),
        "moved before the refusal"
    );
    assert!(outside.join("secret.txt").exists());
}

#[test]
fn ids_are_unique_across_the_process() {
    let (mut one, mut two) = (super::Ids::default(), super::Ids::default());
    let a = one.of(Path::new("/a"));
    let b = two.of(Path::new("/b"));
    assert_ne!(a, b);
    assert_eq!(one.of(Path::new("/a")), a, "kept for its path");
    assert_eq!(two.path(a), None);
}

#[test]
fn a_move_refused_by_permissions_leaves_both_folders_as_they_were() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _) = fixture("eacces");
    std::fs::create_dir(root.join("a/sub")).unwrap();
    std::fs::write(root.join("a/sub/inner.txt"), "in").unwrap();
    let mode = |p: &Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m));
    mode(&root.join("a"), 0o555).unwrap();
    let probe = std::fs::write(root.join("a/probe"), "");
    let got = [
        apply(&root, &[root.join("a/sub")], &root.join("b"), true),
        apply(&root, &[root.join("a/notes.txt")], &root.join("b"), true),
    ];
    mode(&root.join("a"), 0o755).unwrap();
    if probe.is_ok() {
        // Permissions don't bind (run as root): nothing to check.
        return;
    }
    for g in &got {
        assert!(g.is_err(), "{got:?}");
    }
    assert!(root.join("a/sub/inner.txt").exists() && root.join("a/notes.txt").exists());
    let left: Vec<_> = std::fs::read_dir(root.join("b")).unwrap().collect();
    assert!(left.is_empty(), "written into the destination: {left:?}");
}

#[test]
fn a_move_across_devices_that_cant_remove_the_source_removes_its_copy() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _) = fixture("exdev-eacces");
    let mode = |p: &Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m));
    mode(&root.join("a"), 0o555).unwrap();
    let probe = std::fs::write(root.join("a/probe"), "");
    let got = super::copy_then_remove(&root.join("a/notes.txt"), &root.join("b/notes.txt"));
    mode(&root.join("a"), 0o755).unwrap();
    if probe.is_ok() {
        // Permissions don't bind (run as root): nothing to check.
        return;
    }
    assert!(got.is_err(), "{got:?}");
    assert_eq!(
        std::fs::read_to_string(root.join("a/notes.txt")).unwrap(),
        "mine"
    );
    let left: Vec<_> = std::fs::read_dir(root.join("b")).unwrap().collect();
    assert!(left.is_empty(), "the copy stays: {left:?}");
}
