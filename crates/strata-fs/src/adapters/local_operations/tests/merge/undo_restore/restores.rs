// SPDX-License-Identifier: MIT

use crate::services::{RenameCause, RenamedItem};

use super::*;

#[test]
fn merge_undo_restores_staged_originals_and_removes_created() {
    crate::test_support::isolated_test(
        "adapters::local_operations::tests::merge::undo_restore::restores::merge_undo_restores_staged_originals_and_removes_created",
        || check_merge_undo_restores_staged_originals_and_removes_created().expect("merge undo"),
    );
}

fn pin(path: &Path) -> String {
    format!("{} Pin\n", gio::File::for_path(path).uri())
}

/// Pins each of `paths` in the bookmarks file, returning its path.
fn pin_all(paths: &[&Path]) -> io::Result<PathBuf> {
    let bookmarks = crate::adapters::bookmarks::pinned_places_path();
    fs::create_dir_all(bookmarks.parent().expect("bookmarks directory"))?;
    fs::write(
        &bookmarks,
        paths.iter().map(|path| pin(path)).collect::<String>(),
    )?;
    Ok(bookmarks)
}

fn check_merge_undo_restores_staged_originals_and_removes_created() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::create_dir_all(&target)?;
    let shared = target.join("shared.txt");
    let incoming = target.join("incoming.txt");
    let stays = target.join("stays.txt");
    fs::write(&shared, b"incoming wins")?;
    fs::write(&incoming, b"new")?;
    fs::write(&stays, b"keep me")?;
    let bookmarks = pin_all(&[&shared, &incoming, &stays])?;
    let incoming_identity = numbers_at(&incoming);
    let staged = staged_trash_fixture(root.path(), "shared.txt", &shared, b"old")?;
    let staged_path = staged
        .source
        .native_path()
        .expect("staged path")
        .to_path_buf();
    let lookup = staged_lookup(HashMap::from([(shared.clone(), staged)]));

    let placed = placed_now(&[&shared]);
    let parts = (
        vec![Location::local(&incoming)],
        vec![Location::local(&shared)],
        placed,
    );
    let events = undo_merge(90, parts, gio::Cancellable::new(), lookup);

    assert!(
        matches!(events.last(), Some(OperationEvent::Restored { .. })),
        "{events:?}"
    );
    assert_eq!(
        fs::read(&shared)?,
        b"old",
        "the staged original is restored"
    );
    assert!(!incoming.exists(), "the merge-created file is removed");
    let trashed = trashed_items(&events);
    assert_eq!(trashed, [(Location::local(&incoming), incoming_identity)]);
    assert!(!staged_path.exists(), "the staged copy left the fake trash");
    assert_eq!(fs::read(&stays)?, b"keep me");
    assert!(target.exists(), "the destination folder itself survives");
    crate::storage::writer::settle();
    assert_eq!(
        fs::read_to_string(bookmarks)?,
        format!("{}{}", pin(&shared), pin(&stays))
    );
    Ok(())
}

fn merged_created(events: &[OperationEvent]) -> Vec<PathBuf> {
    let mut created = events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::Merged { created, .. } => Some(created.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    created.sort();
    created
}

fn reported_renames(events: &[OperationEvent]) -> Vec<RenamedItem> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemsReported { report, .. } => Some(report.renamed.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn merged_placed(events: &[OperationEvent]) -> HashMap<Location, PlacedSnapshot> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::Merged { placed, .. } => Some(placed.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn undoing_a_merge_with_numbered_aliases_removes_only_its_items() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    let folder = destination.join("folder");
    fs::create_dir_all(sources.join("folder"))?;
    fs::create_dir_all(&folder)?;
    for name in ["Makefile", "makefile"] {
        fs::write(sources.join("folder").join(name), name)?;
    }
    fs::write(folder.join("keep.txt"), b"keep me")?;
    // Rules that do not fold case learn that a name is taken only when it is refused.
    let _scope = FsQuirks::install(
        &destination,
        without_trash(Quirks {
            mount_type: Some(("fuse.ciopfs", "rw")),
            case_insensitive: true,
            ..Quirks::default()
        }),
    );

    let events = run_paste(PasteRequest {
        id: OperationRequestId(94),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(sources.join("folder")),
            conflict: TransferConflict::Merge,
            target_name: None,
        }],
        move_sources: false,
    });

    assert_eq!(
        merged_created(&events),
        [folder.join("Makefile"), folder.join("makefile")]
    );
    let saved_as = Location::local(folder.join("makefile (1)"));
    assert_eq!(
        reported_renames(&events),
        [RenamedItem {
            requested: Location::local(folder.join("makefile")),
            saved_as: saved_as.clone(),
            cause: RenameCause::Alias,
        }]
    );
    let placed = merged_placed(&events);
    assert!(
        placed.contains_key(&saved_as),
        "an alias is recorded as saved"
    );
    let parts = (
        vec![Location::local(folder.join("Makefile")), saved_as],
        Vec::new(),
        placed,
    );
    let undone = undo_merge(
        95,
        parts,
        gio::Cancellable::new(),
        staged_lookup(HashMap::new()),
    );

    assert!(
        matches!(undone.last(), Some(OperationEvent::Restored { .. })),
        "{undone:?}"
    );
    assert_eq!(names(&folder)?, ["keep.txt"]);
    Ok(())
}

#[test]
fn merge_undo_keeps_the_incoming_copy_when_no_staged_original_exists() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::create_dir_all(&target)?;
    let shared = target.join("shared.txt");
    fs::write(&shared, b"incoming wins")?;

    let placed = placed_now(&[&shared]);
    let events = undo_merge(
        91,
        (Vec::new(), vec![Location::local(&shared)], placed),
        gio::Cancellable::new(),
        staged_lookup(HashMap::new()),
    );

    assert!(matches!(
        events.last(),
        Some(OperationEvent::CompletedWithErrors { .. })
    ));
    assert_eq!(
        fs::read(&shared)?,
        b"incoming wins",
        "the lookup runs before the incoming copy is deleted"
    );
    Ok(())
}

#[test]
fn merge_undo_tolerates_created_paths_that_no_longer_exist() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::create_dir_all(&target)?;
    let gone = target.join("already-removed.txt");

    let events = undo_merge(
        92,
        (vec![Location::local(&gone)], Vec::new(), HashMap::new()),
        gio::Cancellable::new(),
        staged_lookup(HashMap::new()),
    );

    assert!(matches!(
        events.last(),
        Some(OperationEvent::Restored { locations, .. }) if locations == &[Location::local(&gone)]
    ));
    Ok(())
}
