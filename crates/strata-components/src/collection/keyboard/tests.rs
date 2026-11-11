//! One test per feature-map Behavior bullet the keyboard machine decides. Each names its
//! bullet; tests of the work package's own scope (type-ahead, Ctrl+Space, layouts) say so.
//! Ids are the entries' first indices. Page sizes are always the geometry's `page_step()`.

use std::time::Duration;

use rune::{Chord, ChordKey, Key, KeyEvent, Mods};

use super::*;
use crate::collection::element::KeyInput;
use crate::collection::selection::{Changed, Selection};
use crate::contract::{
    CollectionGeometry, Direction, EntryId, EntryKind, EntrySource, ListEntry, ListSource,
    UniformRows,
};

mod adapter;
mod grid;

use grid::Grid;

const ROW: f32 = 20.0;

/// A surface over a plain listing, geometry and selection, keeping what the machine emits.
struct Fake {
    src: ListSource,
    geo: Box<dyn CollectionGeometry>,
    settings: KeySettings,
    sel: Selection,
    out: Vec<Outcome>,
}

impl Surface for Fake {
    fn settings(&self) -> &KeySettings {
        &self.settings
    }

    fn source(&self) -> &dyn EntrySource {
        &self.src
    }

    fn geometry(&self) -> &dyn CollectionGeometry {
        &*self.geo
    }

    fn selection(&self) -> &Selection {
        &self.sel
    }

    fn apply(
        &mut self,
        op: &mut dyn FnMut(&mut Selection, &dyn EntrySource) -> Changed,
    ) -> Changed {
        op(&mut self.sel, &self.src)
    }

    fn emit(&mut self, outcome: Outcome) {
        self.out.push(outcome);
    }
}

/// The machine, its surface, its handle and a virtual clock.
struct Rig {
    m: KeyMachine,
    f: Fake,
    handle: KeyHandle,
    now: f64,
}

fn names(n: u64) -> Vec<String> {
    (0..n).map(|i| format!("e{i}")).collect()
}

fn source(names: &[&str]) -> ListSource {
    ListSource::new(
        names
            .iter()
            .enumerate()
            .map(|(i, n)| ListEntry::new(EntryId::new(i as u64), *n, EntryKind::File, ""))
            .collect(),
    )
}

impl Rig {
    fn new(src: ListSource, geo: Box<dyn CollectionGeometry>, s: KeySettings) -> Self {
        let handle = KeyHandle::new();
        Self {
            m: KeyMachine::new(handle.clone()),
            f: Fake {
                src,
                geo,
                settings: s,
                sel: Selection::new(),
                out: Vec::new(),
            },
            handle,
            now: 0.0,
        }
    }

    /// A list (or one Columns column) of `n` rows in a viewport ten rows tall.
    fn list_with(n: u64, s: KeySettings) -> Self {
        let names = names(n);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let geo = UniformRows::new(n as usize, ROW, 400.0).viewport_height(ROW * 10.0);
        Self::new(source(&refs), Box::new(geo), s)
    }

    fn list(n: u64) -> Self {
        Self::list_with(n, KeySettings::default())
    }

    fn tenxer(n: u64) -> Self {
        Self::list_with(n, KeySettings::tenxer())
    }

    /// An icons grid of `n` tiles, `cols` to a row, in a viewport four rows tall.
    fn grid_with(n: u64, cols: usize, s: KeySettings) -> Self {
        let names = names(n);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let geo = Grid {
            len: n as usize,
            cols,
            rows: 4,
        };
        Self::new(source(&refs), Box::new(geo), s)
    }

    fn grid(n: u64, cols: usize) -> Self {
        let s = KeySettings {
            wrap_rows: true,
            ..KeySettings::default()
        };
        Self::grid_with(n, cols, s)
    }

    fn page(&self) -> usize {
        self.f.geo.page_step()
    }

    fn press(&mut self, key: Key, mods: Mods) -> bool {
        let event = KeyEvent { key, mods };
        let mut input = KeyInput::new(&event);
        input.now = self.now;
        self.m.key(&input, &mut self.f)
    }

    fn key(&mut self, key: Key) -> bool {
        self.press(key, Mods::default())
    }

    fn shift(&mut self, key: Key) -> bool {
        self.press(key, SHIFT)
    }

    fn ctrl(&mut self, key: Key) -> bool {
        self.press(key, CTRL)
    }

    /// A key that types `c`, on a US layout.
    fn typed(&mut self, c: &str) -> bool {
        let mods = Mods {
            shift: c.chars().any(char::is_uppercase),
            ..Mods::default()
        };
        self.press(Key::Text(c.into()), mods)
    }

    /// Ctrl and the letter `c`, as a window sends it off macOS.
    fn ctrl_letter(&mut self, c: char) -> bool {
        self.press(Key::Command(c), CTRL)
    }

    /// Puts the cursor (and the selection and anchor) on `i`, as a click would.
    fn click(&mut self, i: usize) {
        self.f.sel.select_only(&self.f.src, i);
    }

    fn selected(&self) -> Vec<u64> {
        self.f
            .sel
            .selected_ids(&self.f.src)
            .into_iter()
            .map(EntryId::get)
            .collect()
    }

    fn cursor(&self) -> Option<usize> {
        self.f.sel.cursor_index(&self.f.src)
    }

    fn anchor(&self) -> Option<usize> {
        self.f.sel.anchor_index(&self.f.src)
    }

    fn take(&mut self) -> Vec<Outcome> {
        std::mem::take(&mut self.f.out)
    }

    fn last_len(&self) -> usize {
        self.f.src.len() - 1
    }
}

const SHIFT: Mods = Mods {
    shift: true,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};

/// Ctrl, as a window reports it off macOS: the key and the primary modifier.
const CTRL: Mods = Mods {
    shift: false,
    cmd: true,
    alt: false,
    ctrl: true,
    logo: false,
};

/// The default map with Type to search off: `h j k l` move.
fn arrows() -> KeySettings {
    KeySettings {
        letters: Letters::Arrows,
        ..KeySettings::default()
    }
}

fn range(a: u64, b: u64) -> Vec<u64> {
    (a..=b).collect()
}

fn activated(out: &[Outcome]) -> Vec<u64> {
    out.iter()
        .filter_map(|o| match o {
            Outcome::Activate { id } => Some(id.get()),
            _ => None,
        })
        .collect()
}

// browser/selection/keyboard-navigation: within the file list.

#[test]
fn up_and_down_move_focus_and_selection() {
    // browser/selection/keyboard-navigation: In Icons and List, Up and Down move focus and selection through entries without changing directory.
    let mut r = Rig::list(8);
    r.click(2);
    assert!(r.key(Key::Down));
    assert_eq!(
        (r.selected(), r.cursor(), r.anchor()),
        (vec![3], Some(3), Some(3))
    );
    assert_eq!(r.take(), [Outcome::Reveal(3)]);
    assert!(r.key(Key::Up) && r.key(Key::Up));
    assert_eq!((r.selected(), r.cursor()), (vec![1], Some(1)));
    assert!(activated(&r.take()).is_empty(), "moving never opens");

    let mut g = Rig::grid(12, 4);
    g.click(1);
    g.key(Key::Down);
    assert_eq!((g.selected(), g.cursor()), (vec![5], Some(5)));
}

#[test]
fn icons_arrows_move_one_tile_by_rows_as_laid_out() {
    // browser/selection/keyboard-navigation: In Icons, arrows move one tile in that direction, using rows as laid out.
    let mut g = Rig::grid(10, 4);
    g.click(5);
    g.key(Key::Right);
    assert_eq!(g.cursor(), Some(6));
    g.key(Key::Down);
    assert_eq!(
        g.cursor(),
        Some(6),
        "no tile below 6 in a 10-tile grid: the bottom edge"
    );
    assert_eq!(g.take().last(), Some(&Outcome::Edge(Direction::Down)));
    g.key(Key::Up);
    assert_eq!(g.cursor(), Some(2));
    g.key(Key::Left);
    assert_eq!((g.selected(), g.cursor()), (vec![1], Some(1)));
}

#[test]
fn list_right_stays_on_the_entry_and_never_opens() {
    // browser/selection/keyboard-navigation: In List, Right keeps focus on the current entry and never opens it or changes directory.
    let mut r = Rig::list(5);
    r.click(2);
    assert!(r.key(Key::Right), "taken, so nothing else moves");
    assert_eq!((r.selected(), r.cursor()), (vec![2], Some(2)));
    assert_eq!(r.take(), [Outcome::Edge(Direction::Right)]);
}

#[test]
fn hjkl_act_as_arrows_with_type_to_search_off() {
    // browser/selection/keyboard-navigation: With Type to search off, `h`, `j`, `k`, and `l` act as Left, Down, Up, and Right in every view and the file chooser.
    let s = KeySettings {
        letters: Letters::Arrows,
        wrap_rows: true,
        ..KeySettings::default()
    };
    let mut g = Rig::grid_with(12, 4, s.clone());
    g.click(5);
    g.typed("l");
    assert_eq!(g.cursor(), Some(6));
    g.typed("j");
    assert_eq!(g.cursor(), Some(10));
    g.typed("h");
    assert_eq!(g.cursor(), Some(9));
    g.typed("k");
    assert_eq!((g.selected(), g.cursor()), (vec![5], Some(5)));

    // Only plain: Shift+J is no Shift+Down.
    let mut r = Rig::list_with(6, s);
    r.click(1);
    assert!(!r.typed("J"));
    assert_eq!(r.selected(), [1]);
}

#[test]
fn hjkl_pass_with_type_to_search_on() {
    // browser/selection/keyboard-navigation: With Type to search on, `h`, `j`, `k`, and `l` pressed in the file list start a search instead of moving.
    let mut r = Rig::list(6);
    r.click(1);
    for c in ["h", "j", "k", "l"] {
        assert!(!r.typed(c), "{c} goes on to the host's search");
    }
    assert_eq!((r.selected(), r.cursor()), (vec![1], Some(1)));
    assert!(r.take().is_empty());
}

#[test]
fn left_at_the_first_tile_and_in_list_reaches_the_edge() {
    // browser/selection/keyboard-navigation: In Icons, Left on the first tile focuses the visible sidebar; in List, Left from any entry does.
    // browser/selection/keyboard-navigation: With the sidebar hidden, Left in Icons or List does not change directory.
    let mut g = Rig::grid(8, 4);
    g.click(0);
    assert!(g.key(Key::Left));
    assert_eq!(g.take(), [Outcome::Edge(Direction::Left)]);
    assert_eq!(g.selected(), [0]);

    let mut r = Rig::list(5);
    r.click(3);
    assert!(r.key(Key::Left));
    assert_eq!(r.take(), [Outcome::Edge(Direction::Left)], "no Activate");
    assert_eq!(r.selected(), [3]);
}

#[test]
fn left_at_a_later_row_start_wraps_to_the_previous_row() {
    // browser/selection/keyboard-navigation: In Icons, Left on a later row's first tile moves to the previous row's last tile, with Keep arrows on or off.
    // browser/selection/keyboard-navigation: In the portal file chooser's Icons view, Left at a row start wraps to the previous row.
    let mut g = Rig::grid(12, 4);
    g.click(4);
    g.key(Key::Left);
    assert_eq!((g.selected(), g.cursor()), (vec![3], Some(3)));
    // Right at a row end wraps the other way, as GTK's grid does.
    g.key(Key::Right);
    assert_eq!(g.cursor(), Some(4));
    g.click(11);
    g.take();
    g.key(Key::Right);
    assert_eq!(g.take(), [Outcome::Edge(Direction::Right)], "the last tile");
}

#[test]
fn up_from_the_first_entry_reaches_the_edge_even_when_empty() {
    // browser/selection/keyboard-navigation: Up from the first Icons row or first List entry focuses the pane header, including in an empty directory.
    let mut r = Rig::list(4);
    r.click(0);
    assert!(r.key(Key::Up));
    assert_eq!(r.take(), [Outcome::Edge(Direction::Up)]);

    let mut g = Rig::grid(8, 4);
    g.click(2);
    g.key(Key::Up);
    assert_eq!(g.take(), [Outcome::Edge(Direction::Up)]);

    let mut empty = Rig::list(0);
    assert!(empty.key(Key::Up));
    assert_eq!(empty.take(), [Outcome::Edge(Direction::Up)]);
}

// browser/selection: keyboard selection.

#[test]
fn shift_up_and_down_extend_from_the_anchor() {
    // browser/selection: Shift+Up and Shift+Down extend the selection by one entry from the range anchor.
    let mut r = Rig::list(8);
    r.click(3);
    r.shift(Key::Down);
    r.shift(Key::Down);
    assert_eq!(
        (r.selected(), r.cursor(), r.anchor()),
        (range(3, 5), Some(5), Some(3))
    );
    r.shift(Key::Up);
    r.shift(Key::Up);
    r.shift(Key::Up);
    assert_eq!(
        (r.selected(), r.cursor(), r.anchor()),
        (range(2, 3), Some(2), Some(3))
    );
    assert_eq!(r.take().last(), Some(&Outcome::Reveal(2)));
}

#[test]
fn shift_page_keys_extend_or_shrink_by_a_page() {
    // browser/selection: Shift+Page Up and Shift+Page Down extend or shrink the selection by one page from the range anchor.
    let mut r = Rig::list(60);
    let page = r.page() as u64;
    r.click(5);
    r.shift(Key::PageDown);
    assert_eq!(r.selected(), range(5, 5 + page));
    r.shift(Key::PageDown);
    assert_eq!(r.selected(), range(5, 5 + 2 * page));
    r.shift(Key::PageUp);
    assert_eq!(
        r.selected(),
        range(5, 5 + page),
        "shrinks back towards the anchor"
    );
    assert_eq!(r.anchor(), Some(5));
}

#[test]
fn shift_down_with_nothing_selected_selects_the_first_then_two() {
    // browser/selection: Shift+Down with nothing selected selects only the first entry; the next Shift+Down selects two.
    let mut r = Rig::list(5);
    r.shift(Key::Down);
    assert_eq!((r.selected(), r.cursor()), (vec![0], Some(0)));
    r.shift(Key::Down);
    assert_eq!(r.selected(), [0, 1]);
}

#[test]
fn with_no_cursor_down_return_and_shift_down_start_at_the_first_up_at_the_last() {
    // browser/selection: With no cursor in a grouped List, Down, Return, and Shift+Down start at the first row shown. Up and Shift+Up start at the last row shown.
    let mut r = Rig::list(5);
    r.key(Key::Down);
    assert_eq!(r.cursor(), Some(0));
    let mut r = Rig::list(5);
    r.key(Key::Up);
    assert_eq!(r.cursor(), Some(4));
    let mut r = Rig::list(5);
    r.shift(Key::Up);
    assert_eq!((r.selected(), r.cursor()), (vec![4], Some(4)));
    // Recorded decision: Return with no cursor selects the first entry and opens it.
    let mut r = Rig::list(5);
    assert!(r.key(Key::Enter));
    assert_eq!(r.selected(), [0]);
    assert_eq!(activated(&r.take()), [0]);
    let mut empty = Rig::list(0);
    assert!(
        !empty.key(Key::Enter),
        "an empty folder leaves Enter to the host"
    );
}

#[test]
fn after_escape_the_next_shift_range_starts_at_the_cursor() {
    // browser/selection: After Escape clears a selection, the next Shift+arrow range starts at the keyboard cursor.
    let mut r = Rig::list(8);
    r.click(2);
    r.shift(Key::Down);
    r.shift(Key::Down);
    assert!(r.key(Key::Escape));
    assert!(r.selected().is_empty());
    assert_eq!(r.cursor(), Some(4));
    r.shift(Key::Down);
    assert_eq!((r.selected(), r.cursor()), (vec![4], Some(4)));
    r.shift(Key::Down);
    assert_eq!(r.selected(), [4, 5]);
}

#[test]
fn plain_home_end_and_pages_move_the_anchor() {
    // browser/selection: In List, Icons, and Columns, plain Home, End, Page Up, and Page Down move the range anchor to the new cursor. Shift+Home after End selects every entry.
    let mut r = Rig::list(40);
    let page = r.page();
    r.click(3);
    r.key(Key::PageDown);
    assert_eq!(r.anchor(), Some(3 + page));
    r.key(Key::PageUp);
    assert_eq!(r.anchor(), Some(3));
    r.key(Key::Home);
    assert_eq!(r.anchor(), Some(0));
    r.key(Key::End);
    assert_eq!(r.anchor(), Some(39));
    r.shift(Key::Home);
    assert_eq!(r.selected(), range(0, 39));
}

#[test]
fn after_home_shift_down_selects_the_first_two() {
    // browser/selection: After Home, Shift+Down selects the first two entries.
    let mut r = Rig::list(6);
    r.click(4);
    r.key(Key::Home);
    r.shift(Key::Down);
    assert_eq!(r.selected(), [0, 1]);
}

#[test]
fn a_plain_move_re_anchors_shift_paging() {
    // browser/selection: In List and Icons, a plain arrow or `h`/`j`/`k`/`l` move that changes the selection moves the range anchor. Shift+Page Up and Shift+Page Down then extend from the new cursor.
    let s = arrows();
    let mut r = Rig::list_with(60, s);
    let page = r.page() as u64;
    r.click(10);
    r.shift(Key::Down);
    r.key(Key::Down);
    assert_eq!(r.anchor(), Some(12));
    r.typed("j");
    assert_eq!(r.anchor(), Some(13));
    r.shift(Key::PageDown);
    assert_eq!(r.selected(), range(13, 13 + page));
}

#[test]
fn columns_shift_home_and_end_extend_from_the_anchor() {
    // browser/selection: In Columns, Shift+Home and Shift+End extend from the range anchor to the first or last entry, not only the edge entry.
    // A Columns column is a list of uniform rows.
    let mut r = Rig::list(9);
    r.click(3);
    r.shift(Key::End);
    assert_eq!((r.selected(), r.anchor()), (range(3, 8), Some(3)));
    r.shift(Key::Home);
    assert_eq!((r.selected(), r.anchor()), (range(0, 3), Some(3)));
}

#[test]
fn ctrl_a_selects_every_entry_and_keeps_the_cursor() {
    // C1c scope: Ctrl+A routing. Which collection hears it (the focused column, not the
    // deepest) is C2's, with one controller per column.
    let mut r = Rig::list(5);
    r.click(2);
    assert!(r.ctrl_letter('a'));
    assert_eq!(r.selected(), range(0, 4));
    assert_eq!(r.cursor(), Some(2), "the cursor stays");
    assert_eq!(
        r.take(),
        [Outcome::Keyboard],
        "keyboard navigation, as Strata's select_all"
    );
}

#[test]
fn escape_clears_the_selection_and_keeps_the_cursor() {
    // browser/selection: With no transient UI open, Escape clears the active pane's selection and keeps the keyboard cursor and directory.
    let mut r = Rig::list(6);
    r.click(1);
    r.shift(Key::Down);
    assert!(r.key(Key::Escape));
    assert!(r.selected().is_empty());
    assert_eq!(r.cursor(), Some(2));
    assert!(
        r.take().contains(&Outcome::Keyboard),
        "the cursor outline shows"
    );
}

#[test]
fn enter_after_escape_opens_the_cursor_entry() {
    // browser/selection: Enter after Escape opens the folder under the keyboard cursor.
    let mut r = Rig::list(6);
    r.click(3);
    r.key(Key::Escape);
    r.take();
    assert!(r.key(Key::Enter));
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: EntryId::new(3)
        }]
    );
}

#[test]
fn escape_with_nothing_selected_is_left_to_the_view() {
    // browser/selection: Outside 10xer mode, in Columns, Escape with nothing selected in the active column closes the deepest column.
    let mut r = Rig::list(4);
    r.click(1);
    assert!(r.key(Key::Escape));
    assert!(
        !r.key(Key::Escape),
        "nothing left to clear: the column may close"
    );
}

// browser/scrolling: paging and jumps.

#[test]
fn page_keys_move_focus_and_selection_by_a_page() {
    // browser/scrolling: Page Up and Page Down
    // (The keypad keys are Rune's logical keys; only the focused column moving is C2's.)
    let mut r = Rig::list(50);
    let page = r.page();
    r.click(2);
    r.key(Key::PageDown);
    assert_eq!(
        (r.cursor(), r.selected()),
        (Some(2 + page), vec![(2 + page) as u64])
    );
    r.key(Key::PageUp);
    assert_eq!(r.cursor(), Some(2));
    // A page with no cursor: Page Down starts at the first entry.
    let mut fresh = Rig::list(50);
    fresh.key(Key::PageDown);
    assert_eq!(fresh.cursor(), Some(0));
}

#[test]
fn a_page_is_the_geometry_page_step_in_icons_rows_times_tiles() {
    // browser/scrolling: A page is the number of rows that fit the viewport minus one, kept as overlap; in Icons it is those rows times the tiles per row.
    // The size is the geometry's (W1 for lists, C3 for the grid): the machine moves by it.
    let mut g = Rig::grid(60, 5);
    let page = g.page();
    g.click(1);
    g.key(Key::PageDown);
    assert_eq!(g.cursor(), Some(1 + page));
    assert_eq!(
        g.cursor().map(|i| i % 5),
        Some(1),
        "the same column, rows down"
    );
}

#[test]
fn paging_stops_at_the_ends_and_keeps_the_cursor_in_view() {
    // browser/scrolling: Paging stops at the first and last entry and keeps the selected item in view.
    let mut r = Rig::list(15);
    let last = r.last_len();
    r.click(12);
    r.key(Key::PageDown);
    assert_eq!(r.cursor(), Some(last));
    assert_eq!(r.take(), [Outcome::Reveal(last)]);
    r.key(Key::PageDown);
    assert_eq!(r.cursor(), Some(last));
    r.click(3);
    r.key(Key::PageUp);
    assert_eq!((r.cursor(), r.selected()), (Some(0), vec![0]));
    assert_eq!(r.take().last(), Some(&Outcome::Reveal(0)));
}

#[test]
fn ctrl_up_and_down_select_the_first_and_last_and_scroll_there() {
    // browser/scrolling: Ctrl+Up and Ctrl+Down select the first or last entry of the focused pane or column and scroll to that edge.
    let mut r = Rig::list(30);
    r.click(10);
    r.shift(Key::Down);
    assert!(r.ctrl(Key::Down));
    assert_eq!((r.selected(), r.cursor()), (vec![29], Some(29)));
    assert_eq!(r.take().last(), Some(&Outcome::Reveal(29)));
    assert!(r.ctrl(Key::Up));
    assert_eq!((r.selected(), r.cursor()), (vec![0], Some(0)));
    assert_eq!(r.take(), [Outcome::Reveal(0)]);
}

#[test]
fn ctrl_up_and_down_with_more_modifiers_do_not_jump() {
    // browser/scrolling: Ctrl+Up or Ctrl+Down with Shift, Alt, or Super added does not jump.
    let mut r = Rig::list(30);
    r.click(10);
    for extra in [Mods { alt: true, ..CTRL }, Mods { logo: true, ..CTRL }] {
        assert!(!r.press(Key::Up, extra), "left to the host and Rune");
        assert!(!r.press(Key::Down, extra));
    }
    assert_eq!((r.selected(), r.cursor()), (vec![10], Some(10)));
    // With Shift it extends instead, as Shift+Up does with Ctrl held (Strata's EXTENDS).
    let ctrl_shift = Mods {
        shift: true,
        ..CTRL
    };
    assert!(r.press(Key::Up, ctrl_shift));
    assert_eq!((r.selected(), r.cursor()), (vec![9, 10], Some(9)));
}

#[test]
fn home_and_end_jump_like_ctrl_up_and_down() {
    // browser/scrolling: In the default key map, Home and End on a focused listing jump to the first or last entry like Ctrl+Up and Ctrl+Down. Keypad Home and End do the same.
    let mut r = Rig::list(20);
    r.click(8);
    r.key(Key::End);
    assert_eq!((r.selected(), r.cursor()), (vec![19], Some(19)));
    r.key(Key::Home);
    assert_eq!((r.selected(), r.cursor()), (vec![0], Some(0)));
    assert_eq!(r.take(), [Outcome::Reveal(19), Outcome::Reveal(0)]);
}

#[test]
fn shift_home_and_end_extend_instead_of_jumping() {
    // browser/scrolling: Shift+Home and Shift+End extend the selection range instead of jumping.
    let mut r = Rig::list(10);
    r.click(4);
    r.shift(Key::End);
    assert_eq!((r.selected(), r.anchor()), (range(4, 9), Some(4)));
    r.shift(Key::Home);
    assert_eq!(r.selected(), range(0, 4));
}

#[test]
fn plain_home_or_end_leaves_only_the_target_selected() {
    // browser/scrolling: Plain Home or End with several entries selected leaves only the target entry selected.
    let mut r = Rig::list(10);
    r.click(2);
    r.shift(Key::Down);
    r.shift(Key::Down);
    r.key(Key::End);
    assert_eq!(r.selected(), [9]);
}

// The work package's own scope.

#[test]
fn ctrl_moves_the_cursor_without_selecting_and_ctrl_space_toggles() {
    // C1c scope: Ctrl moves the cursor without selecting, and Ctrl+Space toggles.
    let mut g = Rig::grid(8, 4);
    g.click(1);
    assert!(g.ctrl(Key::Right));
    assert_eq!(
        (g.selected(), g.cursor(), g.anchor()),
        (vec![1], Some(2), Some(1))
    );
    assert!(g.press(Key::Text(" ".into()), CTRL));
    assert_eq!((g.selected(), g.anchor()), (vec![1, 2], Some(2)));
    g.press(Key::Text(" ".into()), CTRL);
    assert_eq!(g.selected(), [1]);
}

#[test]
fn a_repeated_key_counts_as_a_press() {
    // C1c scope (recorded decision): a key repeat moves like a press.
    let mut r = Rig::list(5);
    r.click(0);
    let down = KeyEvent {
        key: Key::Down,
        mods: Mods::default(),
    };
    let mut input = KeyInput::new(&down);
    input.repeat = true;
    for _ in 0..3 {
        assert!(r.m.key(&input, &mut r.f));
    }
    assert_eq!(r.cursor(), Some(3));
}

#[test]
fn a_host_rebinds_and_lists_keys() {
    // C1c scope: the key map is configurable (Strata's keymap rebinding).
    let mut map = KeyMap::default_map();
    let down = Command::Select(Target::Step(Direction::Down));
    let chords = map.chords_for(down);
    assert!(chords.contains(&Chord::new(ChordKey::Down)));
    assert!(
        chords.contains(&Chord::new(ChordKey::Char('j'))),
        "the alias counts"
    );
    let ctrl_n = Chord::new(ChordKey::Char('n')).with(rune::ChordMod::Ctrl);
    map.alias(ctrl_n, ChordKey::Down);
    map.unbind(Command::SelectAll);
    let s = KeySettings {
        map,
        ..KeySettings::default()
    };
    let mut r = Rig::list_with(5, s);
    r.click(1);
    assert!(
        r.ctrl_letter('n'),
        "an alias with Ctrl applies with letters passed on"
    );
    assert_eq!(r.cursor(), Some(2));
    assert!(!r.ctrl_letter('a'));
    assert_eq!(r.selected(), [2]);
}

// integration/10xer-mode.

#[test]
fn tenxer_moves_the_cursor_in_display_order_and_keeps_the_fill() {
    // integration/10xer-mode: In List and Columns, `j`/`k`, Up/Down, Home, End, and `G` move the cursor in displayed order, including sorted and type-grouped lists.
    let mut t = Rig::tenxer(10);
    t.click(2);
    t.typed("j");
    t.key(Key::Down);
    assert_eq!(t.cursor(), Some(4));
    t.typed("k");
    t.key(Key::Up);
    assert_eq!(t.cursor(), Some(2));
    t.key(Key::End);
    assert_eq!(t.cursor(), Some(9));
    t.key(Key::Home);
    assert_eq!(t.cursor(), Some(0));
    t.typed("G");
    assert_eq!(t.cursor(), Some(9));
    assert_eq!(
        t.selected(),
        [2],
        "the cursor moves apart from the selection"
    );
}

#[test]
fn tenxer_jumps_land_on_the_ends_without_a_cursor() {
    // integration/10xer-mode: In Columns, `g g` and `G` in a column with no cursor land on its first and last entry.
    let mut t = Rig::tenxer(7);
    t.typed("G");
    assert_eq!(t.cursor(), Some(6));
    let mut t = Rig::tenxer(7);
    // `g g` is C18's chord; its step runs the machine's First through the handle.
    t.handle.run(Command::Cursor(Target::First));
    assert!(t.handle.pending());
    assert!(t.m.tick(&mut t.f));
    assert_eq!(t.cursor(), Some(0));
    assert_eq!(t.take(), [Outcome::Reveal(0)]);
    assert!(!t.m.tick(&mut t.f), "nothing left queued");
}

#[test]
fn tenxer_half_and_full_pages() {
    // integration/10xer-mode: Ctrl+U/Ctrl+D move half a page, and Ctrl+B/Ctrl+F or Page Up/Page Down a full page, without filtering, toggling the sidebar, or duplicating.
    let mut t = Rig::tenxer(80);
    let page = t.page();
    let half = (page / 2).max(1);
    t.click(30);
    assert!(t.ctrl_letter('d'));
    assert_eq!(t.cursor(), Some(30 + half));
    assert!(t.ctrl_letter('u'));
    assert_eq!(t.cursor(), Some(30));
    assert!(t.ctrl_letter('f'));
    assert_eq!(t.cursor(), Some(30 + page));
    assert!(t.ctrl_letter('b'));
    assert_eq!(t.cursor(), Some(30));
    assert!(t.key(Key::PageDown) && t.key(Key::PageUp));
    assert_eq!(t.cursor(), Some(30));
}

#[test]
fn tenxer_enter_and_o_open_the_cursor_item() {
    // integration/10xer-mode: Enter and `o` open the focused item, including files.
    let mut t = Rig::tenxer(5);
    t.click(1);
    t.typed("j");
    t.take();
    assert!(t.key(Key::Enter));
    assert!(t.typed("o"));
    assert_eq!(activated(&t.take()), [2, 2]);
}

#[test]
fn tenxer_icons_keys_move_a_tile_and_never_open() {
    // integration/10xer-mode: In Icons, `h`/`j`/`k`/`l` and the arrows move to the next tile in that direction, also over search results. They never open, leave a folder, or enter a preview.
    let mut s = KeySettings::tenxer();
    s.wrap_rows = true;
    let mut g = Rig::grid_with(12, 4, s);
    g.click(5);
    g.typed("l");
    g.typed("j");
    assert_eq!(g.cursor(), Some(10));
    g.key(Key::Left);
    g.key(Key::Up);
    assert_eq!(g.cursor(), Some(5));
    assert!(activated(&g.take()).is_empty());
}

#[test]
fn tenxer_space_toggles_and_moves_down() {
    // integration/10xer-mode: Space toggles the focused item in the selection and moves the cursor down, without previewing and without using the hovered row.
    let mut t = Rig::tenxer(4);
    // A load's automatic pick: the first Space adds that item.
    t.f.sel.auto_select(&t.f.src, 0);
    assert!(t.typed(" "));
    assert_eq!((t.selected(), t.cursor()), (vec![0], Some(1)));
    assert!(!t.f.sel.is_provisional());
    t.typed(" ");
    assert_eq!((t.selected(), t.cursor()), (vec![0, 1], Some(2)));
    t.typed("k");
    t.typed(" ");
    assert_eq!((t.selected(), t.cursor()), (vec![0], Some(2)), "removes 1");
    t.typed("G");
    t.typed(" ");
    assert_eq!(
        (t.selected(), t.cursor()),
        (vec![0, 3], Some(3)),
        "stays on the last"
    );
}

#[test]
fn tenxer_ctrl_a_selects_all_and_ctrl_r_inverts() {
    // integration/10xer-mode: Ctrl+A selects every item in the focused pane, and Ctrl+R inverts that pane's selection instead of renaming.
    let mut t = Rig::tenxer(5);
    t.click(1);
    t.ctrl_letter('r');
    assert_eq!(t.selected(), [0, 2, 3, 4]);
    t.ctrl_letter('a');
    assert_eq!(t.selected(), range(0, 4));
    t.ctrl_letter('r');
    assert!(t.selected().is_empty());
}

#[test]
fn tenxer_moves_after_a_fill_keep_the_selection() {
    // integration/10xer-mode: After Space, Ctrl+A, or Ctrl+R, Home, End, and paging move the cursor and keep the selection.
    let mut t = Rig::tenxer(40);
    t.click(3);
    t.typed(" ");
    t.ctrl_letter('r');
    let fill = t.selected();
    t.key(Key::End);
    t.key(Key::Home);
    t.key(Key::PageDown);
    assert_eq!(t.cursor(), Some(t.page()));
    assert_eq!(t.selected(), fill);
}

#[test]
fn tenxer_v_and_shift_v_start_ranges() {
    // integration/10xer-mode: `v` starts a range from the cursor and `V` starts an unselect range; the footer shows `VISUAL` or `UNSET` while it is active.
    let mut t = Rig::tenxer(10);
    t.click(2);
    t.typed("j");
    assert!(t.typed("v"));
    assert_eq!(t.handle.range(), Some(RangeKind::Select));
    t.typed("j");
    t.typed("j");
    assert_eq!(t.selected(), [2, 3, 4, 5]);
    t.typed("v");
    assert_eq!(t.handle.range(), None, "the same key ends it");
    t.typed("k");
    assert_eq!(
        t.selected(),
        [2, 3, 4, 5],
        "walking after the range keeps the fill"
    );
    t.typed("V");
    assert_eq!(t.handle.range(), Some(RangeKind::Unset));
    t.typed("k");
    assert_eq!(t.selected(), [2, 5]);
    assert!(t.handle.end_range(), "the host's Escape ladder ends it");
    t.typed("k");
    assert_eq!(t.selected(), [2, 5]);
}

#[test]
fn tenxer_shift_arrows_do_nothing_and_paging_extends_a_range() {
    // integration/10xer-mode: `v` and `V` are the only range keys: Shift+arrows do nothing in the listing, and paging extends an active range.
    let mut t = Rig::tenxer(50);
    let page = t.page() as u64;
    t.click(4);
    for k in [Key::Up, Key::Down, Key::Left, Key::Right] {
        assert!(t.shift(k), "taken");
    }
    assert_eq!((t.selected(), t.cursor()), (vec![4], Some(4)));
    t.typed("v");
    t.key(Key::PageDown);
    assert_eq!(t.selected(), range(4, 4 + page));
}

#[test]
fn a_range_ends_when_something_else_changes_the_selection() {
    // integration/10xer-mode: `v` starts a range from the cursor and `V` starts an unselect range; the footer shows `VISUAL` or `UNSET` while it is active.
    // A click during a range ends it, as a pointer change does in Strata.
    let mut t = Rig::tenxer(10);
    t.click(1);
    t.typed("v");
    t.click(6);
    t.typed("j");
    assert_eq!(t.handle.range(), None);
    assert_eq!((t.selected(), t.cursor()), (vec![6], Some(7)));
}

#[test]
fn tenxer_empty_folder_has_nothing_to_select() {
    // integration/10xer-mode: In an empty folder, Space, `v`, `V`, Ctrl+A, and Ctrl+R flash `Nothing to select`.
    let mut t = Rig::tenxer(0);
    t.typed(" ");
    t.typed("v");
    t.typed("V");
    t.ctrl_letter('a');
    t.ctrl_letter('r');
    assert_eq!(t.take(), vec![Outcome::NothingToSelect; 5]);
    assert_eq!(t.handle.range(), None);
}

#[test]
fn tenxer_keys_never_run_their_default_action() {
    // integration/10xer-mode: With the file list focused, `h`/`j`/`k`/`l`, other letters, Space, Ctrl+D, Ctrl+F, Ctrl+B, and Ctrl+R never also run their default-map action.
    // Taking a key keeps it from the page and Rune; letters the 10xer map doesn't bind (its
    // verbs) are C18's and the host's.
    let mut t = Rig::tenxer(30);
    t.click(10);
    for c in ["h", "j", "k", "l", " "] {
        assert!(t.typed(c), "{c:?}");
    }
    for c in ['d', 'f', 'b', 'r'] {
        assert!(t.ctrl_letter(c), "Ctrl+{c}");
    }
}

#[test]
fn tenxer_ctrl_up_and_down_move_the_cursor_only() {
    // C1c scope (recorded decision): in 10xer, Ctrl+Up and Ctrl+Down move the cursor only.
    let mut t = Rig::tenxer(9);
    t.click(4);
    t.ctrl(Key::Down);
    assert_eq!((t.selected(), t.cursor()), (vec![4], Some(8)));
    t.ctrl(Key::Up);
    assert_eq!((t.selected(), t.cursor()), (vec![4], Some(0)));
}

#[test]
fn tenxer_space_inside_a_range_flips_the_cursor_and_keeps_the_range() {
    // integration/10xer-mode: `v` starts a range from the cursor and `V` starts an unselect range; the footer shows `VISUAL` or `UNSET` while it is active.
    // The feature map has no bullet for Space inside a range (an owner item); this cites the
    // closest, as W1d's tests do.
    // Inside a range, Strata's Space flips the cursor's entry against the range and later
    // steps keep the flip (`toggle_visual_cursor`, `VisualRange.toggled`).
    let mut t = Rig::tenxer(10);
    t.click(2);
    t.typed("v");
    t.typed("j");
    t.typed("j");
    assert_eq!(t.selected(), [2, 3, 4]);
    t.typed(" ");
    assert_eq!(t.selected(), [2, 3]);
    assert_eq!(t.handle.range(), Some(RangeKind::Select));
    t.typed("j");
    assert_eq!(
        t.selected(),
        [2, 3, 5],
        "4 stays flipped out as the range walks on"
    );
}

// Type-ahead (the work package's own scope; Strata has no counterpart).

fn type_ahead(names: &[&str]) -> Rig {
    let s = KeySettings {
        letters: Letters::TypeAhead,
        type_ahead_timeout: Duration::from_secs(1),
        ..KeySettings::default()
    };
    let geo = UniformRows::new(names.len(), ROW, 400.0).viewport_height(ROW * 10.0);
    Rig::new(source(names), Box::new(geo), s)
}

#[test]
fn type_ahead_finds_by_prefix_until_the_timeout() {
    // C1c scope: type-ahead finds by a typed prefix, with a timeout.
    let mut r = type_ahead(&["alpha", "Beta", "bravo", "brown", "charlie"]);
    assert!(r.typed("b"));
    assert_eq!(r.selected(), [1], "case-insensitive");
    r.now = 0.3;
    r.typed("r");
    assert_eq!(r.selected(), [2]);
    r.now = 0.6;
    r.typed("o");
    assert_eq!(r.selected(), [3]);
    assert_eq!(r.take().last(), Some(&Outcome::Reveal(3)));
    // After the timeout a new prefix starts, searching on from the cursor and wrapping.
    r.now = 2.0;
    r.typed("a");
    assert_eq!(r.selected(), [0]);
    // No match: the key is taken and nothing moves.
    r.now = 4.0;
    assert!(r.typed("z"));
    assert_eq!(r.selected(), [0]);
}

#[test]
fn type_ahead_repeating_a_letter_steps_through_its_entries() {
    // C1c scope: type-ahead finds by a typed prefix, with a timeout.
    let mut r = type_ahead(&["bat", "cat", "bee", "bird"]);
    r.typed("b");
    assert_eq!(r.cursor(), Some(0));
    r.now = 0.2;
    r.typed("b");
    assert_eq!(r.cursor(), Some(2));
    r.now = 0.4;
    r.typed("b");
    assert_eq!(r.cursor(), Some(3));
    r.now = 0.6;
    r.typed("b");
    assert_eq!(r.cursor(), Some(0), "wraps");
}

#[test]
fn type_ahead_takes_space_only_inside_a_prefix_and_is_off_by_default() {
    // C1c scope: type-ahead finds by a typed prefix, with a timeout.
    let mut r = type_ahead(&["my notes", "myth", "my files"]);
    assert!(!r.typed(" "), "Space with no prefix is the host's");
    r.typed("m");
    r.now = 0.1;
    r.typed("y");
    r.now = 0.2;
    assert!(r.typed(" "));
    r.now = 0.3;
    r.typed("f");
    assert_eq!(r.selected(), [2]);
    // Bound keys still move, and close the prefix.
    r.key(Key::Up);
    assert_eq!(r.cursor(), Some(1));

    let mut off = Rig::list(3);
    assert!(!off.typed("e"));
    assert_eq!(KeySettings::default().letters, Letters::Pass);
}

// Review round 1.

#[test]
fn ctrl_left_and_right_report_no_edge_and_pass_at_one() {
    // C1c scope: only a plain Left goes towards the sidebar (Strata's single_pane_arrow_action).
    // Ctrl+Left and Ctrl+Right report no edge; at one they aren't taken, so a Columns column
    // hands them to C2, as Strata's directory_navigation proceeds with Ctrl held.
    let mut r = Rig::list(5);
    r.click(2);
    assert!(!r.ctrl(Key::Left) && !r.ctrl(Key::Right), "passed on");
    assert!(r.take().is_empty(), "no edge");
    assert_eq!((r.selected(), r.cursor()), (vec![2], Some(2)));
    let mut g = Rig::grid(8, 4);
    g.click(0);
    assert!(!g.ctrl(Key::Left));
    assert!(g.take().is_empty());
    assert!(g.ctrl(Key::Right), "a nudge that moves is taken");
    assert_eq!(
        (g.selected(), g.cursor()),
        (vec![0], Some(1)),
        "a nudge still moves"
    );
    // 10xer moves keep reporting edges: `h` and Left open the parent in List and Columns.
    let mut t = Rig::tenxer(5);
    t.click(2);
    t.typed("h");
    t.key(Key::Left);
    assert_eq!(
        t.take(),
        [
            Outcome::Edge(Direction::Left),
            Outcome::Edge(Direction::Left)
        ]
    );
}

#[test]
fn shift_up_and_down_extend_with_ctrl_alt_or_super_held_too() {
    // browser/selection: Shift+Up and Shift+Down extend the selection by one entry from the range anchor.
    // Strata's selection_navigation ignores Ctrl, Alt and Super on them.
    let mut r = Rig::list(10);
    r.click(2);
    let alt = Mods { alt: true, ..SHIFT };
    let sup = Mods {
        logo: true,
        ..SHIFT
    };
    let all = Mods {
        alt: true,
        logo: true,
        ..Mods {
            shift: true,
            ..CTRL
        }
    };
    assert!(r.press(Key::Down, alt) && r.press(Key::Down, sup) && r.press(Key::Down, all));
    assert_eq!(r.selected(), range(2, 5));
    // Shift+Left and Shift+Right tolerate nothing: Ctrl+Shift+Left is left alone.
    assert!(!r.press(
        Key::Left,
        Mods {
            shift: true,
            ..CTRL
        }
    ));
}

#[test]
fn page_keys_ignore_super_and_escape_ignores_shift() {
    // browser/selection: Shift+Page Up and Shift+Page Down extend or shrink the selection by one page from the range anchor.
    // browser/selection: With no transient UI open, Escape clears the active pane's selection and keeps the keyboard cursor and directory.
    let mut r = Rig::list(60);
    let page = r.page();
    r.click(2);
    let sup = Mods {
        logo: true,
        ..Mods::default()
    };
    assert!(r.press(Key::PageDown, sup));
    assert_eq!(r.cursor(), Some(2 + page));
    assert!(r.press(
        Key::PageDown,
        Mods {
            logo: true,
            ..SHIFT
        }
    ));
    assert_eq!(r.selected().len(), page + 1);
    assert!(r.press(Key::Escape, SHIFT));
    assert!(r.selected().is_empty());
    assert_eq!(r.cursor(), Some(2 + 2 * page));
}

#[test]
fn a_sideways_shift_step_with_no_neighbour_is_left_to_the_view() {
    // browser/selection: Shift+Up and Shift+Down extend the selection by one entry from the range anchor.
    // Shift+Left and Shift+Right have no neighbour in a list or column: Columns' Shift+Right
    // enters the folder (C2), and List does nothing, even with nothing selected.
    let mut r = Rig::list(5);
    r.click(2);
    assert!(!r.shift(Key::Right) && !r.shift(Key::Left));
    assert!(r.take().is_empty());
    r.key(Key::Escape);
    assert!(!r.shift(Key::Left));
    assert!(
        r.selected().is_empty(),
        "nothing selected, and still nothing"
    );
    // In the grid a sideways neighbour extends as usual.
    let mut g = Rig::grid(8, 4);
    g.click(1);
    assert!(g.shift(Key::Right));
    assert_eq!(g.selected(), [1, 2]);
}

#[test]
fn shift_down_with_no_anchor_steps_from_the_first_entry() {
    // browser/selection: Shift+Down with nothing selected selects only the first entry; the next Shift+Down selects two.
    // A selection with no cursor or anchor (a marquee's) isn't "nothing selected": Strata steps,
    // anchoring at the first entry, rather than selecting it in place.
    let mut r = Rig::list(6);
    r.f.sel
        .begin_band(crate::collection::selection::BandMode::Replace);
    r.f.sel.band(&r.f.src, &(3..5).into());
    r.f.sel.end_band();
    assert_eq!((r.selected(), r.cursor()), (vec![3, 4], None));
    r.shift(Key::Down);
    assert_eq!(
        (r.selected(), r.cursor(), r.anchor()),
        (vec![0, 1], Some(1), Some(0))
    );
}

#[test]
fn tenxer_v_with_no_cursor_has_nothing_to_select() {
    // integration/10xer-mode: `v` starts a range from the cursor and `V` starts an unselect range; the footer shows `VISUAL` or `UNSET` while it is active.
    let mut t = Rig::tenxer(5);
    t.typed("v");
    assert_eq!(t.take(), [Outcome::NothingToSelect]);
    assert_eq!((t.handle.range(), t.cursor()), (None, None));
}

#[test]
fn an_edge_reached_by_a_held_key_is_reported_once() {
    // browser/selection/keyboard-navigation: Up from the first Icons row or first List entry focuses the pane header, including in an empty directory.
    let mut r = Rig::list(4);
    r.click(2);
    let up = KeyEvent {
        key: Key::Up,
        mods: Mods::default(),
    };
    let mut held = KeyInput::new(&up);
    held.repeat = true;
    for _ in 0..5 {
        assert!(r.m.key(&held, &mut r.f));
    }
    let edges = r
        .take()
        .into_iter()
        .filter(|o| matches!(o, Outcome::Edge(_)))
        .count();
    assert_eq!(edges, 1, "once on arrival");
    r.key(Key::Up);
    assert_eq!(
        r.take(),
        [Outcome::Edge(Direction::Up)],
        "a new press reports it again"
    );
}

#[test]
fn reset_drops_queued_commands() {
    // C1c scope: a folder change drops what is in progress. A `g g` queued for a folder that was left doesn't run in the next one.
    let mut t = Rig::tenxer(5);
    t.handle.run(Command::Cursor(Target::Last));
    t.m.reset();
    assert!(!t.handle.pending());
    assert!(!t.m.tick(&mut t.f));
    assert_eq!(t.cursor(), None);
}

// Review round 2.

#[test]
fn an_empty_collection_passes_sideways_shift_steps() {
    // C1c scope: a sideways Shift step with no neighbour is the view's (C2's Shift+Right enters,
    // Shift+Left opens the parent), in an empty folder too.
    let mut r = Rig::list(0);
    assert!(!r.shift(Key::Left) && !r.shift(Key::Right));
    assert!(r.shift(Key::Down), "a vertical one is still taken");
}

#[test]
fn an_exact_binding_wins_over_a_tolerated_match() {
    // C1c scope: the key map is configurable (Strata's keymap rebinding, C20).
    // Shift+Escape bound to Select all takes it from Escape, which tolerates Shift.
    let mut map = KeyMap::default_map();
    map.bind(
        Chord::new(ChordKey::Escape).with(rune::ChordMod::Shift),
        Command::SelectAll,
    );
    // Ctrl+Shift+Up bound to the first entry takes it from Shift+Up, which tolerates Ctrl.
    map.bind(
        Chord::new(ChordKey::Up)
            .with(rune::ChordMod::Ctrl)
            .with(rune::ChordMod::Shift),
        Command::Select(Target::First),
    );
    let mut r = Rig::list_with(
        8,
        KeySettings {
            map,
            ..KeySettings::default()
        },
    );
    r.click(4);
    assert!(r.press(Key::Escape, SHIFT));
    assert_eq!(r.selected(), range(0, 7), "Shift+Escape selects all");
    assert!(r.press(
        Key::Up,
        Mods {
            shift: true,
            ..CTRL
        }
    ));
    assert_eq!(
        (r.selected(), r.cursor()),
        (vec![0], Some(0)),
        "Ctrl+Shift+Up jumps"
    );
}

#[test]
fn unbinding_a_tolerated_chord_takes_it_off_its_binding() {
    // C1c scope: the key map is configurable (Strata's keymap rebinding, C20).
    let super_page_down = Chord::new(ChordKey::PageDown).with(rune::ChordMod::Super);
    let sup = Mods {
        logo: true,
        ..Mods::default()
    };
    let mut map = KeyMap::default_map();
    map.unbind_chord(super_page_down);
    let mut r = Rig::list_with(
        40,
        KeySettings {
            map: map.clone(),
            ..KeySettings::default()
        },
    );
    r.click(2);
    assert!(
        !r.press(Key::PageDown, sup),
        "Super+Page Down no longer pages"
    );
    assert!(r.key(Key::PageDown), "plain Page Down still does");
    assert_eq!(r.cursor(), Some(2 + r.page()));
    // Rebound, it runs the new command.
    map.bind(super_page_down, Command::Select(Target::Last));
    let mut r = Rig::list_with(
        40,
        KeySettings {
            map,
            ..KeySettings::default()
        },
    );
    r.click(2);
    assert!(r.press(Key::PageDown, sup));
    assert_eq!(r.cursor(), Some(39));
}
