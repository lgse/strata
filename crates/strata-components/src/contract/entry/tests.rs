use super::*;

fn entry(id: u64, name: &str, kind: EntryKind) -> ListEntry {
    ListEntry::new(EntryId::new(id), name, kind, "")
}

fn names(source: &ListSource) -> Vec<&str> {
    (0..source.len()).map(|i| source.entry(i).name).collect()
}

fn source() -> ListSource {
    ListSource::new(vec![
        entry(10, "charlie", EntryKind::File),
        entry(11, "alpha", EntryKind::Folder),
        entry(12, "bravo", EntryKind::File),
    ])
}

#[test]
fn ids_follow_their_entries_across_a_re_sort() {
    let mut s = source();
    s.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(names(&s), ["alpha", "bravo", "charlie"]);
    for i in 0..s.len() {
        let id = s.id_at(i);
        assert_eq!(s.index_of(id), Some(i));
        assert_eq!(s.entry(i).id, id);
    }
    assert_eq!(
        s.index_of(EntryId::new(10)),
        Some(2),
        "charlie moved to the end"
    );
}

#[test]
fn a_removed_id_is_gone_and_the_rest_reindex() {
    let mut s = source();
    s.retain(|e| e.name != "alpha");
    assert_eq!(s.index_of(EntryId::new(11)), None);
    assert_eq!(s.index_of(EntryId::new(12)), Some(1));
    assert_eq!(s.len(), 2);
}

#[test]
fn a_replacement_indexes_only_the_new_entries() {
    let mut s = source();
    s.replace(vec![entry(20, "delta", EntryKind::File)]);
    assert_eq!(s.index_of(EntryId::new(10)), None);
    assert_eq!(s.index_of(EntryId::new(20)), Some(0));
    s.replace(Vec::new());
    assert!(s.is_empty());
}

#[test]
fn only_folders_and_links_to_folders_are_folder_like() {
    assert!(EntryKind::Folder.is_folder_like());
    assert!(EntryKind::Symlink(LinkTarget::Folder).is_folder_like());
    for kind in [
        EntryKind::File,
        EntryKind::Special,
        EntryKind::Symlink(LinkTarget::File),
        EntryKind::Symlink(LinkTarget::Broken),
    ] {
        assert!(!kind.is_folder_like(), "{kind:?}");
    }
}
