// SPDX-License-Identifier: MIT

//! Restores that copy the Trash item where nothing can place it.

use super::*;

fn fifo_mode(path: &Path) -> io::Result<Option<u32>> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata
        .file_type()
        .is_fifo()
        .then_some(metadata.mode() & 0o777))
}

fn cancels(cancellable: &gio::Cancellable) -> Racer {
    let cancel = cancellable.clone();
    Box::new(move || cancel.cancel())
}

#[test]
fn restore_copies_files_links_and_named_pipes_where_nothing_can_place_them()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = RestoreFixture::new()?;
    fixture.seed(true)?;
    symlink("missing", fixture.source.join("link"))?;
    rustix::fs::mknodat(
        rustix::fs::CWD,
        fixture.source.join("fifo"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )?;
    fs::set_permissions(
        fixture.source.join("fifo"),
        fs::Permissions::from_mode(0o640),
    )?;
    let scope = fixture.install(Quirks::without_placement());

    let report = fixture.restore()?;

    assert!(!scope.used_name(".strata-copy-*"), "nothing is staged");
    assert_eq!(report, TransferReport::default());
    expect_contents(&[(fixture.restored(true), b"original")]);
    assert_eq!(
        fs::read_link(fixture.destination.join("link"))?,
        Path::new("missing")
    );
    assert_eq!(fifo_mode(&fixture.destination.join("fifo"))?, Some(0o640));
    expect_gone(&[&fixture.source]);
    Ok(())
}

#[test]
fn restore_copy_refuses_a_trash_item_replaced_after_it_was_checked() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // Replaced before the copy starts, and between the copy's check of the
    // name and its open of the file.
    for (op, pattern) in [(Op::OpenDir, "."), (Op::OpenSource, "source")] {
        let fixture = RestoreFixture::new()?;
        let original = fixture.path().join("original");
        fs::write(&fixture.source, b"original")?;
        let racer = replaces_source(&fixture.source, &original);
        let _scope = fixture.install(Quirks {
            before: vec![(op, pattern, 0, racer)],
            ..Quirks::without_placement().on_ext4()
        });

        let result = fixture.restore();

        assert!(result.is_err(), "{op:?}");
        expect_gone(&[&fixture.destination]);
        expect_contents(&[
            (fixture.source.clone(), b"replacement"),
            (original, b"original"),
        ]);
    }
    Ok(())
}

/// Adds a foreign entry to the copy and evicts or replaces a copied entry.
fn tampers_with_the_copy(copy: &Path, aside: &Path, evicted: bool) -> Racer {
    let (copy, aside) = (copy.to_path_buf(), aside.to_path_buf());
    Box::new(move || {
        fs::write(copy.join("foreign"), b"unrelated").expect("add a foreign entry");
        if evicted {
            assert!(
                super::super::fs_quirks::evict(&copy.join("contents")),
                "evict"
            );
            return;
        }
        fs::rename(copy.join("contents"), &aside).expect("move a copied entry aside");
        fs::write(copy.join("contents"), b"replacement").expect("replace a copied entry");
    })
}

fn exfat_in_one_tick() -> Quirks {
    Quirks {
        coarse_clock: Some(SystemTime::now() - Duration::from_secs(1)),
        links: Support::Rejected(Errno::MLINK),
        rename_flags: Support::Rejected(Errno::INVAL),
        rename_over_empty_dir: OverEmptyDirectory::NotEmpty,
        ..Quirks::exfat()
    }
}

#[test]
fn a_failed_restore_copy_removes_only_what_it_made() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let rows: [(Preset, bool); 3] = [
        (Quirks::without_placement, false),
        (exfat_in_one_tick, false),
        (exfat_in_one_tick, true),
    ];
    for (preset, evicted) in rows {
        let row = format!("{:?}, evicted: {evicted}", preset().mount_type);
        let fixture = RestoreFixture::new()?;
        fs::create_dir_all(fixture.source.join("sub"))?;
        fs::write(fixture.source.join("contents"), b"original")?;
        fs::write(fixture.source.join("sub/nested"), b"nested")?;
        let racer = tampers_with_the_copy(
            &fixture.destination,
            &fixture.path().join("old-copy"),
            evicted,
        );
        let _scope = fixture.install(Quirks {
            before: vec![(Op::SetTimes, "destination", 0, racer)],
            fail: vec![(Op::SetTimes, "destination", 0, Errno::IO)],
            ..preset()
        });

        assert!(fixture.restore().is_err(), "{row}");

        expect_contents(&[
            (fixture.destination.join("foreign"), b"unrelated"),
            (fixture.source.join("contents"), b"original"),
            (fixture.source.join("sub/nested"), b"nested"),
        ]);
        assert_eq!(
            fs::read(fixture.destination.join("contents")).ok(),
            (!evicted).then(|| b"replacement".to_vec()),
            "{row}"
        );
        assert!(!fixture.destination.join("sub").exists(), "{row}");
    }
    Ok(())
}

#[test]
fn a_failed_restore_copy_of_a_file_leaves_nothing_at_its_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = RestoreFixture::new()?;
    fs::write(&fixture.source, b"original")?;
    let _scope = fixture.install(Quirks {
        fail: vec![(Op::SetTimes, "destination", 0, Errno::IO)],
        ..Quirks::without_placement()
    });

    assert!(fixture.restore().is_err());

    expect_no_entry(&[&fixture.destination]);
    expect_contents(&[(fixture.source.clone(), b"original")]);
    Ok(())
}

/// How a cut-off folder copy's cleanup goes, and whether it removes the folder.
struct CutOffCleanup {
    preset: Preset,
    fail: Option<(Op, usize)>,
    renumber: bool,
    removed: bool,
}

impl CutOffCleanup {
    fn row(&self) -> String {
        format!(
            "{:?}, {:?}, renumbered: {}",
            (self.preset)().mount_type,
            self.fail,
            self.renumber
        )
    }

    fn quirks(&self, destination: &Path, cancellable: &gio::Cancellable) -> Quirks {
        let path = destination.to_path_buf();
        let renumbering: Racer = Box::new(move || super::super::fs_quirks::renumbered(&path));
        Quirks {
            before: vec![
                (Op::CopyRange, "*", 0, cancels(cancellable)),
                (Op::Write, "*", 0, cancels(cancellable)),
            ],
            after: if self.renumber {
                vec![(Op::OpenDir, "destination", 2, renumbering)]
            } else {
                Vec::new()
            },
            fail: self
                .fail
                .map(|(op, skip)| (op, "destination", skip, Errno::IO))
                .into_iter()
                .collect(),
            reflink: Support::Rejected(Errno::OPNOTSUPP),
            ..(self.preset)()
        }
    }
}

#[test]
fn a_cancelled_restore_copy_of_a_folder_is_a_cancel_only_once_the_folder_is_gone()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // Cleanup opens the cut-off folder (its third open, after making and
    // filling it), checks it, empties it and checks its name before removing
    // it. Rows fail the open or a check, or renumber the folder as libfuse
    // without `use_ino` does, where only its type is compared.
    let host: Preset = Quirks::without_placement;
    let ext4 = || Quirks::without_placement().on_ext4();
    let sshfs = || Quirks {
        mount_type: Some(("fuse.sshfs", "rw")),
        ..Quirks::without_placement()
    };
    let rows = [
        (host, Some((Op::OpenDir, 2)), false, false),
        (host, Some((Op::StatFd, 2)), false, false),
        (ext4, None, true, false),
        (host, Some((Op::Stat, 2)), false, false),
        (sshfs, None, true, true),
    ]
    .map(|(preset, fail, renumber, removed)| CutOffCleanup {
        preset,
        fail,
        renumber,
        removed,
    });
    for cleanup in rows {
        let row = cleanup.row();
        let fixture = RestoreFixture::new()?;
        fs::create_dir(&fixture.source)?;
        let contents = vec![0x5a; 4 * 1024 * 1024];
        fs::write(fixture.source.join("data"), &contents)?;
        let cancellable = gio::Cancellable::new();
        let _scope = fixture.install(cleanup.quirks(&fixture.destination, &cancellable));

        let restored = fixture.restore_cancellable(cancellable);

        let error = restored.expect_err("a cut-off copy");
        assert_eq!(
            error.matches(gio::IOErrorEnum::Cancelled),
            cleanup.removed,
            "{row}: {error}"
        );
        assert_eq!(
            fs::symlink_metadata(&fixture.destination).is_err(),
            cleanup.removed,
            "{row}"
        );
        assert_eq!(fs::read(fixture.source.join("data"))?, contents, "{row}");
    }
    Ok(())
}

#[test]
fn a_restore_cancelled_once_its_copy_is_complete_finishes() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = RestoreFixture::new()?;
    fixture.seed(true)?;
    let cancellable = gio::Cancellable::new();
    let _scope = fixture.install(Quirks {
        mount_type: Some(("ext4", "rw")),
        before: vec![(Op::SyncFs, "*", 0, cancels(&cancellable))],
        ..Quirks::without_placement()
    });

    fixture.restore_cancellable(cancellable)?;

    expect_contents(&[(fixture.restored(true), b"original")]);
    expect_no_entry(&[&fixture.source]);
    Ok(())
}

#[test]
fn restore_copies_keep_the_metadata_the_destination_takes() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let refusing = || Quirks {
        extra_xattrs: vec![("security.strata-test", b"label")],
        xattrs_refused: vec![("security.", Errno::PERM)],
        ..Quirks::without_placement()
    };
    // Read-only items keep the xattrs set before their mode, and an xattr
    // the destination refuses is left out.
    let rows: [(bool, u32, Preset); 3] = [
        (false, 0o444, Quirks::without_placement),
        (true, 0o555, Quirks::without_placement),
        (false, 0o640, refusing),
    ];
    for (directory, mode, quirks) in rows {
        let row = format!("directory: {directory}, mode: {mode:o}");
        let fixture = RestoreFixture::new()?;
        if directory {
            fs::create_dir(&fixture.source)?;
        } else {
            fs::write(&fixture.source, b"original")?;
        }
        let opened = fs::File::open(&fixture.source)?;
        set_user_xattr(&opened)?;
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        opened.set_times(fs::FileTimes::new().set_modified(modified))?;
        fs::set_permissions(&fixture.source, fs::Permissions::from_mode(mode))?;
        let _scope = fixture.install(quirks());

        fixture.restore()?;

        let restored = fs::symlink_metadata(&fixture.destination)?;
        assert_eq!(
            (restored.mode() & 0o777, restored.modified()?),
            (mode, modified),
            "{row}"
        );
        assert_eq!(user_xattr(&fixture.destination)?, b"metadata", "{row}");
        assert!(fs::symlink_metadata(&fixture.source).is_err(), "{row}");
    }
    Ok(())
}

#[test]
fn restore_deletes_the_trash_copy_only_once_its_copy_is_flushed() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // The family, how its barrier is reached, whether each restored file is
    // synced before it is closed, and whether the flush fails.
    let rows = [
        ("ext4", Op::SyncFs, false, false),
        ("ext4", Op::SyncFs, false, true),
        ("fuseblk", Op::SyncDir, true, false),
    ];
    for (fs_type, flush, each_file, failing) in rows {
        let row = format!("{fs_type}, failing: {failing}");
        let fixture = RestoreFixture::new()?;
        fixture.seed(true)?;
        let fail = if failing {
            vec![(flush, "*", 0, Errno::IO)]
        } else {
            Vec::new()
        };
        let scope = fixture.install(Quirks {
            mount_type: Some((fs_type, "rw")),
            fail,
            ..Quirks::without_placement()
        });

        let restored = fixture.restore();

        let calls = scope.calls();
        let first = |op: Op| {
            calls
                .iter()
                .position(|call| call.op == op && call.names.iter().any(|n| n == "contents"))
        };
        let flushed = calls.iter().position(|call| call.op == flush);
        assert!(flushed.is_some(), "{row}");
        let synced = first(Op::SyncData);
        assert_eq!(synced.is_some(), each_file, "{row}");
        assert!(synced < first(Op::Close), "{row}");
        expect_contents(&[(fixture.restored(true), b"original")]);
        let deleted = first(Op::Unlink);
        if failing {
            let message = restored.expect_err("the flush failed").to_string();
            assert!(message.contains("could not finish writing"), "{message}");
            assert_eq!(deleted, None, "{row}");
            expect_contents(&[(fixture.seeded(true), b"original")]);
        } else {
            restored?;
            assert!(deleted > flushed, "{row}");
            assert!(!fixture.source.exists(), "{row}");
        }
    }
    Ok(())
}

#[test]
fn restore_directory_copy_preserves_permissions_times_and_xattrs() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = RestoreFixture::new()?;
    fixture.seed(true)?;
    fs::set_permissions(&fixture.source, fs::Permissions::from_mode(0o750))?;
    let source_file = fs::File::open(&fixture.source)?;
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let accessed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    source_file.set_times(
        fs::FileTimes::new()
            .set_accessed(accessed)
            .set_modified(modified),
    )?;
    set_user_xattr(&source_file)?;
    let source_inode = fs::metadata(&fixture.source)?.ino();
    let _scope = fixture.install(Quirks::without_placement());

    fixture.restore()?;

    expect_gone(&[&fixture.source]);
    let restored = fs::metadata(&fixture.destination)?;
    assert_ne!(restored.ino(), source_inode);
    assert_eq!(
        (
            restored.mode() & 0o777,
            restored.modified()?,
            restored.accessed()?
        ),
        (0o750, modified, accessed)
    );
    assert_eq!(user_xattr(&fixture.destination)?, b"metadata");
    Ok(())
}
