// SPDX-License-Identifier: MIT

use std::{
    fs::Permissions,
    os::unix::fs::{PermissionsExt, symlink},
};

use super::*;
use crate::test_support::PrivateContext;

fn as_root() -> bool {
    rustix::process::geteuid().is_root()
}

pub(in crate::adapters) fn refusal(path: &Path) -> glib::Error {
    let format = glib::dgettext(Some("glib20"), "Unable to trash child file %s");
    let path = path.to_str().expect("UTF-8 fixture path");
    let message = format.replace("%1$s", path).replace("%s", path);
    glib::Error::new(gio::IOErrorEnum::PermissionDenied, &message)
}

#[test]
fn trash_child_diagnostics_must_point_inside_the_requested_folder() -> std::io::Result<()> {
    if as_root() {
        return Ok(());
    }
    let fixture = tempfile::tempdir()?;
    let root = fixture.path().join("to-delete");
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested)?;
    std::fs::write(nested.join("file: name.txt"), b"blocker")?;
    std::fs::set_permissions(&nested, Permissions::from_mode(0o500))?;
    for (path, expected) in [
        (nested.join("file: name.txt"), Some("nested/file: name.txt")),
        (root.clone(), None),
        (fixture.path().join("to-delete-other/file.txt"), None),
        (root.join("nested/../../elsewhere"), None),
        (PathBuf::from("nested/file.txt"), None),
    ] {
        assert_eq!(
            reported_trash_child(&root, &refusal(&path)),
            expected.map(PathBuf::from),
            "{path:?}"
        );
    }
    let generic = glib::Error::new(
        gio::IOErrorEnum::PermissionDenied,
        &format!(
            "Unable to trash file {}/file.txt: Permission denied",
            nested.display()
        ),
    );
    assert_eq!(reported_trash_child(&root, &generic), None);
    std::fs::set_permissions(&nested, Permissions::from_mode(0o700))
}

#[test]
fn a_trash_refusal_names_the_folder_unless_the_listed_entry_is_blocked() -> std::io::Result<()> {
    if as_root() {
        return Ok(());
    }
    for (folder, folder_mode, expected) in [
        ("nested", 0o500, Some("nested/mine.txt")),
        ("nested", 0o700, Some("nested")),
        ("", 0o500, Some("mine.txt")),
        ("", 0o700, None),
    ] {
        let fixture = tempfile::tempdir()?;
        let root = fixture.path().join("to-delete");
        let folder = root.join(folder);
        std::fs::create_dir_all(&folder)?;
        std::fs::write(folder.join("mine.txt"), b"mine")?;
        std::fs::set_permissions(&folder, Permissions::from_mode(folder_mode))?;
        let named = reported_trash_child(&root, &refusal(&folder.join("mine.txt")));
        std::fs::set_permissions(&folder, Permissions::from_mode(0o700))?;
        assert_eq!(
            named,
            expected.map(PathBuf::from),
            "{folder:?} {folder_mode:o}"
        );
    }
    Ok(())
}

#[test]
fn gio_path_extraction_handles_reordered_placeholders_and_native_bytes() {
    assert_eq!(
        interpolate_path(
            b"'/fixture/nested/\xff.dat' could not be moved to Trash",
            b"'%1$s' could not be moved to Trash"
        ),
        Some(b"/fixture/nested/\xff.dat".as_slice()),
    );
}

fn blocker_in(folder: &Path) -> DeleteBlocker {
    DeleteBlocker {
        root: folder.join("to-delete"),
        child: PathBuf::from("nested/blocked.txt"),
        one_of_several: false,
    }
}

fn blocked_tree(folder: &Path) -> std::io::Result<DeleteBlocker> {
    let blocker = blocker_in(folder);
    std::fs::create_dir_all(blocker.root.join("nested"))?;
    std::fs::write(blocker.root.join(&blocker.child), b"blocker")?;
    Ok(blocker)
}

fn revealed(blocker: &DeleteBlocker) -> Option<(Location, Location)> {
    PrivateContext::new().block_on(blocker.reveal_target())
}

fn entry_in(folder: PathBuf, name: &str) -> Option<(Location, Location)> {
    Some((
        Location::local(folder.clone()),
        Location::local(folder.join(name)),
    ))
}

#[test]
fn reveal_follows_links_above_the_deleted_folder_but_not_from_it_down() -> std::io::Result<()> {
    let fixture = tempfile::tempdir()?;
    let blocker = blocked_tree(&fixture.path().join("real"))?;
    let alias = fixture.path().join("alias");
    symlink(fixture.path().join("real"), &alias)?;
    assert_eq!(
        revealed(&blocker_in(&alias)),
        entry_in(alias.join("to-delete/nested"), "blocked.txt")
    );

    for swapped in [blocker.root.join("nested"), blocker.root.clone()] {
        let moved = fixture.path().join("outside");
        std::fs::rename(&swapped, &moved)?;
        symlink(&moved, &swapped)?;
        assert!(
            blocker.root.join(&blocker.child).exists(),
            "ordinary path lookup would follow the replacement"
        );
        assert_eq!(revealed(&blocker), None, "{swapped:?}");
        std::fs::remove_file(&swapped)?;
        std::fs::rename(&moved, &swapped)?;
    }
    Ok(())
}

#[test]
fn reveal_falls_back_to_a_folder_that_cannot_be_entered() -> std::io::Result<()> {
    if as_root() {
        return Ok(());
    }
    let fixture = tempfile::tempdir()?;
    let blocker = blocked_tree(fixture.path())?;
    let nested = blocker.root.join("nested");
    std::fs::set_permissions(&nested, Permissions::from_mode(0o444))?;
    let target = revealed(&blocker);
    std::fs::set_permissions(&nested, Permissions::from_mode(0o700))?;
    assert_eq!(target, entry_in(blocker.root, "nested"));
    Ok(())
}

#[test]
fn the_first_blocker_that_still_resolves_is_offered_as_what_it_would_show() -> std::io::Result<()> {
    let fixture = tempfile::tempdir()?;
    let gone = blocker_in(&fixture.path().join("gone"));
    let present = blocked_tree(&fixture.path().join("present"))?;
    let offered =
        |blockers| PrivateContext::new().block_on(DeleteBlocker::first_revealable(blockers));
    assert_eq!(
        offered(vec![gone.clone(), present.clone()]),
        Some(present.clone())
    );
    assert_eq!(offered(vec![gone]), None);
    if as_root() {
        return Ok(());
    }
    let nested = present.root.join("nested");
    std::fs::set_permissions(&nested, Permissions::from_mode(0o444))?;
    let shown = offered(vec![present.clone()]);
    std::fs::set_permissions(&nested, Permissions::from_mode(0o700))?;
    let child = PathBuf::from("nested");
    assert_eq!(shown, Some(DeleteBlocker { child, ..present }));
    Ok(())
}

#[test]
fn a_blocker_on_a_stalled_mount_is_given_up_on() {
    let (release, stalled) = std::sync::mpsc::channel::<()>();
    let started = std::time::Instant::now();
    let found = PrivateContext::new().block_on(verify(Duration::from_millis(20), move || {
        let _ = stalled.recv_timeout(Duration::from_secs(5));
        Some(())
    }));
    assert_eq!(found, None);
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(release);
}
