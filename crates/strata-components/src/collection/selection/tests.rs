//! One test per feature-map Behavior bullet this layer decides. Each names its bullet. Ids are
//! the entries' first indices, so `[2, 3]` reads as "the third and fourth entries as first
//! listed".

use super::*;
use crate::contract::{EntryKind, ListEntry, ListSource};

mod perf;
mod property;

fn entry(raw: u64) -> ListEntry {
    ListEntry::new(EntryId::new(raw), format!("e{raw}"), EntryKind::File, "")
}

/// Entries with ids `0..n`, in order.
fn listing(n: u64) -> ListSource {
    ListSource::new((0..n).map(entry).collect())
}

fn id(raw: u64) -> EntryId {
    EntryId::new(raw)
}

/// The selected ids, in display order.
fn selected(sel: &Selection, s: &ListSource) -> Vec<u64> {
    sel.selected_ids(s).into_iter().map(EntryId::get).collect()
}

fn cursor(sel: &Selection) -> Option<u64> {
    sel.cursor().map(EntryId::get)
}

fn anchor(sel: &Selection) -> Option<u64> {
    sel.anchor().map(EntryId::get)
}

/// Removes the entries with the given ids, as a reload that drops them does.
fn remove(s: &mut ListSource, gone: &[u64]) {
    s.retain(|e| !gone.contains(&e.id.get()));
}

/// Deletes the entries with the given ids the way a host does: tells the selection first,
/// removes them, then syncs with `rule`.
fn delete(sel: &mut Selection, s: &mut ListSource, gone: &[u64], rule: CursorGone) -> Changed {
    sel.before_removal(s, |id| gone.contains(&id.get()));
    remove(s, gone);
    sel.sync(s, SyncRule::complete(rule))
}

#[test]
fn select_only_replaces_the_selection() {
    // browser/selection: A plain click on an entry selects only that entry and replaces the previous selection.
    let s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.extend_to(&s, 4, Extend::Replace);
    let changed = sel.select_only(&s, 2);
    assert_eq!(selected(&sel, &s), [2]);
    assert!(changed.selection && changed.cursor && changed.anchor);
    assert_eq!((cursor(&sel), anchor(&sel)), (Some(2), Some(2)));
}

#[test]
fn toggle_flips_one_entry_and_moves_the_anchor() {
    // browser/selection: Ctrl+click toggles one entry in or out of the selection and moves the range anchor to it.
    let s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.toggle(&s, 3);
    assert_eq!(selected(&sel, &s), [1, 3]);
    assert_eq!(anchor(&sel), Some(3));
    sel.toggle(&s, 1);
    assert_eq!(selected(&sel, &s), [3]);
    assert_eq!(anchor(&sel), Some(1));
    // The next Shift range starts at the toggled entry.
    sel.extend_to(&s, 4, Extend::Replace);
    assert_eq!(selected(&sel, &s), [1, 2, 3, 4]);
}

#[test]
fn extend_to_selects_from_the_anchor_to_the_target() {
    // browser/selection: Shift+click selects every entry from the range anchor to the clicked entry.
    let s = listing(8);
    let mut sel = Selection::new();
    sel.select_only(&s, 5);
    sel.toggle(&s, 0);
    sel.toggle(&s, 0);
    // Anchor on 0 now; Shift+click on 3 replaces the selection with 0..=3.
    sel.extend_to(&s, 3, Extend::Replace);
    assert_eq!(selected(&sel, &s), [0, 1, 2, 3]);
    assert_eq!(anchor(&sel), Some(0));
}

#[test]
fn extend_after_auto_select_ranges_from_the_loaded_entry() {
    // browser/selection: After a navigation auto-selects the first entry, the first Shift+click ranges from that entry
    let s = listing(6);
    let mut sel = Selection::new();
    sel.auto_select(&s, 0);
    sel.extend_to(&s, 3, Extend::Replace);
    assert_eq!(selected(&sel, &s), [0, 1, 2, 3]);
    assert!(!sel.is_provisional());
}

#[test]
fn select_only_re_anchors_extension() {
    // browser/selection: A click re-anchors Shift+Down and Shift+Up, so keyboard extension continues from the clicked entry.
    // browser/selection: In List and Icons, a plain arrow or `h`/`j`/`k`/`l` move that changes the selection moves the range anchor.
    let s = listing(8);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.extend_to(&s, 3, Extend::Replace);
    sel.select_only(&s, 5);
    sel.extend_to(&s, 6, Extend::Replace);
    assert_eq!(selected(&sel, &s), [5, 6]);
}

#[test]
fn toggle_and_extend_move_the_cursor() {
    // browser/selection: Ctrl+click or Shift+click on an entry's name or icon moves keyboard focus to that entry
    let s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 0);
    assert!(sel.toggle(&s, 2).cursor);
    assert_eq!(cursor(&sel), Some(2));
    assert!(sel.extend_to(&s, 4, Extend::Replace).cursor);
    assert_eq!(cursor(&sel), Some(4));
    assert_eq!(sel.cursor_index(&s), Some(4));
}

#[test]
fn extend_to_grows_and_shrinks_around_a_fixed_anchor() {
    // browser/selection: Shift+Up and Shift+Down extend the selection by one entry from the range anchor.
    // browser/selection: Shift+Page Up and Shift+Page Down extend or shrink the selection by one page from the range anchor.
    // browser/selection: In Columns, Shift+Home and Shift+End extend from the range anchor to the first or last entry
    let s = listing(10);
    let mut sel = Selection::new();
    sel.select_only(&s, 4);
    sel.extend_to(&s, 5, Extend::Replace);
    sel.extend_to(&s, 8, Extend::Replace);
    assert_eq!(selected(&sel, &s), [4, 5, 6, 7, 8]);
    // Back past the anchor: the range shrinks, then grows the other way.
    sel.extend_to(&s, 6, Extend::Replace);
    assert_eq!(selected(&sel, &s), [4, 5, 6]);
    sel.extend_to(&s, 2, Extend::Replace);
    assert_eq!(selected(&sel, &s), [2, 3, 4]);
    sel.extend_to(&s, 0, Extend::Replace);
    assert_eq!(selected(&sel, &s), [0, 1, 2, 3, 4]);
    assert_eq!(anchor(&sel), Some(4));
}

#[test]
fn extend_with_no_anchor_starts_at_the_target() {
    // browser/selection: Shift+Down with nothing selected selects only the first entry; the next Shift+Down selects two.
    let s = listing(5);
    let mut sel = Selection::new();
    sel.extend_to(&s, 0, Extend::Replace);
    assert_eq!(selected(&sel, &s), [0]);
    assert_eq!(anchor(&sel), Some(0));
    sel.extend_to(&s, 1, Extend::Replace);
    assert_eq!(selected(&sel, &s), [0, 1]);
}

#[test]
fn clear_re_anchors_at_the_cursor() {
    // browser/selection: After Escape clears a selection, the next Shift+arrow range starts at the keyboard cursor.
    let s = listing(8);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.extend_to(&s, 4, Extend::Replace);
    let changed = sel.clear();
    assert!(changed.anchor);
    assert_eq!(anchor(&sel), Some(4));
    sel.extend_to(&s, 5, Extend::Replace);
    assert_eq!(selected(&sel, &s), [4, 5]);
}

#[test]
fn select_only_then_extend_selects_through_the_anchor() {
    // browser/selection: In List, Icons, and Columns, plain Home, End, Page Up, and Page Down move the range anchor to the new cursor. Shift+Home after End selects every entry.
    // browser/selection: After Home, Shift+Down selects the first two entries.
    let s = listing(7);
    let mut sel = Selection::new();
    sel.select_only(&s, 2);
    sel.select_only(&s, 6); // End
    sel.extend_to(&s, 0, Extend::Replace); // Shift+Home
    assert_eq!(selected(&sel, &s), [0, 1, 2, 3, 4, 5, 6]);
    sel.select_only(&s, 0); // Home
    sel.extend_to(&s, 1, Extend::Replace); // Shift+Down
    assert_eq!(selected(&sel, &s), [0, 1]);
}

#[test]
fn select_all_selects_every_listed_entry() {
    // Ctrl+A selects every listed entry (which column is focused is C2's). Ruling 1: the
    // cursor and the anchor stay.
    let s = listing(5);
    let mut sel = Selection::new();
    sel.select_only(&s, 3);
    let changed = sel.select_all(&s);
    assert!(changed.selection);
    assert_eq!(selected(&sel, &s), [0, 1, 2, 3, 4]);
    assert_eq!(sel.len(), 5);
    // Decision 1: the cursor and the anchor stay put.
    assert_eq!((cursor(&sel), anchor(&sel)), (Some(3), Some(3)));
    assert!(!sel.select_all(&s).any(), "already all selected");

    let mut fresh = Selection::new();
    fresh.select_all(&s);
    assert_eq!((cursor(&fresh), anchor(&fresh)), (Some(0), Some(0)));
}

#[test]
fn select_all_leaves_entries_listed_later_unselected() {
    // browser/selection: With hidden files off, Ctrl+A in Icons, List, and the chooser leaves hidden entries unselected.
    // The host lists only visible entries, so Ctrl+A takes those. Showing hidden files later
    // lists more entries, which stay unselected, as does any entry a live update adds.
    let mut s = listing(4);
    let mut sel = Selection::new();
    sel.select_all(&s);
    let mut more: Vec<ListEntry> = s.entries().to_vec();
    more.insert(1, entry(90));
    more.push(entry(91));
    s.replace(more);
    let changed = sel.sync(&s, SyncRule::default());
    assert!(!changed.any());
    assert_eq!(selected(&sel, &s), [0, 1, 2, 3]);
    assert!(!sel.contains(&s, id(90)) && !sel.contains(&s, id(91)));
}

#[test]
fn move_cursor_keeps_the_selection() {
    // integration/10xer-mode: After Space, Ctrl+A, or Ctrl+R, Home, End, and paging move the cursor and keep the selection.
    let s = listing(8);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.toggle(&s, 3);
    let changed = sel.move_cursor(&s, 7, AnchorRule::Keep);
    assert!(changed.cursor && !changed.selection && !changed.anchor);
    assert_eq!(selected(&sel, &s), [1, 3]);
    assert_eq!(anchor(&sel), Some(3));
    sel.move_cursor(&s, 6, AnchorRule::Move);
    assert_eq!(anchor(&sel), Some(6));
}

#[test]
fn clear_empties_the_selection_and_keeps_the_cursor() {
    // browser/selection: A plain press on blank pane space clears the selection before the drag intent is known.
    // browser/selection: With no transient UI open, Escape clears the active pane's selection and keeps the keyboard cursor and directory.
    // browser/selection: Enter after Escape opens the folder under the keyboard cursor.
    let s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 2);
    sel.extend_to(&s, 4, Extend::Replace);
    let changed = sel.clear();
    assert!(changed.selection && !changed.cursor);
    assert!(sel.is_empty());
    assert_eq!(cursor(&sel), Some(4));
}

#[test]
fn clear_with_nothing_selected_reports_no_change() {
    // browser/selection: Outside 10xer mode, in Columns, Escape with nothing selected in the active column closes the deepest column.
    let s = listing(3);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    assert!(sel.clear().any());
    let revision = sel.revision();
    assert!(
        !sel.clear().any(),
        "nothing left to clear: the column may close"
    );
    assert_eq!(sel.revision(), revision);
}

#[test]
fn sync_drops_gone_ids_and_keeps_the_rest() {
    // browser/selection: F5 or the pane Refresh button keeps a multi-selection; entries gone from disk are dropped.
    // browser/view-modes/position-restore: In Icons and List, these reloads keep the cursor row, selection, and scroll offset
    let mut s = listing(8);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.extend_to(&s, 5, Extend::Replace);
    remove(&mut s, &[3]);
    let changed = sel.sync(&s, SyncRule::default());
    assert!(changed.selection && !changed.cursor);
    assert_eq!(selected(&sel, &s), [1, 2, 4, 5]);
    assert_eq!(sel.len(), 4);
    assert_eq!(cursor(&sel), Some(5));
}

#[test]
fn loading_sync_holds_ids_until_complete() {
    // browser/selection: F5 or the pane Refresh button keeps a multi-selection; entries gone from disk are dropped.
    // browser/view-modes/position-restore: In Icons and List, these reloads keep the cursor row, selection, and scroll offset
    let mut s = listing(8);
    let mut sel = Selection::new();
    sel.select_only(&s, 2);
    sel.toggle(&s, 6);
    // A reload empties the listing, then delivers it in batches; 2 is gone from disk.
    s.replace(Vec::new());
    assert!(!sel.sync(&s, SyncRule::loading()).any());
    s.replace([0, 1, 3].map(entry).to_vec());
    sel.sync(&s, SyncRule::loading());
    assert_eq!(sel.len(), 2, "still waiting for 2 and 6");
    assert_eq!(cursor(&sel), Some(6));
    s.replace([0, 1, 3, 4, 5, 6, 7].map(entry).to_vec());
    let changed = sel.sync(&s, SyncRule::default());
    assert!(changed.selection && !changed.cursor);
    assert_eq!(selected(&sel, &s), [6]);
    assert_eq!(cursor(&sel), Some(6));
}

#[test]
fn delete_moves_the_cursor_to_the_next_survivor_else_the_previous() {
    // browser/selection: Deleting the focused entry, in Strata or externally, focuses the next visible entry, else the previous visible one, and the preview follows.
    let mut s = listing(5);
    let mut sel = Selection::new();
    sel.select_only(&s, 2);
    delete(&mut sel, &mut s, &[2], CursorGone::NeighborSelected);
    assert_eq!((cursor(&sel), selected(&sel, &s)), (Some(3), vec![3]));
    // At the end there's no next entry: the previous takes it.
    sel.select_only(&s, 3);
    delete(&mut sel, &mut s, &[3, 4], CursorGone::NeighborSelected);
    assert_eq!((cursor(&sel), selected(&sel, &s)), (Some(1), vec![1]));

    // A Shift range deleted whole: the next survivor is the entry after it, not the one now
    // at the cursor's old index.
    let mut s = listing(12);
    let mut sel = Selection::new();
    sel.select_only(&s, 2);
    sel.extend_to(&s, 6, Extend::Replace);
    let changed = delete(
        &mut sel,
        &mut s,
        &[2, 3, 4, 5, 6],
        CursorGone::NeighborSelected,
    );
    assert!(changed.cursor && changed.selection);
    assert_eq!((cursor(&sel), selected(&sel, &s)), (Some(7), vec![7]));
}

#[test]
fn deleted_cursor_neighbour_does_not_join_the_fill() {
    // integration/10xer-mode/file-verbs: After `d` or `M`, the next item receives the cursor without joining the fill.
    let mut s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 0);
    sel.toggle(&s, 3);
    // `d` deletes the fill, 0 and 3; the next survivor after 3 is 4.
    delete(&mut sel, &mut s, &[0, 3], CursorGone::Neighbor);
    assert_eq!(cursor(&sel), Some(4));
    assert!(sel.is_empty());
    assert_eq!(sel.targets(&s), Targets::Cursor(id(4)));
}

#[test]
fn removing_every_entry_clears_cursor_and_selection() {
    // browser/selection: Deleting the last visible entry clears focus and selection.
    let mut s = listing(1);
    let mut sel = Selection::new();
    sel.select_only(&s, 0);
    let changed = delete(&mut sel, &mut s, &[0], CursorGone::NeighborSelected);
    assert!(changed.selection && changed.cursor && changed.anchor);
    assert!(sel.is_empty());
    assert_eq!((sel.cursor(), sel.anchor()), (None, None));
}

#[test]
fn inserts_leave_selection_and_cursor_unchanged() {
    // browser/selection: Background changes to an open directory keep the selection and scroll position and do not take focus.
    let mut s = listing(4);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.toggle(&s, 2);
    let mut more = s.entries().to_vec();
    more.insert(0, entry(50));
    more.insert(3, entry(51));
    s.replace(more);
    let revision = sel.revision();
    assert!(!sel.sync(&s, SyncRule::default()).any());
    assert_eq!(sel.revision(), revision);
    assert_eq!(selected(&sel, &s), [1, 2]);
    assert_eq!(cursor(&sel), Some(2));
}

#[test]
fn reload_moves_the_cursor_to_its_old_index_unselected() {
    // browser/selection: A reload can drop the cursor's entry, as after a delete on a remote share. The cursor then moves to the first visible entry at or after its old index, unselected.
    let mut s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 0);
    sel.toggle(&s, 3);
    // A reload comes back without 1 and 3: the entry now at index 3 (5) takes the cursor.
    remove(&mut s, &[1, 3]);
    let changed = sel.sync(&s, SyncRule::default());
    assert!(changed.cursor && changed.selection);
    assert_eq!(cursor(&sel), Some(5));
    assert_eq!(selected(&sel, &s), [0]);
    assert_eq!(anchor(&sel), Some(5), "the anchor's entry went too");
}

#[test]
fn cursor_past_the_end_moves_back() {
    // browser/selection: When no visible entry follows, the cursor moves to the previous visible entry instead.
    let mut s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 5);
    remove(&mut s, &[3, 4, 5]);
    sel.sync(&s, SyncRule::default());
    assert_eq!(cursor(&sel), Some(2));
    assert!(sel.is_empty());
}

#[test]
fn cursor_gone_clear_leaves_no_cursor() {
    // browser/selection: In Columns, when a reload drops the cursor's entry in a column whose child column is open, no neighbour takes the cursor.
    let mut s = listing(4);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    remove(&mut s, &[1]);
    let changed = sel.sync(&s, SyncRule::complete(CursorGone::Clear));
    assert!(changed.cursor);
    assert_eq!((sel.cursor(), sel.anchor()), (None, None));
    assert!(sel.is_empty());
}

#[test]
fn reorder_keeps_cursor_anchor_and_selection_by_id() {
    // integration/10xer-mode/file-verbs: A keyboard sort keeps the cursor on the same item and leaves other Miller columns in their order.
    // integration/10xer-mode/file-verbs: After `, n`, `, l`, or `, c`, the cursor stays on the same item.
    let mut s = listing(10);
    let mut sel = Selection::new();
    sel.select_only(&s, 2);
    sel.extend_to(&s, 5, Extend::Replace);
    s.sort_by(|a, b| b.id.cmp(&a.id));
    assert!(!sel.sync(&s, SyncRule::default()).any());
    assert_eq!(selected(&sel, &s), [5, 4, 3, 2]);
    assert_eq!((cursor(&sel), anchor(&sel)), (Some(5), Some(2)));
    // Extending again ranges over the new order, from the same anchor entry.
    sel.extend_to(
        &s,
        sel.cursor_index(&s).expect("listed") + 1,
        Extend::Replace,
    );
    assert_eq!(selected(&sel, &s), [4, 3, 2]);
}

#[test]
fn relocate_keeps_selection_and_cursor() {
    // integration/10xer-mode/file-verbs: `rename ›` on a focused filter result keeps it focused and selected under its new name
    let mut s = listing(4);
    let mut sel = Selection::new();
    sel.select_only(&s, 2);
    // The host renames entry 2, which gives it id 20.
    let renamed: Vec<ListEntry> = s
        .entries()
        .iter()
        .map(|e| if e.id == id(2) { entry(20) } else { e.clone() })
        .collect();
    s.replace(renamed);
    assert!(sel.relocate(id(2), id(20)).cursor);
    assert!(!sel.sync(&s, SyncRule::default()).any());
    assert_eq!(selected(&sel, &s), [20]);
    assert_eq!((cursor(&sel), anchor(&sel)), (Some(20), Some(20)));
}

#[test]
fn targets_are_the_fill_else_the_cursor() {
    // integration/10xer-mode/file-verbs: `y`, `x`, `d`, `c c`, and `c n` act on the focused pane's fill, or its cursor item when nothing is filled.
    // integration/10xer-mode/file-verbs: In an empty folder, `y`, `x`, `d`, and `r` flash `Nothing to yank`
    let s = listing(5);
    let mut sel = Selection::new();
    assert_eq!(sel.targets(&s), Targets::Nothing);
    sel.select_only(&s, 3);
    sel.toggle(&s, 1);
    assert_eq!(sel.targets(&s), Targets::Filled(vec![id(1), id(3)]));
    sel.clear();
    assert_eq!(sel.targets(&s), Targets::Cursor(id(1)));
    assert_eq!(Selection::new().targets(&listing(0)), Targets::Nothing);
}

#[test]
fn auto_selected_entry_is_not_a_fill() {
    // integration/10xer-mode/file-verbs: `y`, `x`, `d`, `c c`, and `c n` act on the focused pane's fill, or its cursor item when nothing is filled.
    let s = listing(3);
    let mut sel = Selection::new();
    sel.auto_select(&s, 0);
    assert!(sel.is_provisional());
    assert_eq!(selected(&sel, &s), [0], "it still shows as selected");
    assert_eq!(sel.targets(&s), Targets::Cursor(id(0)));
    // A cursor move carries no provisional pick along.
    let changed = sel.move_cursor(&s, 1, AnchorRule::Keep);
    assert!(changed.selection);
    assert!(sel.is_empty() && !sel.is_provisional());
}

#[test]
fn selected_ids_never_include_an_unselected_cursor() {
    // integration/10xer-mode/file-verbs: Default-map direct verbs act on the selection, never on the cursor item.
    let s = listing(4);
    let mut sel = Selection::new();
    sel.select_only(&s, 0);
    sel.move_cursor(&s, 2, AnchorRule::Keep);
    assert_eq!(selected(&sel, &s), [0]);
    sel.clear();
    assert!(sel.selected_ids(&s).is_empty());
}

#[test]
fn no_op_operations_leave_the_revision() {
    // What C1e relies on to announce only real changes. (The 10xer `; N` recheck compares
    // targets, which is the host's: a cursor move bumps the revision without changing them.)
    let s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    let r = sel.revision();
    assert!(!sel.select_only(&s, 1).any());
    assert!(!sel.move_cursor(&s, 1, AnchorRule::Move).any());
    assert!(!sel.cancel_band().any());
    assert_eq!(sel.revision(), r);
    sel.extend_to(&s, 3, Extend::Replace);
    let after_extend = sel.revision();
    assert_ne!(after_extend, r);
    assert!(!sel.extend_to(&s, 3, Extend::Replace).any());
    assert_eq!(sel.revision(), after_extend);
    // A fresh selection never repeats a revision an observer saw.
    assert!(Selection::new().revision() > after_extend);
}

#[test]
fn invert_flips_every_listed_entry() {
    // integration/10xer-mode: Ctrl+A selects every item in the focused pane, and Ctrl+R inverts that pane's selection instead of renaming.
    let s = listing(5);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.toggle(&s, 3);
    assert!(sel.invert(&s).selection);
    assert_eq!(selected(&sel, &s), [0, 2, 4]);
    assert_eq!(cursor(&sel), Some(3));
}

#[test]
fn remove_range_unselects_from_the_anchor() {
    // integration/10xer-mode: `v` starts a range from the cursor and `V` starts an unselect range
    let s = listing(8);
    let mut sel = Selection::new();
    sel.select_all(&s);
    sel.move_cursor(&s, 2, AnchorRule::Move); // `V` at entry 2
    sel.extend_to(&s, 4, Extend::Remove);
    assert_eq!(selected(&sel, &s), [0, 1, 5, 6, 7]);
    sel.extend_to(&s, 3, Extend::Remove); // walking back restores 4
    assert_eq!(selected(&sel, &s), [0, 1, 4, 5, 6, 7]);
    // `v` from 6 adds to what is selected now.
    sel.move_cursor(&s, 6, AnchorRule::Move);
    sel.extend_to(&s, 7, Extend::Add);
    sel.move_cursor(&s, 2, AnchorRule::Move);
    sel.extend_to(&s, 2, Extend::Add);
    assert_eq!(selected(&sel, &s), [0, 1, 2, 4, 5, 6, 7]);
    assert_eq!(sel.len(), 7);
}

#[test]
fn space_inside_a_range_flips_the_cursor_and_the_flip_holds() {
    // integration/10xer-mode: `v` starts a range from the cursor and `V` starts an unselect range
    // The feature map has no bullet for Space inside a range yet. Strata's own test says "a
    // toggled item stays flipped as the range grows" (src/app/navigation/tests/visual.rs).
    let s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 5);
    sel.move_cursor(&s, 3, AnchorRule::Move); // `v` at entry 3
    sel.extend_to(&s, 3, Extend::Add);
    sel.extend_to(&s, 1, Extend::Add);
    assert_eq!(selected(&sel, &s), [1, 2, 3, 5]);

    let changed = sel.toggle_in_run(&s, 1);
    assert_eq!(
        changed,
        Changed {
            selection: true,
            ..Changed::NONE
        },
        "Space in a range moves neither the cursor nor the anchor"
    );
    assert_eq!(selected(&sel, &s), [2, 3, 5]);
    assert_eq!((cursor(&sel), anchor(&sel)), (Some(1), Some(3)));

    sel.extend_to(&s, 0, Extend::Add);
    assert_eq!(
        selected(&sel, &s),
        [0, 2, 3, 5],
        "the flip holds as the range grows"
    );
    // As in Strata, the flip applies over the whole walked result: once the range no longer
    // covers entry 1, flipping its unselected state selects it.
    sel.extend_to(&s, 2, Extend::Add);
    assert_eq!(selected(&sel, &s), [1, 2, 3, 5]);
    assert_eq!(sel.len(), 4);

    // Leaving the range keeps the fill, and the next `v` starts from it.
    sel.move_cursor(&s, 4, AnchorRule::Move);
    sel.extend_to(&s, 4, Extend::Add);
    assert_eq!(selected(&sel, &s), [1, 2, 3, 4, 5]);
}

#[test]
fn space_inside_an_unselect_range_flips_over_the_removal() {
    // integration/10xer-mode: `v` starts a range from the cursor and `V` starts an unselect range
    let s = listing(6);
    let mut sel = Selection::new();
    sel.select_all(&s);
    sel.move_cursor(&s, 1, AnchorRule::Move); // `V` at entry 1
    sel.extend_to(&s, 3, Extend::Remove);
    assert_eq!(selected(&sel, &s), [0, 4, 5]);
    sel.toggle_in_run(&s, 3);
    assert_eq!(selected(&sel, &s), [0, 3, 4, 5]);
    sel.extend_to(&s, 4, Extend::Remove);
    assert_eq!(selected(&sel, &s), [0, 3, 5]);
    sel.toggle_in_run(&s, 3);
    assert_eq!(selected(&sel, &s), [0, 5], "a second Space flips it back");
    assert_eq!(anchor(&sel), Some(1));
}

#[test]
fn toggle_in_run_outside_a_range_is_a_toggle() {
    // integration/10xer-mode: Space toggles the focused item in the selection and moves the cursor down
    let s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 0);
    sel.toggle_in_run(&s, 2);
    assert_eq!(selected(&sel, &s), [0, 2]);
    assert_eq!(anchor(&sel), Some(2));
    // A Shift range is no `v` range: Space there toggles and re-anchors too.
    sel.extend_to(&s, 4, Extend::Replace);
    sel.toggle_in_run(&s, 3);
    assert_eq!(selected(&sel, &s), [2, 4]);
    assert_eq!(anchor(&sel), Some(3));
    // So does a `v` range that a plain move has ended.
    sel.extend_to(&s, 5, Extend::Add);
    sel.move_cursor(&s, 1, AnchorRule::Keep);
    sel.toggle_in_run(&s, 1);
    assert_eq!(selected(&sel, &s), [1, 2, 3, 4, 5]);
    assert_eq!(anchor(&sel), Some(1));
}

#[test]
fn discard_provisional_then_toggle_adds_the_cursor() {
    // integration/10xer-mode: Space toggles the focused item in the selection and moves the cursor down
    let s = listing(3);
    let mut sel = Selection::new();
    sel.auto_select(&s, 0);
    sel.discard_provisional();
    sel.toggle(&s, 0);
    assert_eq!(selected(&sel, &s), [0]);
    assert_eq!(sel.targets(&s), Targets::Filled(vec![id(0)]));
}

#[test]
fn band_modes_apply_to_the_selection_at_press() {
    // browser/selection/marquee: A plain marquee replaces the selection with the entries under the band.
    // browser/selection/marquee: A Ctrl-marquee toggles each crossed entry relative to the selection at press.
    // browser/selection/marquee: A Shift-marquee adds the crossed entries to the selection at press.
    let s = listing(10);
    let mut sel = Selection::new();
    let press = |sel: &mut Selection| {
        sel.select_only(&s, 1);
        sel.toggle(&s, 4);
    };

    press(&mut sel);
    sel.begin_band(BandMode::Replace);
    sel.band(&s, &(3..6).into());
    sel.end_band();
    assert_eq!(selected(&sel, &s), [3, 4, 5]);

    press(&mut sel);
    sel.begin_band(BandMode::Toggle);
    sel.band(&s, &(3..7).into());
    assert_eq!(selected(&sel, &s), [1, 3, 5, 6]);
    // Moving the band back off 5 and 6 restores them to the selection at press.
    sel.band(&s, &(3..5).into());
    assert_eq!(selected(&sel, &s), [1, 3]);
    sel.end_band();

    press(&mut sel);
    sel.begin_band(BandMode::Add);
    sel.band(&s, &(6..8).into());
    sel.end_band();
    assert_eq!(selected(&sel, &s), [1, 4, 6, 7]);
    assert_eq!(cursor(&sel), Some(4), "the band leaves the cursor to C1d");
}

#[test]
fn cancel_band_restores_the_selection_at_press() {
    // browser/selection/marquee: A Ctrl-marquee toggles each crossed entry relative to the selection at press.
    let mut s = listing(10);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.toggle(&s, 4);
    sel.begin_band(BandMode::Toggle);
    sel.band(&s, &(0..10).into());
    // A live update during the band keeps the selection at press.
    remove(&mut s, &[9]);
    sel.sync(&s, SyncRule::default());
    sel.band(&s, &(2..4).into());
    assert_eq!(selected(&sel, &s), [1, 2, 3, 4]);
    assert!(sel.cancel_band().selection);
    assert_eq!(selected(&sel, &s), [1, 4]);
}

#[test]
fn position_round_trips_cursor_anchor_and_selection() {
    // browser/view-modes/position-restore: In Icons and List, returning to one of the last 128 folders left restores its selection, cursor, range anchor, and scroll position after loading.
    // browser/view-modes/position-restore: Pressing Down after a restore moves from the restored row, not the first row.
    // browser/view-modes/position-restore: Selection, cursor, and anchor carry between Icons and List
    let s = listing(8);
    let mut sel = Selection::new();
    sel.select_only(&s, 2);
    sel.extend_to(&s, 4, Extend::Replace);
    sel.toggle(&s, 6);
    let left = sel.position(&s);

    // Back in the folder, possibly in another view: a fresh selection over the same ids.
    let mut back = Selection::new();
    back.restore(&left);
    back.sync(&s, SyncRule::default());
    assert_eq!(selected(&back, &s), [2, 3, 4, 6]);
    assert_eq!((cursor(&back), anchor(&back)), (Some(6), Some(6)));
    assert_eq!(back.cursor_index(&s), Some(6));
}

#[test]
fn restore_drops_entries_gone_since() {
    // browser/view-modes/position-restore: In Icons and List, returning to one of the last 128 folders left restores its selection, cursor, range anchor, and scroll position after loading.
    // Design: restoring matches entries by location, not row number, so a deleted entry is
    // never selected. Decision 5: a cursor deleted since goes to the neighbour at its index.
    let mut s = listing(8);
    let mut sel = Selection::new();
    sel.select_only(&s, 3);
    sel.toggle(&s, 5);
    let left = sel.position(&s);
    remove(&mut s, &[5]);
    let mut back = Selection::new();
    back.restore(&left);
    back.sync(&s, SyncRule::default());
    assert_eq!(selected(&back, &s), [3]);
    assert_eq!(cursor(&back), Some(6));
}

#[test]
fn filter_round_trip_through_position() {
    // browser/search/filter: Escape on a focused result closes the filter and shows the full listing at once. Focus goes to the directory's cursor row, with the directory's own selection.
    // (The model side; C18 owns the bullet.)
    // Decision 4: the filter bar saves the position before filtering and restores it.
    let mut s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.toggle(&s, 4);
    let before = sel.position(&s);
    let all = s.entries().to_vec();
    s.retain(|e| e.id.get() % 2 == 0);
    sel.sync(&s, SyncRule::default());
    assert_eq!(selected(&sel, &s), [4]);
    s.replace(all);
    sel.restore(&before);
    sel.sync(&s, SyncRule::default());
    assert_eq!(selected(&sel, &s), [1, 4]);
    assert_eq!(cursor(&sel), Some(4));
}

#[test]
fn delete_and_rename_mid_band_keep_the_band_going() {
    // browser/selection/marquee: A Ctrl-marquee toggles each crossed entry relative to the selection at press.
    let mut s = listing(10);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.toggle(&s, 4);
    sel.begin_band(BandMode::Toggle);
    sel.band(&s, &(2..6).into());
    // The cursor's entry (4) is deleted mid-band: its neighbour joins the selection at press.
    delete(&mut sel, &mut s, &[4], CursorGone::NeighborSelected);
    assert!(sel.is_banding());
    assert_eq!(cursor(&sel), Some(5));
    // 5 joined the base but the band still crosses it, so the band flips it out.
    assert_eq!(selected(&sel, &s), [1, 2, 3]);
    // Entry 1 is renamed to 21 mid-band.
    let renamed: Vec<ListEntry> = s
        .entries()
        .iter()
        .map(|e| if e.id == id(1) { entry(21) } else { e.clone() })
        .collect();
    s.replace(renamed);
    sel.relocate(id(1), id(21));
    sel.sync(&s, SyncRule::default());
    assert!(sel.is_banding());
    // Moving the band back off everything restores the selection at press, renamed, plus 5.
    sel.band(&s, &IndexRanges::new());
    assert_eq!(selected(&sel, &s), [21, 5]);
    sel.end_band();
    assert!(!sel.is_banding());
    assert!(
        !sel.band(&s, &(0..3).into()).any(),
        "no band: nothing happens"
    );
}

#[test]
#[cfg_attr(
    debug_assertions,
    should_panic(expected = "the source changed without a Selection::sync")
)]
fn unsynced_shrink_then_extend_panics_only_in_debug() {
    let mut s = listing(10);
    let mut sel = Selection::new();
    sel.select_only(&s, 0);
    sel.extend_to(&s, 8, Extend::Replace);
    remove(&mut s, &[5, 6, 7, 8, 9]); // and no sync
    sel.extend_to(&s, 4, Extend::Replace);
    assert_eq!(selected(&sel, &s), [0, 1, 2, 3, 4]);
}

#[test]
fn unsynced_resort_still_gives_the_right_selected_ids() {
    let mut s = listing(10);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.extend_to(&s, 4, Extend::Replace);
    s.sort_by(|a, b| b.id.cmp(&a.id)); // and no sync
    assert_eq!(selected(&sel, &s), [4, 3, 2, 1]);
    assert_eq!(
        sel.targets(&s),
        Targets::Filled([4, 3, 2, 1].map(id).to_vec())
    );
    assert_eq!(sel.position(&s).selected, [4, 3, 2, 1].map(id));
}

#[test]
fn unsynced_resort_with_the_same_ends_still_gives_the_right_ids() {
    let mut s = listing(6);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.extend_to(&s, 3, Extend::Replace);
    // 2 and 4 swap places without a sync: the range's first (1) and last (3) ids stay put.
    let order = [0, 1, 4, 3, 2, 5];
    let rank = |e: &ListEntry| order.iter().position(|&r| r == e.id.get());
    s.sort_by(|a, b| rank(a).cmp(&rank(b)));
    assert_eq!(selected(&sel, &s), [1, 3, 2]);
    assert_eq!(sel.targets(&s), Targets::Filled([1, 3, 2].map(id).to_vec()));
}

#[test]
#[cfg_attr(
    debug_assertions,
    should_panic(expected = "which the selection already holds")
)]
fn relocate_onto_a_held_id_keeps_the_count_in_release() {
    let s = listing(4);
    let mut sel = Selection::new();
    sel.select_only(&s, 1);
    sel.toggle(&s, 2);
    sel.relocate(id(1), id(2));
    assert_eq!(sel.len(), 1);
    assert_eq!(selected(&sel, &s), [2]);
}

#[test]
fn removal_hint_lasts_through_loading_and_ends_at_a_complete_sync() {
    // browser/selection: Deleting the focused entry, in Strata or externally, focuses the next visible entry, else the previous visible one, and the preview follows.
    // A delete whose listing comes back through a loading sync still takes the next survivor.
    let mut s = listing(10);
    let mut sel = Selection::new();
    sel.select_only(&s, 6);
    delete_through_loading(&mut sel, &mut s, &[2, 3, 4, 5, 6]);
    assert_eq!(cursor(&sel), Some(7));

    // A delete that never lands: the host's next sync uses the record up, so a later reload
    // that drops the cursor's entry takes the reload rule (the entry now at its old index).
    let mut s = listing(12);
    let mut sel = Selection::new();
    sel.select_only(&s, 8);
    sel.before_removal(&s, |id| (3..=8).contains(&id.get()));
    assert!(!sel.sync(&s, SyncRule::default()).any());
    reload_without(&mut sel, &mut s, 8);
    assert_eq!(cursor(&sel), Some(9));
}

#[test]
fn before_removal_that_spares_the_cursor_leaves_the_reload_rule() {
    // browser/selection: A reload can drop the cursor's entry, as after a delete on a remote share. The cursor then moves to the first visible entry at or after its old index, unselected.
    let mut s = listing(12);
    let mut sel = Selection::new();
    sel.select_only(&s, 8);
    // The host prepares a removal of 0..5, which doesn't land; then a reload drops 8.
    sel.before_removal(&s, |id| id.get() < 5);
    reload_without(&mut sel, &mut s, 8);
    assert_eq!(cursor(&sel), Some(9));
}

/// A delete whose listing the host delivers through a loading sync first.
fn delete_through_loading(sel: &mut Selection, s: &mut ListSource, gone: &[u64]) {
    sel.before_removal(s, |id| gone.contains(&id.get()));
    remove(s, gone);
    sel.sync(s, SyncRule::loading());
    sel.sync(s, SyncRule::default());
}

/// A streamed reload that comes back without the entry `raw`.
fn reload_without(sel: &mut Selection, s: &mut ListSource, raw: u64) {
    let full: Vec<ListEntry> = s
        .entries()
        .iter()
        .filter(|e| e.id.get() != raw)
        .cloned()
        .collect();
    s.replace(Vec::new());
    sel.sync(s, SyncRule::loading());
    s.replace(full);
    sel.sync(s, SyncRule::default());
}
