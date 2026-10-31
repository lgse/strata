// SPDX-License-Identifier: MIT

use super::super::fixtures::names;
use super::{
    ExtractNameResolver, ExtractionDestination, FUSE_SUPER_MAGIC, MemberMetadata, Placed,
    archive_stem, sanitized_archive_path,
};
use crate::adapters::local_operations::tests::fs_quirks::{FsQuirks, Op, Quirks, Support};
use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fs,
    io::Write,
    os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, PermissionsExt, symlink},
    },
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn put_file(
    destination: &ExtractionDestination,
    path: impl AsRef<Path>,
) -> Result<(fs::File, PathBuf), String> {
    let mut resolver = ExtractNameResolver::new(destination);
    let (placed, file) = new_file(&mut resolver, destination, path.as_ref())?;
    Ok((file, resolver.made(placed)))
}

fn new_file(
    resolver: &mut ExtractNameResolver,
    destination: &ExtractionDestination,
    path: &Path,
) -> Result<(Placed, fs::File), String> {
    Ok(resolver.leaf(destination, path, |placed| {
        destination.create_file(placed, None)
    })?)
}

fn put_link(
    destination: &ExtractionDestination,
    path: impl AsRef<Path>,
    target: &OsStr,
    modified: Option<SystemTime>,
) -> Result<PathBuf, String> {
    let mut resolver = ExtractNameResolver::new(destination);
    let (placed, ()) = resolver.leaf(destination, path.as_ref(), |placed| {
        destination.create_symlink(placed, target, modified)
    })?;
    Ok(resolver.made(placed))
}

fn put_hard_link(
    destination: &ExtractionDestination,
    path: impl AsRef<Path>,
    target: &Path,
) -> Result<PathBuf, String> {
    let mut resolver = ExtractNameResolver::new(destination);
    let (placed, ()) = resolver.leaf(destination, path.as_ref(), |placed| {
        destination.create_hard_link(placed, target)
    })?;
    Ok(resolver.made(placed))
}

fn put_folder(
    destination: &ExtractionDestination,
    path: impl AsRef<Path>,
) -> Result<PathBuf, String> {
    Ok(ExtractNameResolver::new(destination).folder(destination, path.as_ref())?)
}

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
        "C:/evil.txt",
        "./c:notes",
        "safe/../c:notes",
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
        ("names/c:notes", "names/c:notes"),
        ("names\\c:notes", "names/c:notes"),
        ("safe/C:/marker", "safe/C:/marker"),
        ("safe/../names/c:notes", "names/c:notes"),
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
    let mut resolver = ExtractNameResolver::new(&staging);
    for file in files {
        let (placed, mut created) = new_file(&mut resolver, &staging, Path::new(file))?;
        created.write_all(file.as_bytes())?;
        resolver.made(placed);
    }
    Ok((name, staging))
}

#[test]
fn publishing_reports_failures_without_discarding_staged_files() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _quirks = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::Rename, "bundle", 0, rustix::io::Errno::ACCESS)],
            ..Quirks::default()
        },
    );
    let destination = ExtractionDestination::open(root.path())?;
    let (staging, _) = stage(&destination, &["first.txt"])?;
    assert!(staging.to_string_lossy().starts_with(".strata-extraction-"));

    let error = destination
        .publish_staging_as_folder(&staging, "bundle.zip")
        .expect_err("a refused publication must fail");

    assert!(error.contains("Could not publish"), "{error}");
    assert_eq!(
        fs::read(root.path().join(&staging).join("first.txt"))?,
        b"first.txt"
    );
    assert_eq!(
        destination.publish_staging_as_folder(&staging, "bundle.zip")?,
        "bundle"
    );
    assert_eq!(
        fs::read(root.path().join("bundle/first.txt"))?,
        b"first.txt"
    );
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
    let (mut file, created) = put_file(&staging, Path::new("nested/file.txt"))?;
    file.write_all(b"contents")?;
    drop(file);
    assert_eq!(fs::read(moved.join(&name).join(&created))?, b"contents");
    assert!(external.read_dir()?.next().is_none());
    staging.remove_file(&created)?;
    assert!(!moved.join(&name).join(&created).exists());
    let (mut second, _) = put_file(&staging, Path::new("second.txt"))?;
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

    let (mut file, created) = put_file(&destination, Path::new("file.txt"))?;
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
    put_file(&destination, Path::new("file.txt"))?;

    // A folder the extraction did not make is a taken name.
    let nested = Path::new("redirect/new.txt");
    let mut resolver = ExtractNameResolver::new(&destination);
    let (placed, _) = new_file(&mut resolver, &destination, nested)?;
    assert_eq!(resolver.made(placed), Path::new("redirect (2)/new.txt"));
    let (placed, ()) = resolver
        .leaf(&destination, nested, |placed| {
            destination.create_symlink(placed, OsStr::new("file.txt"), None)
        })
        .map_err(String::from)?;
    assert_eq!(resolver.made(placed), Path::new("redirect (2)/new (2).txt"));
    let (placed, ()) = resolver
        .leaf(&destination, nested, |placed| {
            destination.create_hard_link(placed, Path::new("file.txt"))
        })
        .map_err(String::from)?;
    assert_eq!(resolver.made(placed), Path::new("redirect (2)/new (3).txt"));
    assert!(
        put_hard_link(
            &destination,
            Path::new("linked"),
            Path::new("redirect/keep.txt")
        )
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
    for name in ["leaf", "dangling"] {
        let (_, created) = put_file(&destination, Path::new(name))?;
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
    put_file(&destination, Path::new("file.txt"))?
        .0
        .write_all(b"data")?;
    let modified = UNIX_EPOCH + Duration::from_secs(1_000_000_000);

    let first = put_link(
        &destination,
        Path::new("lnk"),
        OsStr::new("/etc/passwd"),
        None,
    )?;
    let second = put_link(
        &destination,
        Path::new("lnk"),
        OsStr::new("file.txt"),
        Some(modified),
    )?;
    let hard = put_hard_link(
        &destination,
        Path::new("nested/hard"),
        Path::new("file.txt"),
    )?;

    assert_eq!(
        [first, second, hard],
        ["lnk", "lnk (2)", "nested/hard"].map(PathBuf::from)
    );
    let path = |name: &str| root.path().join(name);
    assert_eq!(fs::read_link(path("lnk"))?, Path::new("/etc/passwd"));
    assert_eq!(fs::read_link(path("lnk (2)"))?, Path::new("file.txt"));
    assert_eq!(
        fs::symlink_metadata(path("lnk (2)"))?.mtime(),
        1_000_000_000
    );
    assert_eq!(
        fs::metadata(path("nested/hard"))?.ino(),
        fs::metadata(path("file.txt"))?.ino()
    );
    assert!(
        put_hard_link(
            &destination,
            Path::new("missing-link"),
            Path::new("missing")
        )
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
    let mut resolver = ExtractNameResolver::new(&destination);
    let (first, _) = new_file(&mut resolver, &destination, Path::new("folder/one.txt"))?;
    assert_eq!(first.path, Path::new("folder (3)/one.txt"));
    resolver.made(first);
    assert_eq!(
        new_file(
            &mut resolver,
            &destination,
            Path::new("folder/nested/two.txt")
        )?
        .0
        .path,
        Path::new("folder (3)/nested/two.txt")
    );
    assert_eq!(root.path().join("folder").read_dir()?.count(), 0);
    Ok(())
}

#[test]
fn leaf_conflicts_preserve_native_filename_bytes() -> Result<(), Box<dyn Error>> {
    let rows: [(&[u8], &[u8]); 2] = [
        (b"report-\xff.txt", b"report-\xff (2).txt"),
        (b"readme.", b"readme. (2)"),
    ];
    for (name, renamed) in rows {
        let root = tempfile::tempdir()?;
        let name = PathBuf::from(OsString::from_vec(name.to_vec()));
        let renamed = PathBuf::from(OsString::from_vec(renamed.to_vec()));
        fs::write(root.path().join(&name), b"original")?;
        let destination = ExtractionDestination::open(root.path())?;
        let (mut file, created) = put_file(&destination, &name)?;
        file.write_all(b"new")?;
        drop(file);
        assert_eq!(created, renamed);
        assert_eq!(fs::read(root.path().join(&name))?, b"original");
        assert_eq!(fs::read(root.path().join(&created))?, b"new");
        destination.remove_file(&created)?;
        assert!(!root.path().join(&created).exists());
    }
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

type Setup = fn(&Path) -> std::io::Result<()>;

#[test]
fn publish_single_root_suffixes_only_on_a_real_collision() -> Result<(), Box<dyn Error>> {
    let cases: [(&str, Setup, &str); 4] = [
        ("none", |_| Ok(()), "readme.txt"),
        (
            "file",
            |path| fs::write(path, b"existing"),
            "readme (2).txt",
        ),
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
        assert_eq!(
            fs::read(root.path().join(expected))?,
            b"readme.txt",
            "{label}"
        );
        assert!(
            root.path().join(&name).read_dir()?.next().is_none(),
            "{label}"
        );
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
fn numbered_names_fit_the_destination() -> Result<(), Box<dyn Error>> {
    let ntfs_3g = || Quirks {
        rename_flags: Support::Rejected(rustix::io::Errno::INVAL),
        ..Quirks::ntfs_3g()
    };
    for (mount, quirks) in [("host", None), ("ntfs-3g", Some(ntfs_3g))] {
        let root = tempfile::tempdir()?;
        let _quirks = quirks.map(|quirks| FsQuirks::install(root.path(), quirks()));
        let folder = "b".repeat(253);
        let file = format!("{}.txt", "r".repeat(251));
        let numbered_file = format!("{} (2).txt", "r".repeat(247));
        fs::write(root.path().join(&folder), b"taken")?;
        fs::write(root.path().join(&file), b"taken")?;
        let destination = ExtractionDestination::open(root.path())?;

        let (name, _) = stage(&destination, &["a.txt"])?;
        let published = destination
            .publish_staging_as_folder(&name, &format!("{folder}.zip"))
            .map_err(|error| format!("{mount}: {error}"))?;
        let published_folder = format!("{} (1)", "b".repeat(251));
        assert_eq!(published, published_folder, "{mount}");

        let (name, staging) = stage(&destination, &[&file])?;
        let published = destination.publish_single_root(&staging, Path::new(&file))?;
        assert_eq!(published, OsString::from(&numbered_file), "{mount}");
        destination.remove_directory_only_staging(&name)?;

        fs::remove_file(root.path().join(&numbered_file))?;
        let (_, created) = put_file(&destination, Path::new(&file))?;
        assert_eq!(created, Path::new(&numbered_file), "{mount}");
        assert_eq!(
            names(root.path())?,
            [&*published_folder, &folder, &numbered_file, &file],
            "{mount}"
        );
    }
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
    assert_eq!(
        fs::read(root.path().join(&full).join("kept.txt"))?,
        b"kept.txt"
    );
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

#[test]
fn publication_without_rename_flags_never_replaces_entries() -> Result<(), Box<dyn Error>> {
    let cases: [(&str, &str, &str, Setup, &str); 4] = [
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
            "docs-1.2/readme.txt",
            "docs-1.2",
            |root| {
                fs::create_dir(root.join("docs-1.2"))?;
                fs::write(root.join("docs-1.2/keep.txt"), b"keep")
            },
            "docs-1.2 (2)",
        ),
    ];
    for (label, member, root_name, create_existing, expected) in cases {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), Quirks::without_rename_flags());
        create_existing(root.path())?;
        let before = fingerprint(&root.path().join(root_name));
        let destination = ExtractionDestination::open(root.path())?;
        let (name, staging) = stage(&destination, &[member])?;
        let staged_inode = fs::symlink_metadata(root.path().join(&name).join(root_name))?.ino();

        let published = destination.publish_single_root(&staging, Path::new(root_name))?;

        assert_eq!(published, OsString::from(expected), "{label}");
        assert_eq!(
            fs::symlink_metadata(root.path().join(expected))?.ino(),
            staged_inode,
            "{label}"
        );
        let inside = Path::new(member).strip_prefix(root_name)?;
        let moved = if inside.as_os_str().is_empty() {
            Path::new(expected).to_path_buf()
        } else {
            Path::new(expected).join(inside)
        };
        assert_eq!(
            fs::read(root.path().join(moved))?,
            member.as_bytes(),
            "{label}"
        );
        if before.is_some() {
            assert_eq!(fingerprint(&root.path().join(root_name)), before, "{label}");
        }
        assert!(
            root.path().join(&name).read_dir()?.next().is_none(),
            "{label}"
        );
    }

    for taken in [false, true] {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), Quirks::without_rename_flags());
        if taken {
            fs::create_dir(root.path().join("bundle"))?;
            fs::write(root.path().join("bundle/keep.txt"), b"keep")?;
        }
        let destination = ExtractionDestination::open(root.path())?;
        let (name, _) = stage(&destination, &["docs/readme.txt"])?;

        let published = destination.publish_staging_as_folder(&name, "bundle.zip")?;

        let expected = if taken { "bundle (1)" } else { "bundle" };
        assert_eq!(published, expected);
        assert_eq!(
            fs::read(root.path().join(expected).join("docs/readme.txt"))?,
            b"docs/readme.txt"
        );
        assert!(!root.path().join(&name).exists());
        if taken {
            assert_eq!(fs::read(root.path().join("bundle/keep.txt"))?, b"keep");
            assert_eq!(root.path().join("bundle").read_dir()?.count(), 1);
        }
    }
    Ok(())
}

#[test]
fn publication_without_rename_flags_moves_unreadable_subtrees_whole() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let _quirks = FsQuirks::install(root.path(), Quirks::without_rename_flags());
    let destination = ExtractionDestination::open(root.path())?;
    let (name, staging) = stage(&destination, &["ro/sub/child.txt"])?;
    fs::create_dir(root.path().join(&name).join("ro/unreadable"))?;
    let read_only = MemberMetadata {
        mode: Some(0o555),
        modified: Some(UNIX_EPOCH + Duration::from_secs(1_000_000_000)),
    };
    staging.apply_directory_metadata(Path::new("ro/sub"), read_only, 0o022)?;
    let unreadable = root.path().join(&name).join("ro/unreadable");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000))?;
    let staged_inode = fs::metadata(root.path().join(&name).join("ro"))?.ino();

    let published = destination.publish_single_root(&staging, Path::new("ro"));

    let published_root = root.path().join("ro");
    let sub_mode = fs::metadata(published_root.join("sub"));
    let staged_sub = root.path().join(&name).join("ro/sub");
    for directory in [
        published_root.join("unreadable"),
        unreadable,
        published_root.join("sub"),
        staged_sub,
    ] {
        let _ = fs::set_permissions(directory, fs::Permissions::from_mode(0o755));
    }
    assert_eq!(published?, OsString::from("ro"));
    assert_eq!(fs::metadata(&published_root)?.ino(), staged_inode);
    assert_eq!(
        fs::read(published_root.join("sub/child.txt"))?,
        b"ro/sub/child.txt"
    );
    assert_eq!(sub_mode?.permissions().mode() & 0o777, 0o555);
    assert!(published_root.join("unreadable").is_dir());
    assert!(root.path().join(&name).read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn publication_without_links_or_noreplace_renames_over_a_reserved_name()
-> Result<(), Box<dyn Error>> {
    for quirks in [Quirks::sshfs, Quirks::vboxsf] {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), quirks());
        let destination = ExtractionDestination::open(root.path())?;
        let (name, staging) = stage(&destination, &["readme.txt", "docs/guide.txt"])?;
        let staged = root.path().join(&name);
        let inodes = [
            fs::metadata(staged.join("readme.txt"))?.ino(),
            fs::metadata(staged.join("docs"))?.ino(),
        ];

        let file = destination.publish_single_root(&staging, Path::new("readme.txt"))?;
        let folder = destination.publish_single_root(&staging, Path::new("docs"))?;

        assert_eq!([file, folder], ["readme.txt", "docs"]);
        assert_eq!(
            [
                fs::metadata(root.path().join("readme.txt"))?.ino(),
                fs::metadata(root.path().join("docs"))?.ino(),
            ],
            inodes
        );
        assert_eq!(fs::read(root.path().join("readme.txt"))?, b"readme.txt");
        assert_eq!(
            fs::read(root.path().join("docs/guide.txt"))?,
            b"docs/guide.txt"
        );
        assert!(staged.read_dir()?.next().is_none());
    }
    Ok(())
}

#[test]
fn publication_keeps_staging_where_no_rung_can_place() -> Result<(), Box<dyn Error>> {
    for (quirks, member) in [
        (Quirks::without_placement(), "readme.txt"),
        (Quirks::without_folder_placement(), "docs"),
    ] {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), quirks);
        let destination = ExtractionDestination::open(root.path())?;
        let (name, staging) = stage(&destination, &["readme.txt", "docs/guide.txt"])?;

        let error = destination
            .publish_single_root(&staging, Path::new(member))
            .expect_err("publication must refuse an unsafe placement");

        assert!(error.contains("guarantee"), "{error}");
        let staged = root.path().join(&name);
        assert_eq!(fs::read(staged.join("readme.txt"))?, b"readme.txt");
        assert_eq!(fs::read(staged.join("docs/guide.txt"))?, b"docs/guide.txt");
        assert_eq!(
            names(root.path())?,
            vec![name.to_string_lossy().into_owned()]
        );
    }
    Ok(())
}

#[test]
fn removing_staging_never_follows_a_symlink_out_of_it() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("keep.txt"), b"keep")?;
    let destination = ExtractionDestination::open(root.path())?;
    let (name, _) = stage(&destination, &["a/b/deep.txt", "top.txt"])?;
    symlink(outside.path(), root.path().join(&name).join("a/link"))?;

    destination.remove_staging(&name)?;

    assert!(root.path().read_dir()?.next().is_none());
    assert_eq!(fs::read(outside.path().join("keep.txt"))?, b"keep");
    assert_eq!(outside.path().read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn directory_only_staging_is_removed_but_files_keep_it() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = ExtractionDestination::open(root.path())?;
    let (directories, staged_directories) = destination.create_staging()?;
    put_folder(&staged_directories, Path::new("a/b/c"))?;
    put_folder(&staged_directories, Path::new("d"))?;
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

#[test]
fn metadata_the_filesystem_cannot_store_is_skipped() -> Result<(), Box<dyn Error>> {
    use rustix::io::Errno;
    let modified = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let metadata = MemberMetadata {
        mode: Some(0o555),
        modified: Some(modified),
    };
    for (errno, skipped) in [
        (Errno::PERM, true),
        (Errno::OPNOTSUPP, true),
        (Errno::INVAL, true),
        (Errno::IO, false),
    ] {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(
            root.path(),
            Quirks {
                fail: vec![
                    (Op::ChmodFd, "*", 0, errno),
                    (Op::SetTimes, "*", 0, errno),
                    (Op::SetTimes, "*", 0, errno),
                    (Op::SetLinkTimes, "*", 0, errno),
                ],
                ..Quirks::default()
            },
        );
        let destination = ExtractionDestination::open(root.path())?;
        put_folder(&destination, Path::new("folder"))?;
        let (file, _) = put_file(&destination, Path::new("file.txt"))?;

        let applied = [
            destination
                .apply_directory_metadata(Path::new("folder"), metadata, 0o022)
                .is_ok(),
            destination.set_file_times(&file, modified).is_ok(),
        ];
        let link = put_link(
            &destination,
            Path::new("lnk"),
            OsStr::new("file.txt"),
            Some(modified),
        );

        assert_eq!(applied, [skipped; 2], "{errno:?}");
        assert_eq!(link?, Path::new("lnk"), "{errno:?}");
        assert_eq!(
            fs::read_link(root.path().join("lnk"))?,
            Path::new("file.txt")
        );
    }
    Ok(())
}

#[test]
fn a_link_time_is_never_set_on_fuse() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _quirks = FsQuirks::install(
        root.path(),
        Quirks {
            fs_type: Some(FUSE_SUPER_MAGIC),
            ..Quirks::default()
        },
    );
    let destination = ExtractionDestination::open(root.path())?;
    let modified = UNIX_EPOCH + Duration::from_secs(1_000_000_000);

    put_link(
        &destination,
        Path::new("lnk"),
        OsStr::new("target"),
        Some(modified),
    )?;

    assert_ne!(
        fs::symlink_metadata(root.path().join("lnk"))?.mtime(),
        1_000_000_000
    );
    Ok(())
}
