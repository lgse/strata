// SPDX-License-Identifier: MIT

use super::{ExtractNameResolver, ExtractionDestination, sanitized_archive_path};
use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::Write,
    os::unix::{ffi::OsStringExt, fs::symlink},
    path::{Path, PathBuf},
};

#[test]
fn archive_paths_are_sanitized_to_confined_relative_paths() -> Result<(), Box<dyn Error>> {
    for name in [
        "",
        ".",
        "./",
        "././",
        "..",
        "safe/..",
        "safe/../..",
        "/tmp/marker",
        "\\tmp\\marker",
        "C:\\tmp\\marker",
        "C:marker",
        "safe/C:/marker",
        "\\\\server\\share\\marker",
        "//server/share/marker",
    ] {
        assert!(sanitized_archive_path(name).is_err(), "accepted {name:?}");
    }
    for (name, expected) in [
        ("../marker", "marker"),
        ("safe/../marker", "marker"),
        ("safe/../../marker", "marker"),
        ("safe\\..\\..\\marker", "marker"),
        ("folder/./nested//item.txt", "folder/nested/item.txt"),
    ] {
        assert_eq!(
            sanitized_archive_path(name)?,
            Path::new(expected),
            "{name:?}"
        );
    }
    Ok(())
}

#[test]
fn bundling_reports_failures_without_discarding_extracted_files() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = ExtractionDestination::open(root.path())?;
    fs::write(root.path().join("first.txt"), b"first")?;
    let roots = [PathBuf::from("first.txt"), PathBuf::from("missing.txt")];
    let error = destination
        .bundle_roots(&format!("{}.zip", "a".repeat(256)), &roots)
        .expect_err("overlong bundle name must fail");
    assert!(
        error.contains("Could not create extraction folder"),
        "{error}"
    );
    assert_eq!(fs::read(root.path().join("first.txt"))?, b"first");
    let error = destination
        .bundle_roots("bundle.zip", &roots)
        .expect_err("missing extracted root must fail");
    assert!(error.contains("Could not bundle `missing.txt`"), "{error}");
    assert_eq!(fs::read(root.path().join("bundle/first.txt"))?, b"first");
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
    destination.remove_file(&created)?;
    assert!(!moved.join(&created).exists());
    let (mut second, _) = destination.create_file(Path::new("second.txt"))?;
    second.write_all(b"second")?;
    drop(second);
    assert_eq!(
        destination.bundle_roots(
            "bundle.zip",
            &[PathBuf::from("nested"), PathBuf::from("second.txt")]
        )?,
        Some("bundle".to_owned())
    );
    assert!(moved.join("bundle/nested").is_dir());
    assert_eq!(fs::read(moved.join("bundle/second.txt"))?, b"second");
    assert!(external.read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn destination_resolves_a_symlinked_directory_and_pins_it() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let real = root.path().join("real");
    let alias = root.path().join("alias");
    let other = root.path().join("other");
    fs::create_dir(&real)?;
    fs::create_dir(&other)?;
    symlink(&real, &alias)?;

    let destination = ExtractionDestination::open(&alias)?;
    fs::remove_file(&alias)?;
    symlink(&other, &alias)?;

    let (mut file, created) = destination.create_file(Path::new("file.txt"))?;
    file.write_all(b"contents")?;
    drop(file);
    assert_eq!(fs::read(real.join(&created))?, b"contents");
    assert!(other.read_dir()?.next().is_none());
    assert!(ExtractionDestination::open(Path::new("relative")).is_err());
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

#[test]
fn available_bytes_reports_unprivileged_free_space() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = ExtractionDestination::open(root.path())?;
    assert!(
        matches!(destination.available_bytes()?, Some(bytes) if bytes > 0),
        "tempdir should report some free space"
    );
    Ok(())
}
