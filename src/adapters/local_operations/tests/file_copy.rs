// SPDX-License-Identifier: MIT

use std::{
    io::Write,
    os::unix::fs::{FileExt, MetadataExt, PermissionsExt},
    sync::{Mutex, OnceLock, atomic::Ordering},
};

use rustix::io::Errno;

use super::fs_quirks::{Call, NameOpens, Op, Racer, Support, TmpfileLink, evict};
use super::*;
use crate::adapters::volume::{XattrNamespace, mount_capabilities};

struct Mounts {
    _root: tempfile::TempDir,
    sources: PathBuf,
    destination: PathBuf,
    source_scope: FsQuirks,
    destination_scope: FsQuirks,
}

fn mounts(
    source: Quirks,
    destination: impl FnOnce(&Path) -> Quirks,
) -> Result<Mounts, Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let target = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&target)?;
    Ok(Mounts {
        source_scope: FsQuirks::install(&sources, source),
        destination_scope: FsQuirks::install(&target, destination(&target)),
        _root: root,
        sources,
        destination: target,
    })
}

impl Mounts {
    fn source(&self, contents: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.sources.join("source.bin");
        fs::write(&path, contents)?;
        Ok(path)
    }

    fn target(&self) -> PathBuf {
        self.destination.join("target.bin")
    }
}

fn copy(source: &Path, target: &Path) -> Result<(), glib::Error> {
    glib::MainContext::default().block_on(copy_new_recursively(
        gio::File::for_path(source),
        gio::File::for_path(target),
        gio::Cancellable::new(),
    ))
}

fn patterned(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

fn list_into(folder: &Path, seen: &Arc<Mutex<Option<Vec<String>>>>) -> Racer {
    let (folder, seen) = (folder.to_path_buf(), seen.clone());
    Box::new(move || {
        *seen.lock().expect("listing") = Some(names(&folder).expect("list the destination"));
    })
}

fn ops(calls: &[Call]) -> Vec<Op> {
    calls.iter().map(|call| call.op).collect()
}

fn xattrs_set(calls: &[Call]) -> Vec<OsString> {
    calls
        .iter()
        .filter(|call| call.op == Op::SetXattr)
        .filter_map(|call| call.names.last().cloned())
        .collect()
}

#[test]
fn a_file_being_copied_is_invisible_until_complete() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for (case, preset, emulated) in [
        ("tmpfiles", Quirks::default as Preset, None),
        (
            "tmpfiles refused",
            || Quirks {
                tmpfile: Support::Rejected(Errno::OPNOTSUPP),
                ..Quirks::ext4()
            },
            Some((1, Some(false))),
        ),
        ("NFS", Quirks::nfs42, Some((0, None))),
    ] {
        let seen = Arc::new(Mutex::new(None));
        let mounts = mounts(Quirks::default(), |destination| Quirks {
            before: vec![(Op::Write, "*", 0, list_into(destination, &seen))],
            ..preset()
        })?;
        let contents = patterned(2 * 1024 * 1024);
        let source = mounts.source(&contents)?;

        copy(&source, &mounts.target())?;

        let seen = seen
            .lock()
            .map_err(|error| error.to_string())?
            .take()
            .ok_or("the copy wrote nothing")?;
        let scope = &mounts.destination_scope;
        let learned = mount_capabilities(scope.key()).tmpfile;
        let opened = scope.count(Op::OpenTmpfile);
        match emulated {
            Some(expected) => assert_eq!((opened, learned), expected, "{case}"),
            None if learned == Some(true) => assert_eq!(opened, 1, "{case}"),
            None => {}
        }
        if learned == Some(true) {
            assert!(seen.is_empty(), "{case}: {seen:?}");
        } else {
            assert!(
                matches!(seen.as_slice(), [stage] if stage.starts_with(".strata-copy-")),
                "{case}: {seen:?}"
            );
        }
        assert_eq!(names(&mounts.destination)?, ["target.bin"], "{case}");
        assert_eq!(fs::read(mounts.target())?, contents, "{case}");
    }
    Ok(())
}

#[test]
fn a_stage_only_a_held_descriptor_tells_apart_is_held_until_it_is_published()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let evicted = Arc::new(Mutex::new(None));
    let outcome = evicted.clone();
    let mounts = mounts(Quirks::default(), move |destination| {
        let folder = destination.to_path_buf();
        let evicting: Racer = Box::new(move || {
            let stage = names(&folder)
                .expect("list the destination")
                .into_iter()
                .find(|name| name.starts_with(".strata-copy-"))
                .expect("a stage");
            *outcome.lock().expect("outcome") = Some(evict(&folder.join(stage)));
        });
        Quirks {
            tmpfile: Support::Rejected(Errno::OPNOTSUPP),
            before: vec![(Op::Rename, ".strata-copy-*", 0, evicting)],
            ..Quirks::sshfs()
        }
    })?;
    let contents = patterned(64 * 1024);
    let source = mounts.source(&contents)?;

    copy(&source, &mounts.target())?;

    assert_eq!(
        *evicted.lock().map_err(|error| error.to_string())?,
        Some(false)
    );
    assert_eq!(names(&mounts.destination)?, ["target.bin"]);
    assert_eq!(fs::read(mounts.target())?, contents);
    Ok(())
}

#[test]
fn a_tmpfile_that_cannot_be_linked_is_published_without_rereading_the_source()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for (case, destination, staged) in [
        (
            "AT_EMPTY_PATH refused",
            (|| Quirks {
                tmpfile_link: TmpfileLink::ProcOnly,
                ..Quirks::default()
            }) as Preset,
            false,
        ),
        (
            "no form of link",
            || Quirks {
                tmpfile_link: TmpfileLink::Unavailable,
                ..Quirks::default()
            },
            true,
        ),
        (
            "links refused",
            || Quirks {
                links: Support::Rejected(Errno::MLINK),
                ..Quirks::default()
            },
            true,
        ),
    ] {
        // One pass reads the data, then the end; a third read would fail.
        let mounts = mounts(
            Quirks {
                fail: vec![(Op::Read, "source.bin", 2, Errno::IO)],
                ..Quirks::default()
            },
            |_| destination(),
        )?;
        let source = mounts.source(b"contents")?;

        copy(&source, &mounts.target())?;

        let scope = &mounts.destination_scope;
        assert_eq!(scope.used_name(".strata-copy-*"), staged, "{case}");
        assert_eq!(names(&mounts.destination)?, ["target.bin"], "{case}");
        assert_eq!(fs::read(mounts.target())?, b"contents", "{case}");
    }
    Ok(())
}

#[test]
fn a_name_taken_just_before_the_link_is_left_alone() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    fs::write(&source, b"ours")?;
    let target = root.path().join("target.txt");
    let taken = target.clone();
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(
                Op::LinkFd,
                "*",
                0,
                Box::new(move || fs::write(&taken, b"theirs").expect("take the name")),
            )],
            ..Quirks::default()
        },
    );

    let error = copy(&source, &target).expect_err("the name is taken");

    assert!(error.matches(gio::IOErrorEnum::Exists), "{error}");
    // Before Linux 6.10 the `AT_EMPTY_PATH` link fails before the name is
    // looked at and the `/proc` form follows; neither may try another name.
    let links: Vec<_> = scope
        .calls()
        .into_iter()
        .filter(|call| call.op == Op::LinkFd)
        .collect();
    assert!(!links.is_empty());
    assert!(
        links
            .iter()
            .all(|call| call.names == [OsString::from("target.txt")]),
        "{links:?}"
    );
    assert_eq!(fs::read(&target)?, b"theirs");
    assert_eq!(names(root.path())?, ["source.txt", "target.txt"]);
    Ok(())
}

#[test]
fn a_file_put_at_the_published_name_right_after_publishing_stays_there()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let elsewhere = tempfile::tempdir()?;
    let saved = elsewhere.path().join("saved.bin");
    fs::write(&saved, b"saved by another program")?;
    // Without `RENAME_NOREPLACE` the stage is published by a link, then its
    // own name is unlinked; another program renames over the name meanwhile.
    let mounts = mounts(Quirks::default(), |destination| {
        let over = destination.join("target.bin");
        let racer: Racer =
            Box::new(move || fs::rename(&saved, &over).expect("rename over the name"));
        Quirks {
            after: vec![(Op::Unlink, ".strata-copy-*", 0, racer)],
            ..Quirks::nfs3()
        }
    })?;
    let source = mounts.source(b"ours")?;

    copy(&source, &mounts.target())?;

    assert_eq!(fs::read(mounts.target())?, b"saved by another program");
    assert_eq!(names(&mounts.destination)?, ["target.bin"]);
    Ok(())
}

#[test]
fn a_folder_removed_before_the_link_leaves_tmpfiles_in_use() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    fs::write(&source, b"contents")?;
    let removed = root.path().join("removed");
    fs::create_dir(&removed)?;
    let folder = removed.clone();
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(
                Op::LinkFd,
                "*",
                0,
                Box::new(move || fs::remove_dir(&folder).expect("remove the folder")),
            )],
            ..Quirks::default()
        },
    );
    copy(&source, &removed.join("target.txt")).expect_err("the folder is gone");
    scope.clear_calls();

    copy(&source, &root.path().join("target.txt"))?;

    assert_eq!(scope.count(Op::OpenTmpfile), 1, "{:?}", scope.calls());
    assert!(!scope.used_name(".strata-copy-*"));
    assert_eq!(fs::read(root.path().join("target.txt"))?, b"contents");
    Ok(())
}

#[test]
fn what_a_mount_learns_of_its_copy_support_stays_with_that_mount() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for (case, learner, learned) in [
        (
            "AT_EMPTY_PATH refused",
            (|| Quirks {
                tmpfile_link: TmpfileLink::ProcOnly,
                ..Quirks::default()
            }) as Preset,
            [(Op::OpenTmpfile, 1), (Op::LinkFd, 1)],
        ),
        (
            "no form of link",
            || Quirks {
                tmpfile_link: TmpfileLink::Unavailable,
                ..Quirks::default()
            },
            [(Op::OpenTmpfile, 0), (Op::LinkFd, 0)],
        ),
        (
            "copy_file_range missing",
            || Quirks {
                fail: vec![(Op::CopyRange, "*", 0, Errno::NOSYS)],
                ..Quirks::default()
            },
            [(Op::CopyRange, 0), (Op::LinkFd, 1)],
        ),
    ] {
        let without_clones = |quirks: Quirks| Quirks {
            reflink: Support::Rejected(Errno::OPNOTSUPP),
            ..quirks
        };
        let (learning, bystanding) = (tempfile::tempdir()?, tempfile::tempdir()?);
        let learner_scope = FsQuirks::install(learning.path(), without_clones(learner()));
        let bystander_scope =
            FsQuirks::install(bystanding.path(), without_clones(Quirks::default()));
        for folder in [learning.path(), bystanding.path()] {
            fs::write(folder.join("source.bin"), b"contents")?;
        }
        copy(
            &learning.path().join("source.bin"),
            &learning.path().join("first.bin"),
        )?;
        copy(
            &bystanding.path().join("source.bin"),
            &bystanding.path().join("target.bin"),
        )?;
        learner_scope.clear_calls();
        drop(FsQuirks::install(
            tempfile::tempdir()?.path(),
            Quirks::default(),
        ));
        copy(
            &learning.path().join("source.bin"),
            &learning.path().join("second.bin"),
        )?;

        for (op, count) in learned {
            assert_eq!(learner_scope.count(op), count, "{case}: {op:?}");
        }
        assert_eq!(bystander_scope.count(Op::OpenTmpfile), 1, "{case}");
        assert_eq!(bystander_scope.count(Op::LinkFd), 1, "{case}");
        assert!(bystander_scope.count(Op::CopyRange) > 0, "{case}");
        assert!(!bystander_scope.used_name(".strata-copy-*"), "{case}");
        assert_eq!(fs::read(learning.path().join("second.bin"))?, b"contents");
        assert_eq!(fs::read(bystanding.path().join("target.bin"))?, b"contents");
    }
    Ok(())
}

#[test]
fn a_mount_found_unable_to_link_tmpfiles_stays_so_while_another_copy_opens_one()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let folder = tempfile::tempdir()?;
    let source = folder.path().join("source.bin");
    fs::write(&source, b"contents")?;
    let alongside = |name: &str| -> Racer {
        let (source, target) = (source.clone(), folder.path().join(name));
        Box::new(move || {
            glib::MainContext::new()
                .block_on(copy_new_recursively(
                    gio::File::for_path(&source),
                    gio::File::for_path(&target),
                    gio::Cancellable::new(),
                ))
                .expect("the copy alongside");
        })
    };
    let scope = FsQuirks::install(
        folder.path(),
        Quirks {
            tmpfile_link: TmpfileLink::Unavailable,
            before: vec![
                (Op::OpenTmpfile, "*", 0, alongside("first.bin")),
                (Op::LinkFd, "second.bin", 0, alongside("third.bin")),
            ],
            ..Quirks::default()
        },
    );

    copy(&source, &folder.path().join("second.bin"))?;

    assert_eq!(scope.count(Op::OpenTmpfile), 2);
    for name in ["first.bin", "second.bin", "third.bin"] {
        assert_eq!(fs::read(folder.path().join(name))?, b"contents", "{name}");
    }
    Ok(())
}

#[test]
fn running_out_of_space_leaves_nothing_behind() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for (case, preset) in [
        ("unnamed file", Quirks::default as Preset),
        ("named stage", || Quirks {
            tmpfile: Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::ext4()
        }),
    ] {
        let mounts = mounts(Quirks::default(), |_| Quirks {
            fail: vec![(Op::Write, "*", 1, Errno::NOSPC)],
            ..preset()
        })?;
        let source = mounts.source(&patterned(3 * 1024 * 1024))?;

        let error = copy(&source, &mounts.target()).expect_err("the second write fails");

        assert!(error.matches(gio::IOErrorEnum::NoSpace), "{case}: {error}");
        assert!(names(&mounts.destination)?.is_empty(), "{case}");
    }
    Ok(())
}

#[test]
fn short_reads_and_writes_still_copy_every_byte() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let short = || Quirks {
        short_io: Some(4096),
        ..Quirks::default()
    };
    let mounts = mounts(short(), |_| short())?;
    let contents = patterned(100 * 1024);
    let source = mounts.source(&contents)?;

    copy(&source, &mounts.target())?;

    assert_eq!(fs::read(mounts.target())?, contents);
    assert!(mounts.destination_scope.count(Op::Write) >= contents.len() / 4096);
    Ok(())
}

#[test]
fn a_write_error_reported_at_close_fails_the_copy() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // An unnamed file is closed once linked, under its inode's name.
    for (case, closed, preset) in [
        ("named stage", ".strata-copy-*", Quirks::nfs3 as Preset),
        ("unnamed file", "#*", Quirks::default),
    ] {
        let mounts = mounts(Quirks::default(), |_| Quirks {
            fail: vec![(Op::Close, closed, 0, Errno::IO)],
            ..preset()
        })?;
        let source = mounts.source(b"contents")?;

        copy(&source, &mounts.target()).expect_err("the close fails");

        assert!(mounts.destination_scope.used_name(closed), "{case}");
        assert!(names(&mounts.destination)?.is_empty(), "{case}");
    }
    Ok(())
}

#[test]
fn connection_loss_while_copying_metadata_fails_the_copy() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for op in [Op::XattrNames, Op::GetXattr, Op::SetXattr] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source.txt");
        fs::write(&source, b"contents")?;
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                extra_xattrs: vec![("user.tag", b"value")],
                fail: vec![(op, "*", 0, Errno::NOTCONN)],
                ..Quirks::default()
            },
        );

        copy(&source, &root.path().join("target.txt")).expect_err("the connection is lost");

        assert_eq!(names(root.path())?, ["source.txt"], "{op:?}");
    }
    Ok(())
}

#[test]
fn a_link_whose_times_fail_with_eio_is_not_left_behind() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    std::os::unix::fs::symlink("elsewhere", &source)?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::SetLinkTimes, "*", 0, Errno::IO)],
            ..Quirks::default()
        },
    );

    copy(&source, &root.path().join("target")).expect_err("the device fails");

    assert_eq!(names(root.path())?, ["source"]);
    Ok(())
}

#[test]
fn a_clone_failing_with_eio_is_not_retried() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    fs::write(&source, b"contents")?;
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::Clone, "*", 0, Errno::IO)],
            ..Quirks::default()
        },
    );

    copy(&source, &root.path().join("target.txt")).expect_err("the clone fails");

    assert_eq!(scope.count(Op::Clone), 1);
    assert_eq!(scope.count(Op::CopyRange) + scope.count(Op::Write), 0);
    assert_eq!(names(root.path())?, ["source.txt"]);
    Ok(())
}

#[test]
fn a_copy_range_that_ends_early_continues_with_reads_and_writes() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let contents = patterned(2 * 1024 * 1024);
    fs::write(&source, &contents)?;
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            reflink: Support::Rejected(Errno::OPNOTSUPP),
            copy_range_end: Some(1024 * 1024),
            ..Quirks::default()
        },
    );
    let target = root.path().join("target.bin");

    copy(&source, &target)?;

    assert!(scope.count(Op::CopyRange) > 0);
    assert!(fs::read(&target)? == contents, "the copy is not whole");
    Ok(())
}

#[test]
fn a_file_that_reports_no_size_is_copied_whole() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("cpuinfo");
    fs::write(&source, b"processor : 0\n")?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            mount_type: Some(("fuse.lxcfs", "rw")),
            virtual_sizes: true,
            ..Quirks::default()
        },
    );
    let target = root.path().join("copy");

    copy(&source, &target)?;

    assert_eq!(fs::read(&target)?, b"processor : 0\n");
    Ok(())
}

#[test]
fn copy_range_refused_across_mounts_continues_with_reads_and_writes() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let mounts = mounts(Quirks::default(), |_| Quirks::default())?;
    let contents = patterned(3 * 1024 * 1024);
    let source = mounts.source(&contents)?;

    copy(&source, &mounts.target())?;

    let calls = ops(&mounts.destination_scope.calls());
    let range = calls
        .iter()
        .position(|op| *op == Op::CopyRange)
        .ok_or("copy_file_range was not tried")?;
    assert!(calls[range..].contains(&Op::Write), "{calls:?}");
    assert_eq!(fs::read(mounts.target())?, contents);
    Ok(())
}

fn write_sparse(path: &Path) -> io::Result<u64> {
    let run = patterned(64 * 1024);
    let second = run.len() as u64 + 16 * 1024 * 1024;
    let size = second + run.len() as u64 + 1024 * 1024;
    let file = fs::File::create(path)?;
    file.write_all_at(&run, 0)?;
    file.write_all_at(&run, second)?;
    file.set_len(size)?;
    Ok(size)
}

fn append_to(path: &Path) -> Racer {
    let path = path.to_path_buf();
    Box::new(move || {
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(&[7; 64 * 1024]))
            .expect("grow the source");
    })
}

fn truncate(path: &Path) -> Racer {
    let path = path.to_path_buf();
    Box::new(move || {
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_len(64 * 1024))
            .expect("shrink the source");
    })
}

#[test]
fn sparse_files_keep_their_holes_where_both_ends_can() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let (ext4, vfat): (Preset, Preset) = (Quirks::ext4, Quirks::vfat);
    let zfs: Preset = || Quirks {
        mount_type: Some(("zfs", "rw")),
        ..Quirks::default()
    };
    let holes_unknown: Preset = || Quirks {
        fail: vec![(Op::SeekData, "*", 0, Errno::OPNOTSUPP)],
        ..Quirks::ext4()
    };
    let holes_lost: Preset = || Quirks {
        fail: vec![(Op::SeekHole, "*", 1, Errno::OPNOTSUPP)],
        ..Quirks::ext4()
    };
    let invalid: Preset = || Quirks {
        fail: vec![(Op::SeekData, "*", 0, Errno::INVAL)],
        ..Quirks::ext4()
    };
    let not_supported: Preset = || Quirks {
        fail: vec![(Op::SeekData, "*", 0, Errno::from_raw_os_error(524))],
        ..Quirks::ext4()
    };
    let ntfs3: Preset = || Quirks {
        mount_type: Some(("ntfs3", "rw")),
        ..Quirks::default()
    };
    // The source, the destination (`None` on the same mount), what happens
    // to the source as the copy runs, and whether holes are kept.
    type Row = (
        &'static str,
        Preset,
        Option<Preset>,
        Option<(Op, &'static str, fn(&Path) -> Racer)>,
        bool,
    );
    let rows: [Row; 13] = [
        ("one mount", ext4, None, None, true),
        ("two mounts", ext4, Some(ext4), None, true),
        (
            "grown meanwhile",
            ext4,
            Some(ext4),
            Some((Op::SeekHole, "source.img", append_to)),
            true,
        ),
        (
            "shrunk meanwhile",
            ext4,
            Some(ext4),
            Some((Op::SeekHole, "source.img", truncate)),
            true,
        ),
        ("FAT target", ext4, Some(vfat), None, false),
        ("ntfs3 target", ext4, Some(ntfs3), None, false),
        (
            "exfat-fuse target",
            ext4,
            Some(Quirks::exfat_fuse),
            None,
            false,
        ),
        ("ZFS source", zfs, Some(ext4), None, false),
        ("holes unknown", holes_unknown, Some(ext4), None, false),
        ("holes lost midway", holes_lost, Some(ext4), None, false),
        ("EINVAL", invalid, Some(ext4), None, false),
        ("ENOTSUPP", not_supported, Some(ext4), None, false),
        (
            "holes unknown, grown meanwhile",
            holes_unknown,
            None,
            Some((Op::CopyRange, "*", append_to)),
            false,
        ),
    ];
    for (case, source_preset, destination_preset, change, kept) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join("source.img");
        let size = write_sparse(&source)?;
        let mut quirks = Quirks {
            reflink: Support::Rejected(Errno::OPNOTSUPP),
            ..source_preset()
        };
        if let Some((op, pattern, change)) = change {
            quirks.before.push((op, pattern, 0, change(&source)));
        }
        let _scopes = match destination_preset {
            None => vec![FsQuirks::install(root.path(), quirks)],
            Some(preset) => vec![
                FsQuirks::install(&sources, quirks),
                FsQuirks::install(&destination, preset()),
            ],
        };

        let events = run_paste(PasteRequest {
            id: OperationRequestId(1),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            move_sources: false,
        });

        let copy = destination.join("source.img");
        assert!(
            fs::read(&copy)? == fs::read(&source)?,
            "{case}: the copy differs"
        );
        let allocated = fs::metadata(&copy)?.blocks() * 512;
        if kept {
            let source_allocated = fs::metadata(&source)?.blocks() * 512;
            assert!(
                allocated <= 2 * source_allocated,
                "{case}: {allocated} bytes allocated for {source_allocated} of data"
            );
        } else if destination_preset.is_some() {
            // Within one filesystem, copy_file_range may share the source's
            // extents, holes included (btrfs, XFS).
            assert!(
                allocated >= size,
                "{case}: {allocated} of {size} bytes allocated"
            );
        }
        let largest = size.max(fs::metadata(&source)?.len());
        assert_eq!(counted_bytes(&events), Some(largest), "{case}");
    }
    Ok(())
}

fn counted_bytes(events: &[OperationEvent]) -> Option<u64> {
    events.iter().rev().find_map(|event| match event {
        OperationEvent::TransferProgress {
            completed_items: 0,
            transferred_bytes,
            ..
        } => Some(*transferred_bytes),
        _ => None,
    })
}

#[test]
fn a_sparse_source_that_grows_is_counted_once() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for (case, at_end) in [("past a gap", false), ("extending the last run", true)] {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join("source.img");
        let run = patterned(64 * 1024);
        let second = run.len() as u64 + 16 * 1024 * 1024;
        let file = fs::File::create(&source)?;
        file.write_all_at(&run, 0)?;
        file.write_all_at(&run, second)?;
        if !at_end {
            file.set_len(second + run.len() as u64 + 1024 * 1024)?;
        }
        drop(file);
        let size = fs::metadata(&source)?.len();
        let grown = source.clone();
        let racer: Racer = Box::new(move || {
            let offset = if at_end { size } else { size + 1024 * 1024 };
            fs::OpenOptions::new()
                .write(true)
                .open(&grown)
                .and_then(|file| file.write_all_at(&[7; 64 * 1024], offset))
                .expect("grow the source");
        });
        let _scopes = [
            FsQuirks::install(
                &sources,
                Quirks {
                    reflink: Support::Rejected(Errno::OPNOTSUPP),
                    before: vec![(Op::SeekHole, "source.img", usize::from(at_end), racer)],
                    ..Quirks::ext4()
                },
            ),
            FsQuirks::install(&destination, Quirks::ext4()),
        ];

        let events = run_paste(PasteRequest {
            id: OperationRequestId(1),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            move_sources: false,
        });

        assert!(
            fs::read(destination.join("source.img"))? == fs::read(&source)?,
            "{case}: the copy differs"
        );
        let grown = fs::metadata(&source)?.len();
        assert!(grown > size, "{case}");
        assert_eq!(counted_bytes(&events), Some(grown), "{case}");
    }
    Ok(())
}

#[test]
fn times_survive_filesystems_that_reset_them() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for (case, preset, (before, after)) in [
        ("NFSv4.2", Quirks::nfs42 as Preset, (false, true)),
        ("NFSv3", Quirks::nfs3, (true, false)),
        (
            "FUSE writeback cache",
            || Quirks {
                times_reset_at_close: true,
                ..Quirks::sshfs()
            },
            (true, true),
        ),
    ] {
        let mounts = mounts(Quirks::default(), |_| preset())?;
        let source = mounts.source(b"contents")?;
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        fs::File::open(&source)?.set_modified(modified)?;

        copy(&source, &mounts.target())?;

        let calls = ops(&mounts.destination_scope.calls());
        let closed = calls
            .iter()
            .position(|op| *op == Op::Close)
            .ok_or("the stage was not closed")?;
        assert_eq!(
            (
                calls[..closed].contains(&Op::SetTimes),
                calls[closed..].contains(&Op::SetTimes)
            ),
            (before, after),
            "{case}: {calls:?}"
        );
        assert_eq!(
            fs::metadata(mounts.target())?.modified()?,
            modified,
            "{case}"
        );
    }
    Ok(())
}

#[test]
fn copies_their_owner_cannot_read_keep_their_times_on_nfs4() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for mode in [0o244, 0o044] {
        let mounts = mounts(
            Quirks {
                reported_mode: Some(("source.bin", mode)),
                ..Quirks::default()
            },
            |_| Quirks::nfs42(),
        )?;
        let source = mounts.source(b"contents")?;
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        fs::File::open(&source)?.set_modified(modified)?;

        copy(&source, &mounts.target())?;

        let copied = fs::metadata(mounts.target())?;
        assert_eq!(copied.mode() & 0o777, mode, "{mode:o}");
        assert_eq!(copied.modified()?, modified, "{mode:o}");
    }
    Ok(())
}

#[test]
fn special_bits_are_dropped_when_the_owner_cannot_be_kept() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for (case, mode, shared_destination) in [
        ("group refused", 0o4755, false),
        ("setgid destination", 0o2755, true),
    ] {
        let mounts = mounts(
            Quirks {
                foreign_group: true,
                ..Quirks::default()
            },
            |_| Quirks {
                chown: Support::Rejected(Errno::PERM),
                ..Quirks::default()
            },
        )?;
        if shared_destination {
            fs::set_permissions(&mounts.destination, fs::Permissions::from_mode(0o2775))?;
        }
        let source = mounts.source(b"#!/bin/sh\n")?;
        fs::set_permissions(&source, fs::Permissions::from_mode(mode))?;

        copy(&source, &mounts.target())?;

        let copied = fs::metadata(mounts.target())?;
        assert_eq!(copied.mode() & 0o7777, 0o755, "{case}");
        // A setgid folder's group is kept, so the owner is never changed there.
        assert_eq!(
            mounts.destination_scope.count(Op::ChownFd),
            usize::from(!shared_destination),
            "{case}"
        );
        assert_eq!(
            copied.gid(),
            fs::metadata(&mounts.destination)?.gid(),
            "{case}"
        );
    }
    Ok(())
}

#[test]
fn xattr_namespaces_are_learned_separately() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let first = root.path().join("first.txt");
    let second = root.path().join("second.txt");
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;
    let target = root.path().join("copies");
    fs::create_dir(&target)?;

    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            xattrs_refused: vec![("user.", Errno::OPNOTSUPP)],
            extra_xattrs: vec![("user.tag", b"1"), ("security.label", b"2")],
            ..Quirks::default()
        },
    );
    copy(&first, &target.join("first.txt"))?;
    scope.clear_calls();
    copy(&second, &target.join("second.txt"))?;
    assert_eq!(
        xattrs_set(&scope.calls()),
        [OsString::from("security.label")]
    );
    let refused = mount_capabilities(scope.key()).refused_xattrs;
    assert!(refused.contains(XattrNamespace::User));
    assert!(!refused.contains(XattrNamespace::Security));
    drop(scope);

    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            extra_xattrs: vec![
                ("security.evm", b"stale"),
                ("$LXMOD", b"\x00\x00"),
                ("user.tag", b"kept"),
            ],
            ..Quirks::default()
        },
    );
    copy(&first, &target.join("third.txt"))?;
    assert_eq!(xattrs_set(&scope.calls()), [OsString::from("user.tag")]);
    Ok(())
}

#[test]
fn xattrs_are_kept_when_an_acl_takes_the_owners_write_access() -> Result<(), Box<dyn Error>> {
    use rustix::fs::{XattrFlags, getxattr, setxattr};
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    fs::write(&source, b"contents")?;
    setxattr(
        &source,
        "user.strata-test",
        b"metadata",
        XattrFlags::empty(),
    )?;
    // Read-only for everyone; the named user makes it an ACL rather than a mode.
    let entry = |tag: u16, id: u32| {
        [
            &tag.to_le_bytes()[..],
            &4u16.to_le_bytes(),
            &id.to_le_bytes(),
        ]
        .concat()
    };
    let acl = [
        2u32.to_le_bytes().to_vec(),
        entry(0x01, u32::MAX),
        entry(0x02, 4321),
        entry(0x04, u32::MAX),
        entry(0x10, u32::MAX),
        entry(0x20, u32::MAX),
    ]
    .concat();
    setxattr(
        &source,
        "system.posix_acl_access",
        &acl,
        XattrFlags::empty(),
    )?;
    let target = root.path().join("copy.txt");
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            acls_listed_first: true,
            ..Quirks::default()
        },
    );

    copy(&source, &target)?;

    let mut value = [0u8; 64];
    let length = getxattr(&target, "user.strata-test", &mut value[..])?;
    assert_eq!(&value[..length], b"metadata");
    let length = getxattr(&target, "system.posix_acl_access", &mut value[..])?;
    assert_eq!(&value[..length], acl);
    Ok(())
}

#[test]
fn selected_names_of_a_linked_file_on_an_smb_share_are_opened_one_at_a_time()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // The first linked name opened stays open until the other was looked
    // at, as its copy does in parallel just before opening it; small reads
    // keep the first open while that copy would open the other. The client
    // may show a link count of 1, as it does for files it listed.
    type Reader = Box<dyn Fn() -> Vec<Call> + Send + Sync>;
    for single_link_count in [false, true] {
        let reader: Arc<OnceLock<Reader>> = Arc::default();
        let read = reader.clone();
        let both_looked_at = move || {
            read.get().is_some_and(|calls| {
                let calls = calls();
                ["linked-a", "linked-b"].iter().all(|name| {
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
        let smb = Quirks {
            mount_type: Some(("cifs", "rw")),
            name_opens: NameOpens::OneAtATime,
            short_io: Some(4096),
            single_link_count,
            after: vec![(Op::OpenSource, "linked-*", 0, hold)],
            ..Quirks::default()
        };
        let mounts = mounts(smb, |_| Quirks {
            rotational: Some(false),
            ..Quirks::default()
        })?;
        if thread::available_parallelism().map_or(1, usize::from) < 2 {
            eprintln!("skipped: the bulk path copies one file at a time on this machine");
            return Ok(());
        }
        let _ = reader.set(Box::new(mounts.source_scope.reader()));
        fs::write(mounts.sources.join("linked-a"), patterned(1 << 20))?;
        fs::hard_link(
            mounts.sources.join("linked-a"),
            mounts.sources.join("linked-b"),
        )?;
        // Enough files for the bulk path, the linked names first, so they are
        // copied in parallel.
        let mut names = vec!["linked-a".to_owned(), "linked-b".to_owned()];
        for index in 0..8 {
            let name = format!("file-{index}");
            fs::write(mounts.sources.join(&name), &name)?;
            names.push(name);
        }

        run_paste(PasteRequest {
            id: OperationRequestId(98 + u64::from(single_link_count)),
            destination: Location::local(&mounts.destination),
            items: names
                .iter()
                .map(|name| PasteItem {
                    source: Location::local(mounts.sources.join(name)),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                })
                .collect(),
            move_sources: false,
        });

        assert!(
            !timed_out.load(Ordering::Relaxed),
            "{single_link_count}: the other linked name was never looked at while the first was open"
        );
        for name in &names {
            assert_eq!(
                fs::read(mounts.destination.join(name))?,
                fs::read(mounts.sources.join(name))?,
                "{single_link_count}: {name}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_linked_name_on_an_smb_share_swapped_before_its_open_is_not_copied()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // The hold is on the file seen at the name, so it keeps no other copy
    // off a file put there meanwhile, which may have a name open elsewhere.
    let swapped: Arc<OnceLock<PathBuf>> = Arc::default();
    let at = swapped.clone();
    let swap: Racer = Box::new(move || {
        let name = at.get().expect("the linked name");
        let other = name.with_file_name("other");
        fs::write(&other, b"another file").expect("write another file");
        fs::rename(&other, name).expect("put it at the linked name");
    });
    let smb = Quirks {
        mount_type: Some(("cifs", "rw")),
        name_opens: NameOpens::OneAtATime,
        before: vec![(Op::OpenSource, "linked-b", 0, swap)],
        ..Quirks::default()
    };
    let mounts = mounts(smb, |_| Quirks::default())?;
    let linked = mounts.sources.join("linked-b");
    fs::write(mounts.sources.join("linked-a"), b"linked")?;
    fs::hard_link(mounts.sources.join("linked-a"), &linked)?;
    let _ = swapped.set(linked.clone());

    let error = copy(&linked, &mounts.target()).expect_err("a swapped name is not copied");

    assert_eq!(
        error.message(),
        rust_i18n::t!(
            "%{name} changed while it was being copied",
            name = "linked-b"
        )
    );
    assert!(!mounts.target().exists());
    Ok(())
}

const CLOCK_START: Duration = Duration::from_secs(1_800_000_000);

/// Opens of `b` fail with `EINVAL` for `refused_for` on the test clock, as
/// an SMB client's do for another name of a file this machine just wrote.
/// `b` is no such name, which a real SMB mount would refuse in real time.
fn refused_for_a_while(
    (fs_type, options): (&'static str, &'static str),
    refused_for: Duration,
    more: Quirks,
) -> Result<(ClockGuard, Mounts, PathBuf), Box<dyn Error>> {
    let clock = ClockGuard::set(SystemTime::UNIX_EPOCH + CLOCK_START);
    let mounts = mounts(
        Quirks {
            mount_type: Some((fs_type, options)),
            deferred_close: Some(("b", SystemTime::UNIX_EPOCH + CLOCK_START + refused_for)),
            ..more
        },
        |_| Quirks::default(),
    )?;
    let folder = mounts.sources.join("folder");
    fs::create_dir(&folder)?;
    fs::write(folder.join("b"), b"just written")?;
    Ok((clock, mounts, folder))
}

fn waited() -> Result<Duration, Box<dyn Error>> {
    Ok(super::super::copy_engine::clock_for_test()
        .duration_since(SystemTime::UNIX_EPOCH + CLOCK_START)?)
}

fn opens_of_b(mounts: &Mounts) -> usize {
    mounts
        .source_scope
        .calls()
        .iter()
        .filter(|call| call.op == Op::OpenSource && call.names == ["b"])
        .count()
}

#[test]
fn another_name_of_a_file_just_written_on_smb_is_copied_once_the_client_closes_it()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    type Copier = fn(&Mounts, &Path) -> Result<PathBuf, Box<dyn Error>>;
    let rows: [(&str, Copier); 3] = [
        ("selected", |mounts, folder| {
            copy(&folder.join("b"), &mounts.target())?;
            Ok(mounts.target())
        }),
        ("in a folder", |mounts, folder| {
            let target = mounts.destination.join("folder");
            copy(folder, &target)?;
            Ok(target.join("b"))
        }),
        ("among enough files for the bulk path", |mounts, folder| {
            let mut items = vec![folder.join("b")];
            for index in 0..super::super::MAX_LOCAL_IO_WORKERS {
                let file = mounts.sources.join(format!("file-{index}"));
                fs::write(&file, b"other")?;
                items.push(file);
            }
            run_paste(PasteRequest {
                id: OperationRequestId(97),
                destination: Location::local(&mounts.destination),
                items: items
                    .into_iter()
                    .map(|source| PasteItem {
                        source: Location::local(source),
                        conflict: TransferConflict::FailIfExists,
                        target_name: None,
                    })
                    .collect(),
                move_sources: false,
            });
            Ok(mounts.destination.join("b"))
        }),
    ];
    for (row, copier) in rows {
        let (_clock, mounts, folder) = refused_for_a_while(
            ("cifs", "rw"),
            Duration::from_millis(1200),
            Quirks::default(),
        )?;

        let copied = copier(&mounts, &folder)?;

        assert_eq!(fs::read(copied)?, b"just written", "{row}");
        assert!(opens_of_b(&mounts) > 1, "{row}");
    }
    Ok(())
}

#[test]
fn source_opens_are_retried_only_on_smb_and_only_for_its_close_timeout()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let second = Duration::from_secs(1);
    let exactly = |waited: Duration| waited..waited + Duration::from_nanos(1);
    let at_once = exactly(Duration::ZERO);
    // The mount, the error opens of `b` fail with, how long `EINVAL` lasts,
    // and how long a copy that fails waited.
    for (row, mount, errno, refused_for, failed_after) in [
        (
            "closetimeo=5",
            ("smb3", "rw,closetimeo=5"),
            Errno::INVAL,
            4 * second,
            None,
        ),
        (
            "past the default close timeout",
            ("cifs", "rw"),
            Errno::INVAL,
            60 * second,
            Some(exactly(Duration::from_millis(1500))),
        ),
        (
            "closetimeo=60",
            ("cifs", "rw,closetimeo=60"),
            Errno::INVAL,
            120 * second,
            Some(exactly(10 * second)),
        ),
        (
            "ext4",
            ("ext4", "rw"),
            Errno::INVAL,
            second,
            Some(at_once.clone()),
        ),
        (
            "NFS",
            ("nfs4", "rw,vers=4.2"),
            Errno::INVAL,
            second,
            Some(at_once.clone()),
        ),
        (
            "another error",
            ("cifs", "rw"),
            Errno::ACCESS,
            Duration::ZERO,
            Some(at_once.clone()),
        ),
    ] {
        let more = match errno {
            Errno::INVAL => Quirks::default(),
            errno => Quirks {
                fail: vec![(Op::OpenSource, "b", 0, errno)],
                ..Quirks::default()
            },
        };
        let (_clock, mounts, folder) = refused_for_a_while(mount, refused_for, more)?;

        let copied = copy(&folder.join("b"), &mounts.target());

        let Some(failed_after) = failed_after else {
            copied?;
            assert_eq!(fs::read(mounts.target())?, b"just written", "{row}");
            continue;
        };
        let error = copied.expect_err(row);
        assert_eq!(
            error.message(),
            rust_i18n::t!(
                "Could not copy %{name}: %{error}",
                name = "b",
                error = crate::services::io_error_detail(&io::Error::from(errno))
            ),
            "{row}"
        );
        assert!(failed_after.contains(&waited()?), "{row}: {:?}", waited()?);
        if failed_after == at_once {
            assert_eq!(opens_of_b(&mounts), 1, "{row}");
        }
        assert!(!mounts.target().exists(), "{row}");
    }
    Ok(())
}

#[test]
fn a_copy_cancelled_while_waiting_for_an_smb_close_stops_at_once() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let (_clock, mounts, folder) = refused_for_a_while(
        ("cifs", "rw"),
        Duration::from_secs(60),
        Quirks {
            before: vec![(Op::OpenSource, "b", 1, Box::new(move || cancel.cancel()))],
            ..Quirks::default()
        },
    )?;

    let copied = glib::MainContext::default().block_on(copy_new_recursively(
        gio::File::for_path(folder.join("b")),
        gio::File::for_path(mounts.target()),
        cancellable,
    ));

    let error = copied.expect_err("the copy was cancelled");
    assert!(error.matches(gio::IOErrorEnum::Cancelled), "{error}");
    assert_eq!(opens_of_b(&mounts), 2);
    assert!(waited()? < Duration::from_millis(100), "{:?}", waited()?);
    assert!(!mounts.target().exists());
    Ok(())
}
