// SPDX-License-Identifier: MIT

use std::os::unix::fs::{MetadataExt, PermissionsExt};

use rustix::io::Errno;

use super::super::copy_engine::{Destination, Options, copy_local};
use super::fs_quirks::{Call, NameOpens, Op, OverEmptyDirectory, Racer, Support, hold};
use super::*;
use crate::adapters::volume::{MountCapabilities, learn_mount_capability};
use crate::model::hidden_names::foreign_leftover;

struct Mounts {
    _root: tempfile::TempDir,
    sources: PathBuf,
    destination: PathBuf,
    source_scope: FsQuirks,
    destination_scope: FsQuirks,
}

fn mounts(destination: Quirks) -> Result<Mounts, Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let target = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&target)?;
    Ok(Mounts {
        source_scope: FsQuirks::install(&sources, Quirks::default()),
        destination_scope: FsQuirks::install(&target, destination),
        _root: root,
        sources,
        destination: target,
    })
}

fn copy(source: &Path, target: &Path) -> Result<(), glib::Error> {
    glib::MainContext::default().block_on(copy_new_recursively(
        gio::File::for_path(source),
        gio::File::for_path(target),
        gio::Cancellable::new(),
    ))
}

fn mode(path: &Path) -> io::Result<u32> {
    Ok(fs::symlink_metadata(path)?.mode() & 0o7777)
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

fn unlock(root: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Ok(());
    }
    set_mode(root, 0o755)?;
    for entry in fs::read_dir(root)? {
        unlock(&entry?.path())?;
    }
    Ok(())
}

fn set_mtime(path: &Path, seconds: i64) -> io::Result<()> {
    let time = rustix::fs::Timespec {
        tv_sec: seconds,
        tv_nsec: 0,
    };
    rustix::fs::utimensat(
        rustix::fs::CWD,
        path,
        &rustix::fs::Timestamps {
            last_access: time,
            last_modification: time,
        },
        rustix::fs::AtFlags::empty(),
    )?;
    Ok(())
}

fn calls_naming(scope: &FsQuirks, op: Op, name: &str) -> usize {
    scope
        .calls()
        .into_iter()
        .filter(|call| call.op == op && call.names.iter().any(|called| called == name))
        .count()
}

#[test]
fn copying_a_folder_keeps_folder_permissions_and_times() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("private"))?;
    fs::write(source.join("private/file.txt"), b"contents")?;
    let folders = [
        ("private", 0o700, 1_000_000_000),
        ("", 0o750, 1_100_000_000),
    ];
    for (relative, mode, seconds) in folders {
        let folder = source.join(relative);
        rustix::fs::setxattr(
            &folder,
            "user.strata-test",
            relative.as_bytes(),
            rustix::fs::XattrFlags::empty(),
        )?;
        set_mode(&folder, mode)?;
        rustix::fs::utimensat(
            rustix::fs::CWD,
            &folder,
            &rustix::fs::Timestamps {
                last_access: rustix::fs::Timespec {
                    tv_sec: seconds - 1_000,
                    tv_nsec: 0,
                },
                last_modification: rustix::fs::Timespec {
                    tv_sec: seconds,
                    tv_nsec: 0,
                },
            },
            rustix::fs::AtFlags::empty(),
        )?;
    }

    copy(&source, &target)?;

    for (relative, expected, seconds) in folders {
        let folder = target.join(relative);
        let metadata = fs::symlink_metadata(&folder)?;
        assert_eq!(metadata.mode() & 0o7777, expected, "{relative:?}");
        assert_eq!(metadata.mtime(), seconds, "{relative:?}");
        assert_eq!(metadata.atime(), seconds - 1_000, "{relative:?}");
        let mut value = [0u8; 32];
        let length = rustix::fs::getxattr(&folder, "user.strata-test", &mut value[..])?;
        assert_eq!(&value[..length], relative.as_bytes(), "{relative:?}");
    }
    assert_eq!(fs::read(target.join("private/file.txt"))?, b"contents");
    Ok(())
}

const DESCRIPTOR_LIMIT: &str = "STRATA_TEST_DESCRIPTOR_LIMIT";

#[test]
fn copying_many_read_only_folders_needs_no_descriptor_per_folder() -> Result<(), Box<dyn Error>> {
    let inner = "adapters::local_operations::tests::folder_copy::read_only_folders_under_a_descriptor_limit";
    let output = std::process::Command::new(std::env::current_exe()?)
        .args([inner, "--exact", "--ignored", "--test-threads=1"])
        .env(DESCRIPTOR_LIMIT, "256")
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
#[ignore = "run by copying_many_read_only_folders_needs_no_descriptor_per_folder"]
fn read_only_folders_under_a_descriptor_limit() -> Result<(), Box<dyn Error>> {
    let Some(limit) = std::env::var(DESCRIPTOR_LIMIT)
        .ok()
        .and_then(|limit| limit.parse().ok())
    else {
        return Ok(());
    };
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    let groups = (0..40).map(|group| source.join(group.to_string()));
    for group in groups.clone() {
        for folder in 0..50 {
            fs::create_dir_all(group.join(folder.to_string()))?;
        }
    }
    for group in groups.clone() {
        for folder in 0..50 {
            set_mode(&group.join(folder.to_string()), 0o555)?;
        }
        set_mode(&group, 0o555)?;
    }
    rustix::process::setrlimit(
        rustix::process::Resource::Nofile,
        rustix::process::Rlimit {
            current: Some(limit),
            maximum: Some(limit),
        },
    )?;

    let copied = copy(&source, &target);

    let modes = (0..40)
        .flat_map(|group| (0..50).map(move |folder| format!("{group}/{folder}")))
        .chain((0..40).map(|group| group.to_string()))
        .map(|folder| mode(&target.join(folder)))
        .collect::<io::Result<HashSet<_>>>();
    unlock(&source)?;
    if target.exists() {
        unlock(&target)?;
    }
    copied?;
    assert_eq!(modes?, HashSet::from([0o555]));
    Ok(())
}

#[test]
fn copying_into_a_setgid_folder_keeps_it_shared() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let (sources, shared) = (root.path().join("sources"), root.path().join("shared"));
    fs::create_dir_all(sources.join("project/sub"))?;
    fs::write(sources.join("project/file.txt"), b"contents")?;
    set_mode(&sources.join("project/file.txt"), 0o644)?;
    set_mode(&sources.join("project/sub"), 0o750)?;
    set_mode(&sources.join("project"), 0o755)?;
    fs::create_dir(&shared)?;
    set_mode(&shared, 0o2775)?;
    // The source reports a group of its own, which the shared folder's replaces.
    let _sources = FsQuirks::install(
        &sources,
        Quirks {
            foreign_group: true,
            ..Quirks::default()
        },
    );
    let destination = FsQuirks::install(&shared, Quirks::default());

    copy(&sources.join("project"), &shared.join("project"))?;

    let group = fs::metadata(&shared)?.gid();
    for (relative, expected) in [
        ("project", 0o2755),
        ("project/sub", 0o2750),
        ("project/file.txt", 0o644),
    ] {
        let metadata = fs::symlink_metadata(shared.join(relative))?;
        assert_eq!(metadata.mode() & 0o7777, expected, "{relative}");
        assert_eq!(metadata.gid(), group, "{relative}");
    }
    assert_eq!(destination.count(Op::ChownFd), 0);
    Ok(())
}

#[test]
fn a_lost_create_reply_inside_a_stage_is_adopted() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // `File.txt` and `file.txt` may be one name, so they are claimed first.
    for (op, name) in [
        (Op::Mkdir, "dir"),
        (Op::CreateNode, "file.txt"),
        (Op::Create, "other.txt"),
        (Op::Symlink, "link"),
    ] {
        let mounts = mounts(Quirks {
            lost_reply: vec![(op, name, Errno::EXIST)],
            ..Quirks::nfs3()
        })?;
        let source = mounts.sources.join("source");
        fs::create_dir_all(source.join("dir"))?;
        fs::write(source.join("dir/inner.txt"), b"inner")?;
        for file in ["File.txt", "file.txt", "other.txt"] {
            fs::write(source.join(file), file)?;
        }
        std::os::unix::fs::symlink("file.txt", source.join("link"))?;
        let target = mounts.destination.join("target");

        copy(&source, &target)?;

        assert_eq!(
            names(&target)?,
            ["File.txt", "dir", "file.txt", "link", "other.txt"],
            "{op:?}"
        );
        assert_eq!(fs::read(target.join("dir/inner.txt"))?, b"inner", "{op:?}");
        for file in ["File.txt", "file.txt", "other.txt"] {
            assert_eq!(fs::read_to_string(target.join(file))?, file, "{op:?}");
        }
        assert_eq!(fs::read_link(target.join("link"))?, Path::new("file.txt"));
        assert_eq!(
            calls_naming(&mounts.destination_scope, op, name),
            1,
            "{op:?}"
        );
    }
    Ok(())
}

#[test]
fn only_names_that_may_collide_are_claimed() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for (files, claimed) in [
        (
            &["Makefile", "makefile", "notes.txt", "readme."][..],
            &["Makefile", "makefile"][..],
        ),
        (
            &["caf\u{e9}", "cafe\u{301}", "data.bin"],
            &["cafe\u{301}", "caf\u{e9}"],
        ),
        // A FAT short name may stand for any long name.
        (
            &["PROGRA~1", "Program Files"],
            &["PROGRA~1", "Program Files"],
        ),
        (&["notes~2.txt", "other.txt"], &["notes~2.txt", "other.txt"]),
        // A `~N` outside the short-name shape stands for nothing.
        (
            &["Report v2~1 (draft).pdf", "foo~1bar.tar.gz", "data.bin"],
            &[],
        ),
    ] {
        let mounts = mounts(Quirks::nfs3())?;
        let source = mounts.sources.join("source");
        fs::create_dir(&source)?;
        for file in files {
            fs::write(source.join(file), file)?;
        }
        let target = mounts.destination.join("target");

        copy(&source, &target)?;

        for file in files {
            assert_eq!(fs::read_to_string(target.join(file))?, *file);
        }
        let mut made = mounts
            .destination_scope
            .calls()
            .into_iter()
            .filter(|call| call.op == Op::CreateNode)
            .flat_map(|call| call.names)
            .collect::<Vec<_>>();
        made.sort();
        assert_eq!(made, claimed, "{files:?}");
    }
    Ok(())
}

#[test]
fn files_are_claimed_by_creating_them_where_nodes_are_refused_or_on_smb()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let refused = || Quirks {
        mount_type: Some(("fuse.example", "rw")),
        node_create: Support::Rejected(Errno::NOSYS),
        ..Quirks::default()
    };
    let smb = || Quirks {
        mount_type: Some(("cifs", "rw")),
        ..Quirks::default()
    };
    for (row, quirks, nodes) in [("refused", refused as Preset, 1), ("SMB", smb, 0)] {
        let mounts = mounts(quirks())?;
        let source = mounts.sources.join("source");
        fs::create_dir(&source)?;
        // A name like a FAT short name may stand for any other, so every name is claimed.
        let files = ["a.txt", "b.txt", "PROGRA~1.TXT"];
        for file in files {
            fs::write(source.join(file), file)?;
        }
        let target = mounts.destination.join("target");

        copy(&source, &target)?;

        for file in files {
            assert_eq!(fs::read_to_string(target.join(file))?, file, "{row}");
        }
        let scope = &mounts.destination_scope;
        assert_eq!(scope.count(Op::CreateNode), nodes, "{row}");
        assert_eq!(scope.count(Op::CreateReadonly), 3, "{row}");
    }
    Ok(())
}

#[test]
fn a_claimed_name_swapped_for_a_pipe_fails_without_waiting() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let (sources, destination) = (root.path().join("sources"), root.path().join("destination"));
    fs::create_dir_all(sources.join("source"))?;
    fs::create_dir(&destination)?;
    for file in ["File.bin", "file.bin"] {
        fs::write(sources.join("source").join(file), b"contents")?;
    }
    let pipe = Arc::new(std::sync::Mutex::new(None));
    let racer: Racer = {
        let (destination, pipe) = (destination.clone(), pipe.clone());
        Box::new(move || {
            let stage = fs::read_dir(&destination)
                .and_then(|mut entries| entries.next().expect("the stage"))
                .expect("list the destination")
                .path();
            let claimed = stage.join("file.bin");
            fs::remove_file(&claimed).expect("remove the claim");
            rustix::fs::mknodat(
                rustix::fs::CWD,
                &claimed,
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_raw_mode(0o600),
                0,
            )
            .expect("make a pipe");
            *pipe.lock().expect("pipe") = Some(claimed);
        })
    };
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before: vec![(Op::OpenWrite, "file.bin", 0, racer)],
            ..Quirks::nfs3()
        },
    );
    // Without a reader a blocking open waits for ever, so a reader comes
    // after a while to let the test end.
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let rescue = thread::spawn(move || {
        if finished.recv_timeout(Duration::from_secs(5)).is_ok() {
            return false;
        }
        if let Some(pipe) = pipe.lock().expect("pipe").clone() {
            let _reader = rustix::fs::open(
                &pipe,
                rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK,
                rustix::fs::Mode::empty(),
            );
        }
        true
    });

    let result = copy(&sources.join("source"), &destination.join("target"));
    let _ = done.send(());

    assert!(
        !rescue.join().expect("rescue"),
        "the copy waited on the pipe"
    );
    assert!(result.is_err(), "{result:?}");
    assert_eq!(names(&destination)?, Vec::<String>::new());
    Ok(())
}

#[test]
fn a_claimed_file_evicted_before_its_job_is_still_written() -> Result<(), Box<dyn Error>> {
    use std::sync::atomic::Ordering;
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let (source, destination) = (root.path().join("source"), root.path().join("destination"));
    fs::create_dir(&source)?;
    fs::create_dir(&destination)?;
    for file in ["PROGRA~1", "Program Files"] {
        fs::write(source.join(file), file)?;
    }
    let evicted = Arc::new(AtomicBool::new(false));
    let racer: Racer = {
        let (destination, evicted) = (destination.clone(), evicted.clone());
        Box::new(move || {
            let stage = fs::read_dir(&destination)
                .and_then(|mut entries| entries.next().expect("the stage"))
                .expect("list the destination")
                .path();
            let claimed = stage.join("PROGRA~1");
            evicted.store(super::fs_quirks::evict(&claimed), Ordering::SeqCst);
        })
    };
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before: vec![(Op::OpenWrite, "PROGRA~1", 0, racer)],
            ..Quirks::vfat()
        },
    );
    let target = destination.join("target");

    copy(&source, &target)?;

    assert!(evicted.load(Ordering::SeqCst));
    assert_eq!(fs::read_to_string(target.join("PROGRA~1"))?, "PROGRA~1");
    Ok(())
}

#[test]
fn a_source_folder_evicted_before_its_job_is_copied() -> Result<(), Box<dyn Error>> {
    use std::sync::atomic::Ordering;
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let (sources, destination) = (root.path().join("sources"), root.path().join("destination"));
    let folder = sources.join("folder");
    fs::create_dir_all(folder.join("sub"))?;
    fs::create_dir(&destination)?;
    fs::write(folder.join("sub/a.txt"), b"a")?;
    let evicted = Arc::new(AtomicBool::new(false));
    let racer: Racer = {
        let (folder, evicted) = (folder.clone(), evicted.clone());
        Box::new(move || evicted.store(super::fs_quirks::evict(&folder), Ordering::SeqCst))
    };
    let _sources = FsQuirks::install(
        &sources,
        Quirks {
            before: vec![(Op::OpenDir, "folder", 0, racer)],
            ..Quirks::sshfs()
        },
    );
    let _destination = FsQuirks::install(&destination, Quirks::default());

    copy(&folder, &destination.join("copy"))?;

    assert!(evicted.load(Ordering::SeqCst));
    assert_eq!(fs::read(destination.join("copy/sub/a.txt"))?, b"a");
    Ok(())
}

#[test]
fn a_cancelled_copy_on_nfs_leaves_no_stage() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let mounts = mounts(Quirks {
        before: vec![(Op::Write, "*", 0, Box::new(move || cancel.cancel()))],
        ..Quirks::nfs3()
    })?;
    let source = mounts.sources.join("source");
    fs::create_dir_all(source.join("nested"))?;
    for index in 0..8 {
        fs::write(source.join(format!("{index}.bin")), vec![0x5a; 256 * 1024])?;
        fs::write(
            source.join(format!("nested/{index}.bin")),
            vec![0xa5; 256 * 1024],
        )?;
    }

    let result = glib::MainContext::default().block_on(copy_new_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(mounts.destination.join("target")),
        cancellable,
    ));

    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)),
        "{result:?}"
    );
    assert_eq!(names(&mounts.destination)?, Vec::<String>::new());
    Ok(())
}

fn assert_only_complete(
    row: &str,
    live: &Path,
    source: &Path,
    relative: &Path,
    existing: &[&str],
) -> Result<(), Box<dyn Error>> {
    for name in names(&live.join(relative))? {
        assert!(
            !name.starts_with(".strata-"),
            "{row}: {name} left in {relative:?}"
        );
        let path = relative.join(&name);
        let is_existing = existing.iter().any(|existing| path == Path::new(existing));
        if fs::symlink_metadata(live.join(&path))?.is_dir() {
            if !is_existing {
                assert_eq!(
                    names(&live.join(&path))?,
                    names(&source.join(&path))?,
                    "{row}: {path:?} is incomplete"
                );
            }
            assert_only_complete(row, live, source, &path, existing)?;
        } else if !is_existing {
            let (copied, original) = (fs::read(live.join(&path))?, fs::read(source.join(&path))?);
            assert!(
                copied == original,
                "{row}: {path:?} has {} of {} bytes",
                copied.len(),
                original.len()
            );
        }
    }
    Ok(())
}

#[test]
fn a_stopped_merge_leaves_nothing_partial_at_a_real_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let presets: [(&str, Preset); 2] = [
        ("staged", Quirks::nfs3),
        ("in place", Quirks::without_folder_placement),
    ];
    for (row, preset) in presets {
        for cancelled in [true, false] {
            let cancellable = gio::Cancellable::new();
            let cancel = cancellable.clone();
            let mut quirks = preset();
            if cancelled {
                quirks
                    .before
                    .push((Op::Write, "*", 0, Box::new(move || cancel.cancel())));
            } else {
                quirks.fail.push((Op::Write, "*", 0, Errno::IO));
            }
            let mounts = mounts(quirks)?;
            if row == "in place" {
                learn_mount_capability(mounts.destination_scope.key(), |capabilities| {
                    capabilities.no_replace = Some(false);
                    capabilities.hard_links = Some(true);
                    capabilities.rename_over_empty_directory = Some(false);
                });
            }
            let source = mounts.sources.join("source");
            let live = mounts.destination.join("live");
            for folder in ["new/deeper", "sub/newer"] {
                fs::create_dir_all(source.join(folder))?;
            }
            for (folder, count) in [
                ("", 8),
                ("new", 8),
                ("new/deeper", 4),
                ("sub", 4),
                ("sub/newer", 4),
            ] {
                for index in 0..count {
                    let file = source.join(folder).join(format!("{index}.bin"));
                    fs::write(file, vec![index as u8; 256 * 1024])?;
                }
            }
            fs::create_dir_all(live.join("sub"))?;
            fs::write(live.join("stays.txt"), b"keep me")?;
            fs::write(live.join("sub/old.txt"), b"old")?;

            let result = glib::MainContext::default().block_on(copy_recursively(
                gio::File::for_path(&source),
                gio::File::for_path(&live),
                true,
                cancellable,
            ));

            let row = format!("{row}, cancelled: {cancelled}");
            assert!(
                result.as_ref().is_err_and(|error| {
                    error.matches(gio::IOErrorEnum::Cancelled) == cancelled
                }),
                "{row}: {result:?}"
            );
            assert_eq!(fs::read(live.join("stays.txt"))?, b"keep me", "{row}");
            assert_eq!(fs::read(live.join("sub/old.txt"))?, b"old", "{row}");
            let existing = ["stays.txt", "sub", "sub/old.txt"];
            assert_only_complete(&row, &live, &source, Path::new(""), &existing)?;
        }
    }
    Ok(())
}

#[test]
fn a_folder_is_copied_in_place_where_nothing_can_place_it() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("nested/file.txt"), b"contents")?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let scope = FsQuirks::install(&destination, Quirks::without_folder_placement());
    learn_mount_capability(scope.key(), |capabilities| {
        capabilities.no_replace = Some(false);
        capabilities.hard_links = Some(true);
        capabilities.rename_over_empty_directory = Some(false);
    });

    copy(&source, &destination.join("target"))?;

    assert_eq!(names(&destination)?, ["target"]);
    assert_eq!(
        fs::read(destination.join("target/nested/file.txt"))?,
        b"contents"
    );
    assert!(!scope.used_name(".strata-*"), "{:?}", scope.calls());
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Interrupted {
    CloseFails,
    Cancelled,
    CloseFailsAndTheTargetIsSwapped,
}

#[test]
fn a_failed_copy_in_place_removes_only_what_it_made() -> Result<(), Box<dyn Error>> {
    use Interrupted::*;
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let rows: [(&[&str], Option<&str>, Interrupted); 5] = [
        (&["a.txt", "nested/b.txt"], None, CloseFails),
        (&["a.txt"], Some("a.txt"), CloseFails),
        (&["nested/b.txt"], Some("nested/b.txt"), CloseFails),
        (&["a.txt"], Some("a.txt"), Cancelled),
        (&["a.txt"], Some("a.txt"), CloseFailsAndTheTargetIsSwapped),
    ];
    for (files, opened_elsewhere, interrupted) in rows {
        let row = format!("{files:?}, {interrupted:?}");
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        fs::create_dir_all(source.join("nested"))?;
        for file in files {
            fs::write(source.join(file), b"copied")?;
        }
        std::os::unix::fs::symlink("a.txt", source.join("link"))?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let target = destination.join("target");
        let swapped = destination.join("target.moved");
        let cancellable = gio::Cancellable::new();
        let (live, cancel) = (target.clone(), cancellable.clone());
        let holds = Arc::new(std::sync::Mutex::new(Vec::new()));
        let held = holds.clone();
        let racer: Racer = Box::new(move || {
            match opened_elsewhere {
                Some(file) => held.lock().expect("the holds").push(hold(&live.join(file))),
                None => fs::write(live.join("user.txt"), b"user data").expect("write"),
            }
            if interrupted == Cancelled {
                cancel.cancel();
            }
        });
        let closed = opened_elsewhere
            .and_then(|file| file.rsplit('/').next())
            .unwrap_or("*");
        let (live, aside) = (target.clone(), swapped.clone());
        let meanwhile: (Op, &str, usize, Racer) = match interrupted {
            CloseFailsAndTheTargetIsSwapped => (
                Op::Unlink,
                "target",
                0,
                Box::new(move || {
                    fs::rename(&live, &aside).expect("move the target");
                    fs::create_dir(&live).expect("make another target");
                }),
            ),
            _ => (
                Op::Names,
                "*",
                0,
                Box::new(move || holds.lock().expect("the holds").clear()),
            ),
        };
        let scope = FsQuirks::install(
            &destination,
            Quirks {
                before: vec![(Op::Close, closed, 0, racer)],
                after: vec![meanwhile],
                fail: if interrupted == Cancelled {
                    Vec::new()
                } else {
                    vec![(Op::Close, "*", 0, Errno::IO)]
                },
                rename_over_empty_dir: OverEmptyDirectory::NotEmpty,
                ..Quirks::nfs3()
            },
        );
        learn_mount_capability(scope.key(), |capabilities| {
            capabilities.no_replace = Some(false);
            capabilities.hard_links = Some(true);
            capabilities.rename_over_empty_directory = Some(false);
        });

        let result = glib::MainContext::default().block_on(copy_new_recursively(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            cancellable,
        ));

        assert!(result.is_err(), "{row}: {result:?}");
        if opened_elsewhere.is_none() {
            assert_eq!(names(&target)?, ["user.txt"]);
            assert_eq!(fs::read(target.join("user.txt"))?, b"user data");
            continue;
        }
        if interrupted == CloseFailsAndTheTargetIsSwapped {
            assert!(
                target.is_dir() && names(&target)?.is_empty(),
                "{row}: {result:?}"
            );
            let left = names(&swapped)?;
            assert!(
                left.len() == 1 && foreign_leftover(OsStr::new(&left[0])).is_some(),
                "{row}: {left:?}"
            );
            continue;
        }
        if interrupted == Cancelled {
            assert!(
                result
                    .as_ref()
                    .is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)),
                "{row}: {result:?}"
            );
        }
        assert!(!target.exists(), "{row}: {result:?}");
    }
    Ok(())
}

#[test]
fn an_item_put_at_a_linked_name_right_after_the_link_is_kept() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("a"), b"linked")?;
    fs::hard_link(source.join("a"), source.join("a2"))?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let target = destination.join("target");
    // Either name may be the one linked; its check is its first stat.
    let racer = |name: &'static str, other: &'static str| -> Racer {
        let live = target.clone();
        Box::new(move || {
            let inode = |name| fs::metadata(live.join(name)).map(|metadata| metadata.ino());
            let inode = |name| inode(name).ok();
            if inode(name).is_some() && inode(name) == inode(other) {
                fs::write(live.join("user"), b"user data").expect("write the user's item");
                fs::rename(live.join("user"), live.join(name)).expect("replace the link");
            }
        })
    };
    let scope = FsQuirks::install(
        &destination,
        Quirks {
            before: vec![
                (Op::Stat, "a", 0, racer("a", "a2")),
                (Op::Stat, "a2", 0, racer("a2", "a")),
            ],
            ..Quirks::without_folder_placement()
        },
    );
    learn_mount_capability(scope.key(), |capabilities| {
        capabilities.no_replace = Some(false);
        capabilities.hard_links = Some(true);
        capabilities.rename_over_empty_directory = Some(false);
    });

    let result = copy(&source, &target);

    assert!(result.is_err(), "{result:?}");
    let names = names(&target)?;
    assert_eq!(names.len(), 1, "{names:?}");
    assert_eq!(fs::read(target.join(&names[0]))?, b"user data");
    Ok(())
}

#[test]
fn a_stage_with_read_only_folders_is_removed_when_it_cannot_be_published()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let smb: Preset = || Quirks {
        mount_type: Some(("cifs", "rw")),
        stale_read_only: true,
        ..Quirks::default()
    };
    for (row, preset) in [("default", Quirks::default as Preset), ("SMB", smb)] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        fs::create_dir_all(source.join("sub"))?;
        fs::write(source.join("sub/file.txt"), b"contents")?;
        set_mode(&source.join("sub"), 0o555)?;
        set_mode(&source, 0o555)?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let taken = destination.join("target");
        let racer: Racer = Box::new(move || fs::create_dir(&taken).expect("take the name"));
        let _scope = FsQuirks::install(
            &destination,
            Quirks {
                before: vec![(Op::Rename, ".strata-copy-*", 0, racer)],
                ..preset()
            },
        );

        let result = copy(&source, &destination.join("target"));

        unlock(&source)?;
        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.matches(gio::IOErrorEnum::Exists)),
            "{row}: {result:?}"
        );
        assert_eq!(names(&destination)?, ["target"], "{row}");
        assert_eq!(
            names(&destination.join("target"))?,
            Vec::<String>::new(),
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn a_copied_folder_keeps_its_hard_links_where_the_destination_can() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    type Learned = fn(&mut MountCapabilities);
    let in_place: Learned = |capabilities| {
        capabilities.no_replace = Some(false);
        capabilities.hard_links = Some(true);
        capabilities.rename_over_empty_directory = Some(false);
    };
    let rows: [(&str, Preset, Option<Learned>, bool); 5] = [
        ("ext4", Quirks::ext4, None, true),
        ("nfs3", Quirks::nfs3, None, true),
        (
            "in place",
            Quirks::without_folder_placement,
            Some(in_place),
            true,
        ),
        ("vfat", Quirks::vfat, None, false),
        ("sshfs", Quirks::sshfs, None, false),
    ];
    for (row, preset, learned, linked) in rows {
        let mounts = mounts(preset())?;
        if let Some(learned) = learned {
            learn_mount_capability(mounts.destination_scope.key(), learned);
        }
        let source = mounts.sources.join("source");
        fs::create_dir_all(source.join("sub"))?;
        fs::write(source.join("a"), b"linked")?;
        fs::hard_link(source.join("a"), source.join("sub/b"))?;
        fs::write(source.join("c"), b"single")?;
        fs::hard_link(source.join("c"), mounts.sources.join("outside"))?;
        let folders = [("sub", 1_000_000_000), ("", 1_100_000_000)];
        for (folder, seconds) in folders {
            set_mtime(&source.join(folder), seconds)?;
        }
        let target = mounts.destination.join("target");

        copy(&source, &target)?;

        let [a, b, c] = ["a", "sub/b", "c"].map(|name| fs::metadata(target.join(name)));
        let (a, b, c) = (a?, b?, c?);
        assert_eq!(a.ino() == b.ino(), linked, "{row}");
        let links = if linked { 2 } else { 1 };
        assert_eq!(
            (a.nlink(), b.nlink(), c.nlink()),
            (links, links, 1),
            "{row}"
        );
        assert_ne!(c.ino(), a.ino(), "{row}");
        assert_eq!(fs::read(target.join("a"))?, b"linked", "{row}");
        assert_eq!(fs::read(target.join("sub/b"))?, b"linked", "{row}");
        assert_eq!(fs::read(target.join("c"))?, b"single", "{row}");
        for (folder, seconds) in folders {
            let mtime = fs::metadata(target.join(folder))?.mtime();
            assert_eq!(mtime, seconds, "{row}: {folder:?}");
        }
    }
    Ok(())
}

#[test]
fn names_of_a_linked_file_on_an_smb_share_are_opened_one_at_a_time() -> Result<(), Box<dyn Error>> {
    use std::sync::{OnceLock, atomic::Ordering};
    type Reader = Box<dyn Fn() -> Vec<Call> + Send + Sync>;
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // Two workers copy both names: the destination keeps no links, or the
    // client shows a link count of 1, as it does for files it listed. The
    // first name opened stays open until the other was looked at.
    let rows: [(&str, bool, Preset); 2] = [
        ("no links at the destination", false, Quirks::vfat),
        ("listed link count", true, Quirks::default),
    ];
    for (row, single_link_count, destination_quirks) in rows {
        let reader: Arc<OnceLock<Reader>> = Arc::default();
        let read = reader.clone();
        let both_looked_at = move || {
            read.get().is_some_and(|calls| {
                let calls = calls();
                ["a", "b"].iter().all(|name| {
                    calls
                        .iter()
                        .any(|call| call.op == Op::Stat && call.names == [*name])
                })
            })
        };
        let timed_out = Arc::new(AtomicBool::new(false));
        let timer = timed_out.clone();
        let hold: Racer = Box::new(move || {
            let start = Instant::now();
            while !both_looked_at() {
                if start.elapsed() > Duration::from_secs(5) {
                    timer.store(true, Ordering::Relaxed);
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        let source = sources.join("folder");
        fs::create_dir_all(&source)?;
        fs::create_dir(&destination)?;
        fs::write(source.join("a"), b"linked")?;
        fs::hard_link(source.join("a"), source.join("b"))?;
        let source_scope = FsQuirks::install(
            &sources,
            Quirks {
                mount_type: Some(("cifs", "rw")),
                name_opens: NameOpens::OneAtATime,
                single_link_count,
                after: vec![(Op::OpenSource, "*", 0, hold)],
                ..Quirks::default()
            },
        );
        let _destination_scope = FsQuirks::install(&destination, destination_quirks());
        let _ = reader.set(Box::new(source_scope.reader()));
        let target = destination.join("folder");

        glib::MainContext::default().block_on(copy_local(
            source,
            Destination::New(target.clone(), Taken::Fail),
            Options {
                workers: Some(2),
                ..Options::default()
            },
            None,
            &gio::Cancellable::new(),
        ))?;

        assert!(
            !timed_out.load(Ordering::Relaxed),
            "{row}: the other name was never looked at"
        );
        for name in ["a", "b"] {
            assert_eq!(fs::read(target.join(name))?, b"linked", "{row}: {name}");
        }
    }
    Ok(())
}

#[test]
fn probing_for_hard_links_leaves_nothing_in_the_copy() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // The probe's link is made, but its reply is lost, so it tries another name.
    let mounts = mounts(Quirks {
        lost_reply: vec![(Op::Link, ".strata-probe-*", Errno::EXIST)],
        ..Quirks::nfs3()
    })?;
    let source = mounts.sources.join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("a"), b"linked")?;
    fs::hard_link(source.join("a"), source.join("b"))?;
    let target = mounts.destination.join("target");

    copy(&source, &target)?;

    assert_eq!(names(&target)?, ["a", "b"]);
    assert_eq!(
        fs::metadata(target.join("a"))?.ino(),
        fs::metadata(target.join("b"))?.ino()
    );
    Ok(())
}

#[test]
fn a_claimed_name_is_linked_when_the_reply_to_removing_its_claim_is_lost()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let mounts = mounts(Quirks {
        lost_reply: vec![
            (Op::Unlink, "x", Errno::NOENT),
            (Op::Unlink, "y", Errno::NOENT),
        ],
        ..Quirks::nfs3()
    })?;
    learn_mount_capability(mounts.destination_scope.key(), |capabilities| {
        capabilities.hard_links = Some(true);
    });
    let source = mounts.sources.join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("x"), b"linked")?;
    fs::hard_link(source.join("x"), source.join("y"))?;
    for name in ["X", "Y"] {
        fs::write(source.join(name), name)?;
    }
    let target = mounts.destination.join("target");

    copy(&source, &target)?;

    let [x, y] = ["x", "y"].map(|name| fs::metadata(target.join(name)));
    let (x, y) = (x?, y?);
    assert_eq!((x.ino(), x.nlink()), (y.ino(), 2));
    for name in ["X", "Y"] {
        assert_eq!(fs::read_to_string(target.join(name))?, name);
    }
    Ok(())
}

#[test]
fn a_refused_link_is_copied_from_the_first_copy_and_ends_linking() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let mounts = mounts(Quirks {
        fail: vec![(Op::Link, "*", 0, Errno::MLINK)],
        ..Quirks::default()
    })?;
    learn_mount_capability(mounts.destination_scope.key(), |capabilities| {
        capabilities.hard_links = Some(true);
    });
    let source = mounts.sources.join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("a"), b"linked")?;
    for name in ["b", "d"] {
        fs::hard_link(source.join("a"), source.join(name))?;
    }
    let target = mounts.destination.join("target");

    copy(&source, &target)?;

    let mut inodes = HashSet::new();
    for name in ["a", "b", "d"] {
        let metadata = fs::metadata(target.join(name))?;
        assert_eq!(metadata.nlink(), 1, "{name}");
        inodes.insert(metadata.ino());
        assert_eq!(fs::read(target.join(name))?, b"linked", "{name}");
    }
    assert_eq!(inodes.len(), 3);
    assert_eq!(mounts.destination_scope.count(Op::Link), 1);
    let read = mounts
        .source_scope
        .calls()
        .into_iter()
        .filter(|call| call.op == Op::Read)
        .flat_map(|call| call.names)
        .collect::<HashSet<_>>();
    assert_eq!(read.len(), 1, "the source is read once: {read:?}");
    Ok(())
}

#[test]
fn a_copy_calls_an_entry_changed_only_when_its_name_no_longer_holds_it()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let smb = || Quirks {
        mount_type: Some(("cifs", "rw")),
        ..Quirks::default()
    };
    let plain = Quirks::default as Preset;
    for (row, selected, merge, (preset, op, name, error), changed) in [
        (
            "selected file",
            "folder/keys",
            false,
            (plain, Op::OpenSource, "keys", Errno::ACCESS),
            false,
        ),
        (
            "file",
            "folder",
            false,
            (plain, Op::OpenSource, "keys", Errno::ACCESS),
            false,
        ),
        (
            "folder",
            "folder",
            false,
            (plain, Op::OpenDir, "secrets", Errno::ACCESS),
            false,
        ),
        (
            "file on SMB",
            "folder",
            false,
            (smb, Op::Stat, "keys", Errno::ACCESS),
            false,
        ),
        (
            "merged folder",
            "folder",
            true,
            (plain, Op::OpenDir, "sub", Errno::ACCESS),
            false,
        ),
        (
            "file gone",
            "folder",
            false,
            (plain, Op::OpenSource, "keys", Errno::NOENT),
            true,
        ),
        (
            "folder now a link",
            "folder",
            false,
            (plain, Op::OpenDir, "secrets", Errno::LOOP),
            true,
        ),
    ] {
        let root = tempfile::tempdir()?;
        let (sources, destination) = (root.path().join("sources"), root.path().join("destination"));
        let folder = sources.join("folder");
        for made in [
            folder.join("secrets"),
            folder.join("sub"),
            destination.join("folder/sub"),
        ] {
            fs::create_dir_all(made)?;
        }
        for file in ["keys", "secrets/a", "sub/b"] {
            fs::write(folder.join(file), file)?;
        }
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                fail: vec![(op, name, 0, error)],
                ..preset()
            },
        );
        let _destination = FsQuirks::install(&destination, Quirks::default());
        let target = if merge {
            Destination::Merge(destination.join("folder"), None)
        } else {
            Destination::New(destination.join("copy"), Taken::Fail)
        };

        let (copied, _) = reporting(&sources.join(selected), target, Options::default());

        let error = crate::services::io_error_detail(&io::Error::from(error));
        let expected = if changed {
            rust_i18n::t!(
                "%{name} changed while it was being copied: %{error}",
                name = name,
                error = error
            )
        } else {
            rust_i18n::t!(
                "Could not copy %{name}: %{error}",
                name = name,
                error = error
            )
        };
        assert_eq!(copied.expect_err(row).message(), expected, "{row}");
    }
    Ok(())
}
