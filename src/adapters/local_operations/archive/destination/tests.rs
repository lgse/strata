// SPDX-License-Identifier: MIT

use super::{
    ExtractNameResolver, ExtractionDestination, MemberMetadata, MetadataCalls, archive_stem,
    sanitized_archive_path,
};
use std::os::fd::BorrowedFd;
use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fs,
    io::Write,
    os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, symlink},
    },
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
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

fn stage(
    destination: &ExtractionDestination,
    files: &[&str],
) -> Result<(OsString, ExtractionDestination), Box<dyn Error>> {
    let (name, staging) = destination.create_staging()?;
    for file in files {
        let (mut created, _) = staging.create_file(Path::new(file), None)?;
        created.write_all(file.as_bytes())?;
    }
    Ok((name, staging))
}

#[test]
fn publishing_reports_failures_without_discarding_staged_files() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = ExtractionDestination::open(root.path())?;
    let (staging, _) = stage(&destination, &["first.txt"])?;
    assert!(staging.to_string_lossy().starts_with(".strata-extraction-"));

    let error = destination
        .publish_staging_as_folder(&staging, &format!("{}.zip", "a".repeat(256)))
        .expect_err("overlong folder name must fail");

    assert!(error.contains("Could not publish"), "{error}");
    assert_eq!(fs::read(root.path().join(&staging).join("first.txt"))?, b"first.txt");
    assert_eq!(
        destination.publish_staging_as_folder(&staging, "bundle.zip")?,
        "bundle"
    );
    assert_eq!(fs::read(root.path().join("bundle/first.txt"))?, b"first.txt");
    assert!(!root.path().join(&staging).exists());
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

    let (name, staging) = destination.create_staging()?;
    let (mut file, created) = staging.create_file(Path::new("nested/file.txt"), None)?;
    file.write_all(b"contents")?;
    drop(file);
    assert_eq!(fs::read(moved.join(&name).join(&created))?, b"contents");
    assert!(external.read_dir()?.next().is_none());
    staging.remove_file(&created)?;
    assert!(!moved.join(&name).join(&created).exists());
    let (mut second, _) = staging.create_file(Path::new("second.txt"), None)?;
    second.write_all(b"second")?;
    drop(second);
    assert_eq!(
        destination.publish_staging_as_folder(&name, "bundle.zip")?,
        "bundle"
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

    let (mut file, created) = destination.create_file(Path::new("file.txt"), None)?;
    file.write_all(b"contents")?;
    drop(file);
    assert_eq!(fs::read(real.join(&created))?, b"contents");
    assert!(other.read_dir()?.next().is_none());
    assert!(ExtractionDestination::open(Path::new("relative")).is_err());
    Ok(())
}

#[test]
fn destination_never_writes_through_symlinks() -> Result<(), Box<dyn Error>> {
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
    destination.create_file(Path::new("file.txt"), None)?;

    let nested = Path::new("redirect/new.txt");
    assert!(destination.create_file(nested, None).is_err());
    assert!(
        destination
            .create_symlink(nested, OsStr::new("file.txt"), None)
            .is_err()
    );
    assert!(
        destination
            .create_hard_link(nested, Path::new("file.txt"))
            .is_err()
    );
    assert!(
        destination
            .create_hard_link(Path::new("linked"), Path::new("redirect/keep.txt"))
            .is_err()
    );
    let metadata = MemberMetadata {
        mode: Some(0o700),
        modified: None,
    };
    assert!(
        destination
            .apply_directory_metadata(Path::new("redirect"), metadata, 0o022)
            .is_err()
    );
    // An existing leaf symlink is skipped like a file, never written through.
    for name in ["leaf", "dangling"] {
        let (_, created) = destination.create_file(Path::new(name), None)?;
        assert_eq!(created, PathBuf::from(format!("{name} (2)")));
    }
    assert_eq!(fs::read(external.path().join("keep.txt"))?, b"original");
    assert_eq!(external.path().read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn links_are_created_as_stored_and_renamed_on_conflict() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = ExtractionDestination::open(root.path())?;
    destination.create_file(Path::new("file.txt"), None)?.0.write_all(b"data")?;
    let modified = UNIX_EPOCH + Duration::from_secs(1_000_000_000);

    let first = destination.create_symlink(Path::new("lnk"), OsStr::new("/etc/passwd"), None)?;
    let second =
        destination.create_symlink(Path::new("lnk"), OsStr::new("file.txt"), Some(modified))?;
    let hard = destination.create_hard_link(Path::new("nested/hard"), Path::new("file.txt"))?;

    assert_eq!(
        [first, second, hard],
        ["lnk", "lnk (2)", "nested/hard"].map(PathBuf::from)
    );
    let path = |name: &str| root.path().join(name);
    assert_eq!(fs::read_link(path("lnk"))?, Path::new("/etc/passwd"));
    assert_eq!(fs::read_link(path("lnk (2)"))?, Path::new("file.txt"));
    assert_eq!(fs::symlink_metadata(path("lnk (2)"))?.mtime(), 1_000_000_000);
    assert_eq!(
        fs::metadata(path("nested/hard"))?.ino(),
        fs::metadata(path("file.txt"))?.ino()
    );
    assert!(
        destination
            .create_hard_link(Path::new("missing-link"), Path::new("missing"))
            .is_err()
    );
    assert!(fs::symlink_metadata(path("missing-link")).is_err());
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
    destination.create_file(&first, None)?;
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
    let (mut file, created) = destination.create_file(&name, None)?;
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

fn fingerprint(path: &Path) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    Some(if metadata.is_symlink() {
        format!("link to {}", fs::read_link(path).ok()?.display())
    } else if metadata.is_dir() {
        format!("directory of {}", fs::read_dir(path).ok()?.count())
    } else {
        format!("file {:?}", fs::read(path).ok()?)
    })
}

#[test]
fn publish_single_root_suffixes_only_on_a_real_collision() -> Result<(), Box<dyn Error>> {
    let cases: [(&str, fn(&Path) -> std::io::Result<()>, &str); 4] = [
        ("none", |_| Ok(()), "readme.txt"),
        ("file", |path| fs::write(path, b"existing"), "readme (2).txt"),
        ("directory", |path| fs::create_dir(path), "readme (2).txt"),
        (
            "dangling symlink",
            |path| symlink("missing-target", path),
            "readme (2).txt",
        ),
    ];
    for (label, create_existing, expected) in cases {
        let root = tempfile::tempdir()?;
        let occupied = root.path().join("readme.txt");
        create_existing(&occupied)?;
        let before = fingerprint(&occupied);
        let destination = ExtractionDestination::open(root.path())?;
        let (name, staging) = stage(&destination, &["readme.txt"])?;

        let published = destination.publish_single_root(&staging, Path::new("readme.txt"))?;

        assert_eq!(published, OsString::from(expected), "{label}");
        assert_eq!(fs::read(root.path().join(expected))?, b"readme.txt", "{label}");
        assert!(root.path().join(&name).read_dir()?.next().is_none(), "{label}");
        if before.is_some() {
            assert_eq!(fingerprint(&occupied), before, "{label}");
        }
        assert!(!root.path().join("missing-target").exists(), "{label}");
    }
    Ok(())
}

#[test]
fn publish_staging_as_folder_skips_taken_names() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("bundle"), b"file")?;
    symlink("elsewhere", root.path().join("bundle (1)"))?;
    let destination = ExtractionDestination::open(root.path())?;
    let (name, _) = stage(&destination, &["a.txt"])?;

    assert_eq!(
        destination.publish_staging_as_folder(&name, "bundle.zip")?,
        "bundle (2)"
    );

    assert_eq!(fs::read(root.path().join("bundle (2)/a.txt"))?, b"a.txt");
    assert_eq!(fs::read(root.path().join("bundle"))?, b"file");
    assert_eq!(
        fs::read_link(root.path().join("bundle (1)"))?,
        Path::new("elsewhere")
    );
    Ok(())
}

#[test]
fn remove_empty_staging_reports_non_empty() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = ExtractionDestination::open(root.path())?;
    let (empty, _) = stage(&destination, &[])?;
    let (full, _) = stage(&destination, &["kept.txt"])?;

    assert!(destination.remove_empty_staging(&empty)?);
    assert!(!destination.remove_empty_staging(&full)?);

    assert!(!root.path().join(&empty).exists());
    assert_eq!(fs::read(root.path().join(&full).join("kept.txt"))?, b"kept.txt");
    Ok(())
}

#[test]
fn archive_stem_strips_known_extensions() {
    for (name, expected) in [
        ("a.tar.gz", "a"),
        ("a.TGZ", "a"),
        ("a.tar", "a"),
        ("a.zip", "a"),
        ("a.7z", "a"),
        ("a.rar", "a"),
        ("archive", "archive"),
        (".zip", ".zip"),
        ("..tar.gz", "..tar.gz"),
    ] {
        assert_eq!(archive_stem(name), expected, "{name}");
    }
}

fn no_replace_unsupported(
    _: BorrowedFd<'_>,
    _: &OsStr,
    _: BorrowedFd<'_>,
    _: &OsStr,
) -> rustix::io::Result<()> {
    Err(rustix::io::Errno::INVAL)
}

#[test]
fn publication_without_rename_noreplace_never_replaces_entries() -> Result<(), Box<dyn Error>> {
    let cases: [(&str, &str, &str, fn(&Path) -> std::io::Result<()>, &str); 4] = [
        ("file", "readme.txt", "readme.txt", |_| Ok(()), "readme.txt"),
        (
            "file collision",
            "readme.txt",
            "readme.txt",
            |root| fs::write(root.join("readme.txt"), b"existing"),
            "readme (2).txt",
        ),
        ("directory", "docs/readme.txt", "docs", |_| Ok(()), "docs"),
        (
            "directory collision",
            "docs/readme.txt",
            "docs",
            |root| {
                fs::create_dir(root.join("docs"))?;
                fs::write(root.join("docs/readme.txt"), b"existing")
            },
            "docs (2)",
        ),
    ];
    for (label, member, root_name, create_existing, expected) in cases {
        let root = tempfile::tempdir()?;
        create_existing(root.path())?;
        let before = fingerprint(&root.path().join(root_name));
        let destination = ExtractionDestination::open(root.path())?;
        let (name, staging) = stage(&destination, &[member])?;

        let published = destination.publish_single_root_with(
            &staging,
            Path::new(root_name),
            no_replace_unsupported,
        )?;

        assert_eq!(published, OsString::from(expected), "{label}");
        let inside = Path::new(member).strip_prefix(root_name)?;
        let moved = if inside.as_os_str().is_empty() {
            Path::new(expected).to_path_buf()
        } else {
            Path::new(expected).join(inside)
        };
        assert_eq!(fs::read(root.path().join(moved))?, member.as_bytes(), "{label}");
        if before.is_some() {
            assert_eq!(fingerprint(&root.path().join(root_name)), before, "{label}");
        }
        assert!(root.path().join(&name).read_dir()?.next().is_none(), "{label}");
    }

    for (label, taken, expected) in [("folder", false, "bundle"), ("folder collision", true, "bundle (1)")] {
        let root = tempfile::tempdir()?;
        if taken {
            fs::create_dir(root.path().join("bundle"))?;
            fs::write(root.path().join("bundle/keep.txt"), b"keep")?;
        }
        let destination = ExtractionDestination::open(root.path())?;
        let (name, _) = stage(&destination, &["a.txt", "b.txt"])?;

        let published =
            destination.publish_staging_as_folder_with(&name, "bundle.zip", no_replace_unsupported)?;

        assert_eq!(published, expected, "{label}");
        assert_eq!(fs::read(root.path().join(expected).join("a.txt"))?, b"a.txt", "{label}");
        assert!(!root.path().join(&name).exists(), "{label}");
        if taken {
            assert_eq!(fs::read(root.path().join("bundle/keep.txt"))?, b"keep", "{label}");
            assert_eq!(root.path().join("bundle").read_dir()?.count(), 1, "{label}");
        }
    }
    Ok(())
}

#[test]
fn directory_only_staging_is_removed_but_files_keep_it() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = ExtractionDestination::open(root.path())?;
    let (directories, staged_directories) = destination.create_staging()?;
    staged_directories.create_directories(Path::new("a/b/c"))?;
    staged_directories.create_directories(Path::new("d"))?;
    let (with_file, _) = stage(&destination, &["a/b/deep.txt"])?;

    assert!(destination.remove_directory_only_staging(&directories)?);
    assert!(!destination.remove_directory_only_staging(&with_file)?);

    assert!(!root.path().join(&directories).exists());
    assert_eq!(
        fs::read(root.path().join(&with_file).join("a/b/deep.txt"))?,
        b"a/b/deep.txt"
    );
    Ok(())
}

fn refusing<const ERRNO: i32>() -> MetadataCalls {
    MetadataCalls {
        chmod: |_, _| Err(rustix::io::Errno::from_raw_os_error(ERRNO)),
        set_times: |_, _| Err(rustix::io::Errno::from_raw_os_error(ERRNO)),
        set_link_times: |_, _, _| Err(rustix::io::Errno::from_raw_os_error(ERRNO)),
    }
}

#[test]
fn metadata_the_filesystem_cannot_store_is_skipped() -> Result<(), Box<dyn Error>> {
    use rustix::io::Errno;
    let modified = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let metadata = MemberMetadata {
        mode: Some(0o555),
        modified: Some(modified),
    };
    for (errno, calls, skipped) in [
        (Errno::PERM, refusing::<{ Errno::PERM.raw_os_error() }>(), true),
        (Errno::OPNOTSUPP, refusing::<{ Errno::OPNOTSUPP.raw_os_error() }>(), true),
        (Errno::INVAL, refusing::<{ Errno::INVAL.raw_os_error() }>(), true),
        (Errno::IO, refusing::<{ Errno::IO.raw_os_error() }>(), false),
    ] {
        let root = tempfile::tempdir()?;
        let destination = ExtractionDestination::open(root.path())?.with_metadata_calls(calls);
        destination.create_directories(Path::new("folder"))?;
        let (file, _) = destination.create_file(Path::new("file.txt"), None)?;

        let applied = [
            destination
                .apply_directory_metadata(Path::new("folder"), metadata, 0o022)
                .is_ok(),
            destination.set_file_times(&file, modified).is_ok(),
            destination
                .create_symlink(Path::new("lnk"), OsStr::new("file.txt"), Some(modified))
                .is_ok(),
        ];

        assert_eq!(applied, [skipped; 3], "{errno:?}");
        // A link whose time was refused for another reason is not left behind.
        assert_eq!(
            fs::symlink_metadata(root.path().join("lnk")).is_ok(),
            skipped,
            "{errno:?}"
        );
    }
    Ok(())
}
