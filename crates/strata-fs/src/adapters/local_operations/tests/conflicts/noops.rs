// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn transfers_into_the_same_location_or_a_descendant_are_noops() {
    let source = gio::File::for_path("/fixture/source");
    let parent = gio::File::for_path("/fixture");
    let descendant = gio::File::for_path("/fixture/source/nested");
    let unrelated = gio::File::for_path("/elsewhere");

    for (destination, noop) in [
        (&parent, true),
        (&source, true),
        (&descendant, true),
        (&unrelated, false),
    ] {
        assert_eq!(
            transfer_is_noop(&source, destination, &destination.child("source")),
            noop,
            "{}",
            destination.uri()
        );
    }
}

#[test]
fn transfer_noops_resolve_aliased_destinations_but_not_symlink_sources()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let docs = root.path().join("docs");
    fs::create_dir_all(docs.join("Backup"))?;
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&docs, &link)?;
    let docs = gio::File::for_path(&docs);
    let link = gio::File::for_path(&link);
    let aliased_backup = link.child("Backup");

    assert!(transfer_is_noop(
        &docs,
        &aliased_backup,
        &aliased_backup.child("docs")
    ));
    assert!(!transfer_is_noop(&link, &docs, &docs.child("link")));
    Ok(())
}

#[test]
fn cutting_in_the_same_folder_remains_a_noop() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let destination = scratch.path();
    let file = destination.join("document.txt");
    let directory = destination.join("folder");
    fs::write(&file, b"content")?;
    fs::create_dir_all(&directory)?;

    let events = run_paste(paste_request(
        14,
        destination,
        vec![
            item(&file, TransferConflict::FailIfExists),
            item(&directory, TransferConflict::FailIfExists),
        ],
        true,
    ));

    assert!(item_moves(&events).is_empty());
    assert!(file.exists());
    assert!(directory.is_dir());
    expect_present(&[
        (destination.join("document (1).txt"), false),
        (destination.join("folder (1)"), false),
    ]);
    Ok(())
}

/// The source folder still holds only its report and empty nested folder.
fn expect_source_untouched(source: &Path, nested: &Path) -> io::Result<()> {
    assert_eq!(fs::read_dir(source)?.count(), 2);
    assert_eq!(fs::read_dir(nested)?.count(), 0);
    assert_eq!(fs::read(source.join("report.txt"))?, b"original");
    Ok(())
}

#[test]
fn keeping_both_preserves_transfer_noops() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let source = scratch.path().join("source");
    let nested = source.join("nested");
    fs::create_dir_all(&nested)?;
    fs::write(source.join("report.txt"), b"original")?;

    for moving in [false, true] {
        for destination in [&source, &nested] {
            let created = run_paste_collecting_created(paste_request(
                77,
                destination,
                vec![item(&source, TransferConflict::KeepBoth)],
                moving,
            ))?;
            assert!(created.into_iter().flatten().next().is_none());
            expect_source_untouched(&source, &nested)?;
        }
    }
    Ok(())
}

/// Pastes `todo.txt` into the folder that holds it.
fn pasting_todo_onto_itself(
    id: u64,
    conflict: TransferConflict,
) -> Result<(Scratch, PathBuf), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let source = scratch.path().join("todo.txt");
    fs::write(&source, b"existing\n")?;
    run_paste(paste_request(
        id,
        scratch.path(),
        vec![item(&source, conflict)],
        false,
    ));
    Ok((scratch, source))
}

#[test]
fn pasting_onto_itself_with_replace_is_a_noop() -> Result<(), Box<dyn Error>> {
    let (scratch, source) = pasting_todo_onto_itself(32, TransferConflict::ReplaceExisting)?;
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"existing\n");
    assert!(!scratch.path().join("todo (1).txt").exists());
    Ok(())
}

#[test]
fn pasting_onto_itself_with_keep_both_creates_numbered_copy() -> Result<(), Box<dyn Error>> {
    let (scratch, source) = pasting_todo_onto_itself(33, TransferConflict::KeepBoth)?;
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"existing\n");
    assert_eq!(
        fs::read(scratch.path().join("todo (1).txt"))?,
        b"existing\n"
    );
    Ok(())
}
