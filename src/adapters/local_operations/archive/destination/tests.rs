// SPDX-License-Identifier: MIT

use super::{ExtractNameResolver, ExtractionDestination, validated_archive_path};
use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::Write,
    os::unix::{ffi::OsStringExt, fs::symlink},
    path::{Path, PathBuf},
};

#[test]
fn archive_paths_must_be_nonempty_confined_relative_paths() -> Result<(), Box<dyn Error>> {
    for path in [
        "",
        ".",
        "./",
        "././",
        "../marker",
        "safe/../marker",
        "/tmp/marker",
        "\\tmp\\marker",
        "C:\\tmp\\marker",
        "C:marker",
        "safe/C:/marker",
        "\\\\server\\share\\marker",
        "//server/share/marker",
    ] {
        assert!(validated_archive_path(path).is_err(), "accepted {path:?}");
    }
    assert_eq!(
        validated_archive_path("folder/./nested//item.txt")?,
        Path::new("folder/nested/item.txt")
    );
    Ok(())
}

#[test]
fn pinned_destination_survives_path_replacement() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    let moved = root.path().join("moved");
    let external = root.path().join("external");
    fs::create_dir(&target)?;
    fs::create_dir(&external)?;
    let destination = ExtractionDestination::open(&target)?;
    fs::rename(&target, &moved)?;
    symlink(&external, &target)?;

    let (mut file, created) = destination.create_file(Path::new("nested/file.txt"))?;
    file.write_all(b"contents")?;
    drop(file);
    assert_eq!(fs::read(moved.join(&created))?, b"contents");
    assert!(external.read_dir()?.next().is_none());
    assert!(ExtractionDestination::open(&target).is_err());
    destination.remove_file(&created)?;
    assert!(!moved.join(&created).exists());
    Ok(())
}

#[test]
fn destination_refuses_symlinks_at_every_write_component() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let external = tempfile::tempdir()?;
    fs::write(external.path().join("keep.txt"), b"original")?;
    let destination = ExtractionDestination::open(root.path())?;
    symlink(external.path(), root.path().join("redirect"))?;
    symlink(external.path().join("keep.txt"), root.path().join("leaf"))?;
    symlink(
        external.path().join("missing"),
        root.path().join("dangling"),
    )?;

    for name in ["redirect/new.txt", "leaf", "dangling"] {
        assert!(
            destination.create_file(Path::new(name)).is_err(),
            "accepted {name}"
        );
    }
    assert_eq!(fs::read(external.path().join("keep.txt"))?, b"original");
    assert_eq!(external.path().read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn resolver_keeps_nested_members_under_the_same_renamed_root() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("folder"))?;
    fs::create_dir(root.path().join("folder (2)"))?;
    let destination = ExtractionDestination::open(root.path())?;
    let mut resolver = ExtractNameResolver::new();
    let first = resolver.resolve(&destination, Path::new("folder/one.txt"))?;
    assert_eq!(first, Path::new("folder (3)/one.txt"));
    destination.create_file(&first)?;
    assert_eq!(
        resolver.resolve(&destination, Path::new("folder/nested/two.txt"))?,
        Path::new("folder (3)/nested/two.txt")
    );
    assert_eq!(root.path().join("folder").read_dir()?.count(), 0);
    Ok(())
}

#[test]
fn leaf_conflicts_preserve_native_filename_bytes() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let name = PathBuf::from(OsString::from_vec(b"report-\xff.txt".to_vec()));
    let renamed = PathBuf::from(OsString::from_vec(b"report-\xff (2).txt".to_vec()));
    fs::write(root.path().join(&name), b"original")?;
    let destination = ExtractionDestination::open(root.path())?;
    let (mut file, created) = destination.create_file(&name)?;
    file.write_all(b"new")?;
    drop(file);
    assert_eq!(created, renamed);
    assert_eq!(fs::read(root.path().join(&name))?, b"original");
    assert_eq!(fs::read(root.path().join(&created))?, b"new");
    destination.remove_file(&created)?;
    assert!(!root.path().join(&created).exists());
    Ok(())
}
