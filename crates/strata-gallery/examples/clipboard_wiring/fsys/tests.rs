//! A paste's names and guards: Strata's numbering, the own-folder rules, Keep Both, and
//! nothing written outside the root or read in from outside it.

use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use super::{paste, paste_image};
use crate::adapter::{After, Board};

/// A fresh `root` (with folders `a` and `b`) and an `outside` beside it, under the gallery's
/// target directory (never the desktop's /tmp).
fn fixture(name: &str) -> (PathBuf, PathBuf) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp/x3-fsys")
        .join(name);
    // A run that failed may have left a folder read-only.
    let _ = std::fs::read_dir(base.join("root")).map(|rd| {
        for e in rd.flatten() {
            let _ = std::fs::set_permissions(e.path(), std::fs::Permissions::from_mode(0o755));
        }
    });
    let _ = std::fs::remove_dir_all(&base);
    let (root, outside) = (base.join("root"), base.join("outside"));
    for d in [root.join("a/sub"), root.join("b"), outside.clone()] {
        std::fs::create_dir_all(d).expect("fixture folder");
    }
    std::fs::write(root.join("a/todo.txt"), "todo").unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    (root.canonicalize().unwrap(), outside)
}

fn names(dir: &Path) -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    n.sort();
    n
}

#[test]
fn a_copy_into_its_own_folder_is_numbered_and_counts_on() {
    // operations/clipboard: Pasting a copied file into its own folder creates `name (1).ext`, then `name (2).ext`, with no dialog.
    let (root, _) = fixture("own-folder");
    let a = root.join("a");
    let todo = [a.join("todo.txt")];
    assert_eq!(
        paste(&root, &todo, &a, false).unwrap().written,
        ["todo (1).txt"]
    );
    let done = paste(&root, &todo, &a, false).unwrap();
    assert_eq!(done.written, ["todo (2).txt"]);
    assert!(done.conflicts.is_empty(), "no conflict for its own folder");
    // operations/clipboard: Pasting `document (1).txt` into its own folder creates `document (2).txt`; a folder becomes `folder (1)`.
    std::fs::remove_file(a.join("todo (2).txt")).unwrap();
    let one = [a.join("todo (1).txt")];
    assert_eq!(
        paste(&root, &one, &a, false).unwrap().written,
        ["todo (2).txt"]
    );
    let sub = [a.join("sub")];
    assert_eq!(paste(&root, &sub, &a, false).unwrap().written, ["sub (1)"]);
}

#[test]
fn numbering_keeps_only_the_last_extension_and_reads_no_leading_zero() {
    let (root, _) = fixture("numbering");
    let a = root.join("a");
    for f in ["archive.tar.gz", ".hidden", "v (01).txt", "noext"] {
        std::fs::write(a.join(f), "x").unwrap();
    }
    let pasted = |f: &str| paste(&root, &[a.join(f)], &a, false).unwrap().written;
    assert_eq!(pasted("archive.tar.gz"), ["archive.tar (1).gz"]);
    assert_eq!(pasted(".hidden"), [".hidden (1)"]);
    assert_eq!(pasted("v (01).txt"), ["v (01) (1).txt"]);
    assert_eq!(pasted("noext"), ["noext (1)"]);
}

#[test]
fn a_cut_into_its_own_folder_and_a_folder_into_itself_do_nothing() {
    // operations/clipboard: Cut and paste into the same folder does nothing.
    let (root, _) = fixture("no-ops");
    let a = root.join("a");
    let done = paste(&root, &[a.join("todo.txt")], &a, true).unwrap();
    assert!(done.written.is_empty() && done.moved.is_empty(), "{done:?}");
    assert_eq!(names(&a), ["sub", "todo.txt"]);
    // operations/clipboard: Moving a folder into itself or a descendant, including through a symlink alias, does nothing and leaves the source untouched.
    for moving in [false, true] {
        let done = paste(&root, std::slice::from_ref(&a), &a.join("sub"), moving).unwrap();
        assert_eq!(done.skipped, [("a".to_owned(), "into itself")], "{done:?}");
    }
    symlink(a.join("sub"), root.join("alias")).unwrap();
    let done = paste(&root, std::slice::from_ref(&a), &root.join("alias"), true).unwrap();
    assert_eq!(done.skipped.len(), 1, "through a link: {done:?}");
    assert!(a.join("todo.txt").exists());
}

#[test]
fn a_taken_name_elsewhere_is_kept_beside() {
    // operations/clipboard/conflicts: Keep Both creates the first free `name (N).ext` beside the existing item and leaves it untouched.
    let (root, _) = fixture("keep-both");
    let (a, b) = (root.join("a"), root.join("b"));
    std::fs::write(b.join("todo.txt"), "theirs").unwrap();
    let done = paste(&root, &[a.join("todo.txt")], &b, true).unwrap();
    assert_eq!(done.written, ["todo (1).txt"]);
    assert_eq!(
        done.conflicts,
        [("todo.txt".to_owned(), "todo (1).txt".to_owned())]
    );
    assert_eq!(done.moved, [a.join("todo.txt")]);
    assert_eq!(
        std::fs::read_to_string(b.join("todo.txt")).unwrap(),
        "theirs"
    );
    assert_eq!(
        std::fs::read_to_string(b.join("todo (1).txt")).unwrap(),
        "todo"
    );
    assert!(!a.join("todo.txt").exists());
}

#[test]
fn a_source_or_destination_outside_the_root_refuses_the_whole_paste() {
    let (root, outside) = fixture("outside");
    let (a, b) = (root.join("a"), root.join("b"));
    let sources = [a.join("todo.txt"), outside.join("secret.txt")];
    assert!(paste(&root, &sources, &b, false).is_err());
    assert!(names(&b).is_empty(), "nothing written before the refusal");
    assert!(paste(&root, &[a.join("todo.txt")], &outside, false).is_err());
    assert!(paste_image(&root, &outside, b"png").is_err());
    // A link out of the root as the destination resolves outside it.
    symlink(&outside, root.join("out")).unwrap();
    assert!(paste(&root, &[a.join("todo.txt")], &root.join("out"), false).is_err());
    assert_eq!(names(&outside), ["secret.txt"]);
}

#[test]
fn a_link_is_copied_as_the_link() {
    // operations/clipboard: A symlink inside a copied folder is recreated as a symlink pointing at the same target, not followed.
    let (root, outside) = fixture("links");
    let a = root.join("a");
    symlink(outside.join("secret.txt"), a.join("sub/link")).unwrap();
    paste(&root, &[a.join("sub")], &root.join("b"), false).unwrap();
    let copied = root.join("b/sub/link");
    assert!(copied.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read_link(copied).unwrap(),
        outside.join("secret.txt")
    );
}

#[test]
fn a_folder_moves_whole_and_an_image_is_numbered() {
    let (root, _) = fixture("move-folder");
    let (a, b) = (root.join("a"), root.join("b"));
    std::fs::write(a.join("sub/inner.txt"), "in").unwrap();
    let done = paste(&root, &[a.join("sub")], &b, true).unwrap();
    assert_eq!(done.written, ["sub"]);
    assert!(b.join("sub/inner.txt").exists() && !a.join("sub").exists());
    // operations/clipboard: Pasting while the clipboard holds an image and no files writes `image.png`, then `image (1).png`, into a local destination.
    assert_eq!(paste_image(&root, &b, b"one").unwrap(), "image.png");
    assert_eq!(paste_image(&root, &b, b"two").unwrap(), "image (1).png");
    assert_eq!(std::fs::read(b.join("image (1).png")).unwrap(), b"two");
}

fn set_mode(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

#[test]
fn a_cut_that_fails_partway_keeps_what_it_moved_and_leaves_the_rest_cut() {
    let (root, _) = fixture("partway");
    let (a, b) = (root.join("a"), root.join("b"));
    // A folder moves only with write access to it (its `..` changes): this one fails.
    let locked = a.join("locked");
    std::fs::create_dir(&locked).unwrap();
    set_mode(&locked, 0o555);
    let cut = vec![a.join("todo.txt"), locked.clone()];
    let mut board = Board::<u32>::default();
    board.copied(1, cut.clone(), true);
    let done = paste(&root, &cut, &b, true);
    set_mode(&locked, 0o755);
    let done = done.expect("a paste that failed partway still says what it did");
    assert_eq!(
        (&done.written, &done.moved),
        (&vec!["todo.txt".to_owned()], &vec![a.join("todo.txt")])
    );
    assert_eq!(done.failed.as_ref().map(|f| f.0.as_str()), Some("locked"));
    assert!(b.join("todo.txt").exists() && !b.join("locked").exists());
    // What moved leaves the cut; the rest stays cut, and pastes on its own.
    assert_eq!(
        board.moved(&done.moved),
        After::Recut(1, vec![locked.clone()])
    );
    let again = paste(&root, std::slice::from_ref(&locked), &b, true).expect("the rest pastes");
    assert_eq!((again.moved, again.failed), (vec![locked], None));
}

#[test]
fn a_file_whose_source_cant_be_removed_is_not_left_in_both_places() {
    let (root, _) = fixture("unremovable");
    let (c, b) = (root.join("c"), root.join("b"));
    std::fs::create_dir(&c).unwrap();
    std::fs::write(c.join("kept.txt"), "kept").unwrap();
    set_mode(&c, 0o555);
    let done = paste(&root, &[c.join("kept.txt")], &b, true);
    set_mode(&c, 0o755);
    assert!(
        !b.join("kept.txt").exists(),
        "the new name is gone again: {done:?}"
    );
    assert_eq!(std::fs::read_to_string(c.join("kept.txt")).unwrap(), "kept");
}

#[test]
fn the_largest_number_is_read_as_part_of_the_name() {
    // Strata's `parse_copy_suffix`: no number past it, so it is no suffix.
    let (root, _) = fixture("largest");
    let (a, b) = (root.join("a"), root.join("b"));
    let largest = format!("name ({}).ext", u64::MAX);
    std::fs::write(a.join(&largest), "x").unwrap();
    let done = paste(&root, &[a.join(&largest)], &a, false).unwrap();
    assert_eq!(done.written, [format!("name ({}) (1).ext", u64::MAX)]);
    // One below it still counts on, to it.
    let below = format!("name ({}).ext", u64::MAX - 1);
    std::fs::write(b.join(&below), "x").unwrap();
    let done = paste(&root, &[b.join(&below)], &b, false).unwrap();
    assert_eq!(done.written, [largest]);
}
