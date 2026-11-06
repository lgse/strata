// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_move_across_volumes_reads_the_source_mount_once() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir(&destination)?;
    let names = ["a.txt", "b.txt", "c.txt"];
    for name in names {
        fs::write(sources.join(name), name)?;
    }
    let (source_scope, _destination_scope) = separate_volumes(&sources, &destination);

    run_paste(PasteRequest {
        id: OperationRequestId(96),
        destination: Location::local(&destination),
        items: names
            .iter()
            .map(|name| PasteItem {
                source: Location::local(sources.join(name)),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            })
            .collect(),
        move_sources: true,
    });

    assert_eq!(super::super::names(&destination)?, names);
    assert!(super::super::names(&sources)?.is_empty());
    assert_eq!(source_scope.count(Op::MountType), 1);
    let smb = mount_capabilities(source_scope.key()).family == Some(Family::Smb);
    let reads_again = source_scope.count(Op::CurrentMountType);
    assert!(
        reads_again <= usize::from(smb) * names.len(),
        "{reads_again} reads of the current mount type, SMB: {smb}"
    );
    Ok(())
}

#[test]
fn a_source_entry_that_cannot_be_deleted_is_kept_with_the_reason() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let folder = source_dir.join("folder");
    fs::create_dir_all(folder.join("read-only"))?;
    fs::write(folder.join("a.txt"), b"a")?;
    let child = folder.join("read-only/child.txt");
    fs::write(&child, b"child")?;
    // As in a folder whose mode is 0555, for any user but root.
    let _source = FsQuirks::install(
        &source_dir,
        Quirks {
            fail: vec![(Op::Unlink, "child.txt", 0, Errno::ACCESS)],
            ..Quirks::default()
        },
    );
    let destination = root.path().join("destination");
    fs::create_dir_all(&destination)?;
    let _destination = FsQuirks::install(&destination, Quirks::default());

    let (_operation, events) =
        paste_move(96, &destination, &folder, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::Pasted { .. })
    ));
    assert_eq!(
        fs::read(destination.join("folder/read-only/child.txt"))?,
        b"child"
    );
    assert_eq!(names(&folder)?, ["read-only"]);
    assert_eq!(fs::read(&child)?, b"child");
    let reason = Errno::ACCESS.raw_os_error();
    assert_eq!(
        reported(&events).kept,
        [kept(child, KeptCause::Undeletable(reason))]
    );
    Ok(())
}
