// SPDX-License-Identifier: MIT

//! Restores that place the Trash item by renaming or linking it.

use super::*;

fn rejecting_rename_flags(errno: Errno) -> Quirks {
    Quirks {
        rename_flags: Support::Rejected(errno),
        ..Quirks::default()
    }
}

#[test]
fn restore_without_rename_flags_never_replaces_a_racing_destination() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // The name is taken just before the step that would take it: the
    // fallback rung, or the copy made where nothing can place the item,
    // whose barrier is opened before it writes.
    let rows: [(Preset, Option<(Op, &str)>); 4] = [
        (Quirks::without_rename_flags, None),
        (|| rejecting_rename_flags(Errno::NOSYS), None),
        (|| rejecting_rename_flags(Errno::OPNOTSUPP), None),
        (Quirks::without_placement, Some((Op::OpenDir, "."))),
    ];
    for (quirks, copying) in rows {
        for directory in [false, true] {
            let fixture = RestoreFixture::new()?;
            fixture.seed(directory)?;
            let (op, pattern) = copying.unwrap_or(if directory {
                (Op::Mkdir, "destination")
            } else {
                (Op::Link, "destination")
            });
            let racer = writes(&fixture.destination, b"concurrent user data");
            let _scope = fixture.install(Quirks {
                before: vec![(op, pattern, 0, racer)],
                ..quirks()
            });

            let message = fixture
                .restore()
                .expect_err("the destination was taken")
                .to_string();

            assert!(
                message.contains("something already exists at the destination"),
                "{message}"
            );
            expect_contents(&[
                (fixture.destination.clone(), b"concurrent user data"),
                (fixture.seeded(directory), b"original"),
            ]);
        }
    }
    Ok(())
}

#[test]
fn restoring_without_rename_flags_moves_the_item_without_copying() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let presets: [Preset; 5] = [
        Quirks::without_rename_flags,
        || rejecting_rename_flags(Errno::NOSYS),
        || rejecting_rename_flags(Errno::OPNOTSUPP),
        Quirks::sshfs,
        Quirks::vboxsf,
    ];
    for quirks in presets {
        for directory in [false, true] {
            let fixture = RestoreFixture::new()?;
            let _scope = fixture.install(quirks());
            fixture.seed(directory)?;
            let inode = fs::metadata(&fixture.source)?.ino();

            fixture.restore()?;

            assert!(
                !fixture.source.exists(),
                "source should be removed from trash"
            );
            assert_eq!(fs::metadata(&fixture.destination)?.ino(), inode);
            expect_contents(&[(fixture.restored(directory), b"original")]);
        }
    }
    Ok(())
}

/// Moves both parents aside and redirects their names outside the allowed root.
fn redirects_parents(parents: [&Path; 2], retained: [&Path; 2], outside: &Path) -> Racer {
    let [moved_source, moved_target] = parents.map(Path::to_path_buf);
    let [old_source, old_target] = retained.map(Path::to_path_buf);
    let escaped = outside.to_path_buf();
    Box::new(move || {
        fs::rename(&moved_source, &old_source).expect("move source parent");
        fs::rename(&moved_target, &old_target).expect("move destination parent");
        symlink(&escaped, &moved_source).expect("redirect source parent");
        symlink(&escaped, &moved_target).expect("redirect destination parent");
    })
}

#[test]
fn restore_without_rename_flags_keeps_both_parents_pinned() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let allowed = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let source_parent = allowed.path().join("trash");
    let target_parent = allowed.path().join("documents");
    fs::create_dir(&source_parent)?;
    fs::create_dir(&target_parent)?;
    let source = source_parent.join("report");
    fs::create_dir(&source)?;
    fs::write(source.join("contents"), b"original")?;
    fs::create_dir(outside.path().join("report"))?;
    fs::write(outside.path().join("report/contents"), b"unrelated")?;
    let retained_source = allowed.path().join("old-trash");
    let retained_target = allowed.path().join("old-documents");
    let racer = redirects_parents(
        [&source_parent, &target_parent],
        [&retained_source, &retained_target],
        outside.path(),
    );
    let _scope = FsQuirks::install(
        allowed.path(),
        Quirks {
            before: vec![(Op::Rename, "report", 0, racer)],
            ..Quirks::without_rename_flags()
        },
    );

    restore_path(&source, &target_parent.join("report"), allowed.path())?;

    expect_contents(&[
        (retained_target.join("report/contents"), b"original"),
        (outside.path().join("report/contents"), b"unrelated"),
    ]);
    expect_gone(&[&retained_source.join("report")]);
    Ok(())
}

#[test]
fn restore_without_rename_flags_preserves_a_replaced_source() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = RestoreFixture::new()?;
    let original = fixture.path().join("original");
    fs::write(&fixture.source, b"original")?;
    let racer = replaces_source(&fixture.source, &original);
    let _scope = fixture.install(Quirks {
        before: vec![(Op::Rename, "source", 0, racer)],
        ..Quirks::without_rename_flags()
    });

    assert!(fixture.restore().is_err());

    expect_contents(&[
        (fixture.source.clone(), b"replacement"),
        (original, b"original"),
    ]);
    expect_gone(&[&fixture.destination]);
    Ok(())
}

#[test]
fn restore_refuses_a_destination_created_immediately_before_rename() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = RestoreFixture::new()?;
    fs::write(&fixture.source, b"original")?;
    let racer = writes(&fixture.destination, b"concurrent user data");
    let _scope = fixture.install(Quirks {
        before: vec![(Op::Rename, "destination", 0, racer)],
        ..Quirks::default()
    });

    let result = fixture.restore();

    assert!(
        result
            .expect_err("collision")
            .message()
            .contains("already exists")
    );
    expect_contents(&[
        (fixture.source.clone(), b"original"),
        (fixture.destination.clone(), b"concurrent user data"),
    ]);
    Ok(())
}

#[test]
fn restore_execution_refuses_a_parent_symlink_escape() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let allowed = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let source = allowed.path().join("source");
    fs::write(&source, b"original")?;
    symlink(outside.path(), allowed.path().join("parent"))?;

    let result = restore_path(
        &source,
        &allowed.path().join("parent/report"),
        allowed.path(),
    );

    assert!(result.is_err());
    expect_contents(&[(source, b"original")]);
    expect_gone(&[&outside.path().join("report")]);
    Ok(())
}
