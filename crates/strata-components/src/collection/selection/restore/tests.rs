use super::*;

fn position(selected: u64) -> Position {
    Position {
        selected: (0..selected).map(EntryId::new).collect(),
        ..Position::default()
    }
}

#[test]
fn restore_memory_evicts_the_least_recently_left() {
    // browser/view-modes/position-restore: In Icons and List, returning to one of the last 128 folders left restores its selection, cursor, range anchor, and scroll position after loading.
    let mut memory = RestoreMemory::new();
    for folder in 0..130 {
        assert!(memory.remember(folder, position(1)));
    }
    assert_eq!(memory.len(), 128);
    assert!(memory.recall(&0).is_none() && memory.recall(&1).is_none());
    assert!(memory.recall(&2).is_some() && memory.recall(&129).is_some());

    // Leaving folder 2 again makes it the most recent; recalling 3 doesn't.
    assert!(memory.recall(&3).is_some());
    memory.remember(2, position(1));
    memory.remember(130, position(1));
    assert!(memory.recall(&2).is_some());
    assert!(memory.recall(&3).is_none());
}

#[test]
fn restore_memory_keeps_within_its_id_budget() {
    let mut memory = RestoreMemory::with_limits(10, 100);
    memory.remember("a", position(40));
    memory.remember("b", position(40));
    memory.remember("a", position(10)); // replaces a's 40
    assert_eq!(memory.ids(), 50);
    // 60 more ids leave too little room: b, left longest ago, goes.
    memory.remember("c", position(60));
    assert_eq!(memory.ids(), 70);
    assert!(memory.recall(&"b").is_none());
    assert!(memory.recall(&"a").is_some() && memory.recall(&"c").is_some());
    // A selection bigger than the whole budget isn't kept, and costs nothing.
    assert!(!memory.remember("d", position(101)));
    assert!(memory.recall(&"d").is_none());
    assert_eq!(memory.ids(), 70);
    assert_eq!(memory.forget(&"c").map(|p| p.selected.len()), Some(60));
    assert_eq!(memory.ids(), 10);
}
