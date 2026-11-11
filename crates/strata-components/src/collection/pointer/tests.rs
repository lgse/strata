//! One test per feature-map Behavior bullet the pointer machine decides. Each names its
//! bullet. Entries 0–2 are folders and 3–9 files; ids are the entries' first indices.

use std::time::Duration;

use rune::Mods;

use super::*;

mod element;
use crate::collection::selection::{Changed, Extend, Selection, SyncRule};
use crate::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource, Point};

const ROW: f32 = 20.0;
const INTERVAL: f64 = 0.4;

/// A surface over a plain listing and selection, keeping what the machine emits.
struct Fake {
    settings: PointerSettings,
    src: ListSource,
    sel: Selection,
    out: Vec<Outcome>,
}

impl Surface for Fake {
    fn settings(&self) -> PointerSettings {
        self.settings
    }

    fn source(&self) -> &dyn EntrySource {
        &self.src
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

/// The machine, its surface and a virtual clock.
struct Rig {
    m: PointerMachine,
    f: Fake,
    now: f64,
}

fn entry(raw: u64) -> ListEntry {
    let kind = if raw < 3 {
        EntryKind::Folder
    } else {
        EntryKind::File
    };
    ListEntry::new(EntryId::new(raw), format!("e{raw}"), kind, "")
}

fn id(raw: u64) -> EntryId {
    EntryId::new(raw)
}

const PLAIN: Mods = Mods {
    shift: false,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};
const CTRL: Mods = Mods {
    ctrl: true,
    cmd: true,
    ..PLAIN
};
const SHIFT: Mods = Mods {
    shift: true,
    ..PLAIN
};
const ALT: Mods = Mods { alt: true, ..PLAIN };

fn modes(files: ClickCount, folders: ClickCount) -> PointerSettings {
    PointerSettings {
        modes: ClickModes::new(files, folders),
        ..PointerSettings::default()
    }
}

impl Rig {
    fn new(settings: PointerSettings) -> Self {
        Self {
            m: PointerMachine::new(),
            f: Fake {
                settings,
                src: ListSource::new((0..10).map(entry).collect()),
                sel: Selection::new(),
                out: Vec::new(),
            },
            now: 0.0,
        }
    }

    /// Files and folders both open on a double click: Strata's default in Icons and List.
    fn double() -> Self {
        Self::new(PointerSettings::default())
    }

    /// Folders open on one click, files on two: Strata's default in Columns.
    fn columns() -> Self {
        Self::new(modes(ClickCount::Two, ClickCount::One))
    }

    fn wait(&mut self, secs: f64) -> &mut Self {
        self.now += secs;
        self
    }

    fn down_with(&mut self, button: Button, index: Option<usize>, spot: Spot, mods: Mods) {
        let y = index.map_or(500.0, |i| i as f32 * ROW + ROW / 2.0);
        let at = Point::new(40.0, y);
        let press = Press {
            button,
            at,
            window: at,
            index,
            spot,
            mods,
            now: self.now,
        };
        self.m.press(&press, &mut self.f);
    }

    fn down(&mut self, index: usize, spot: Spot, mods: Mods) {
        self.down_with(Button::Primary, Some(index), spot, mods);
    }

    fn to(&mut self, dx: f32, dy: f32) {
        self.m.moved(Point::new(dx, dy), &mut self.f);
    }

    fn up_at(&mut self, travel: Point, mods: Mods, cancelled: bool) {
        let release = Release {
            travel,
            mods,
            now: self.now,
            cancelled,
        };
        self.m.release(&release, &mut self.f);
    }

    fn up(&mut self) {
        self.up_at(Point::default(), PLAIN, false);
    }

    fn up_with(&mut self, mods: Mods) {
        self.up_at(Point::default(), mods, false);
    }

    /// A click: down, and up a moment later on the same spot.
    fn click(&mut self, index: usize, spot: Spot, mods: Mods) {
        self.down(index, spot, mods);
        self.wait(0.05);
        self.up();
    }

    fn tick(&mut self) -> bool {
        self.m.tick(self.now, &mut self.f)
    }

    fn take(&mut self) -> Vec<Outcome> {
        std::mem::take(&mut self.f.out)
    }

    fn selected(&self) -> Vec<u64> {
        self.f
            .sel
            .selected_ids(&self.f.src)
            .into_iter()
            .map(EntryId::get)
            .collect()
    }

    fn select(&mut self, indices: &[usize]) {
        let (sel, src) = (&mut self.f.sel, &self.f.src);
        sel.clear();
        for &i in indices {
            sel.toggle(src, i);
        }
    }

    /// A click on the name of the only selected `index`, after it was selected and the
    /// interval passed: a rename is now pending.
    fn arm(&mut self, index: usize) {
        self.select(&[index]);
        self.wait(1.0);
        self.click(index, Spot::Name, PLAIN);
        assert!(self.m.rename_pending(), "the slow click armed a rename");
        assert_eq!(
            self.take(),
            [Outcome::WakeAfter(Duration::from_secs_f64(INTERVAL))]
        );
    }

    /// Runs the clock past a pending rename's interval and ticks.
    fn settle(&mut self) {
        self.wait(INTERVAL + 0.01);
        self.tick();
    }
}

fn drag(ids: &[u64]) -> impl Fn(&Outcome) -> bool + '_ {
    move |o| matches!(o, Outcome::BeginDrag { ids: got, .. } if got.iter().map(|i| i.get()).eq(ids.iter().copied()))
}

fn is_marquee(o: &Outcome) -> bool {
    matches!(o, Outcome::BeginMarquee { .. })
}

fn opens(o: &Outcome) -> bool {
    matches!(o, Outcome::Activate { .. } | Outcome::Clicked { .. })
}

// Hit regions and drag selection.

#[test]
fn a_drag_from_an_unselected_entry_selects_it_on_press_and_drags_it() {
    // browser/selection/pointer-intent: On an unselected entry, a drag from the icon or rendered filename starts a file drag.
    // browser/selection/pointer-intent: Pressing and dragging an unselected entry selects it on press and deselects the previous selection.
    for spot in [Spot::Content, Spot::Name] {
        let mut r = Rig::double();
        r.select(&[5, 6]);
        r.down(3, spot, PLAIN);
        assert_eq!(r.selected(), [3], "selected at the press, before any move");
        r.to(0.0, 30.0);
        let out = r.take();
        assert_eq!(out.len(), 1);
        assert!(drag(&[3])(&out[0]));
    }
}

#[test]
fn a_drag_from_blank_space_beside_an_unselected_entry_starts_a_marquee() {
    // browser/selection/pointer-intent: On an unselected entry, a drag from blank row space beside the filename starts a marquee.
    let mut r = Rig::double();
    r.down(4, Spot::Gutter, PLAIN);
    r.to(30.0, 30.0);
    assert_eq!(
        r.take(),
        [Outcome::BeginMarquee {
            at: Point::new(40.0, 4.0 * ROW + ROW / 2.0),
            mods: PLAIN,
        }],
        "the band starts at the press"
    );
}

#[test]
fn a_drag_from_blank_space_on_a_selected_entry_drags_the_selection() {
    // browser/selection/pointer-intent: On a selected entry, a drag from blank row or card space starts a file drag of the selection.
    let mut r = Rig::double();
    r.select(&[2, 4]);
    r.down(4, Spot::Gutter, PLAIN);
    r.to(0.0, -30.0);
    let out = r.take();
    assert!(out.iter().any(drag(&[2, 4])));
    assert!(!out.iter().any(is_marquee));
}

#[test]
fn a_plain_press_inside_a_multi_selection_keeps_the_group_for_the_drag() {
    // browser/selection/pointer-intent: Pressing an entry inside a multi-selection without Ctrl or Shift keeps the group, so the drag carries every selected entry.
    let mut r = Rig::double();
    r.select(&[3, 5, 7]);
    r.down(5, Spot::Name, PLAIN);
    assert_eq!(r.selected(), [3, 5, 7]);
    r.to(20.0, 0.0);
    r.up_at(Point::new(20.0, 0.0), PLAIN, false);
    assert!(r.take().iter().any(drag(&[3, 5, 7])));
    assert_eq!(r.selected(), [3, 5, 7], "a drag never collapses the group");
}

#[test]
fn a_plain_click_inside_a_multi_selection_collapses_it_to_the_entry() {
    // browser/view-modes/columns: With folders opening on one click, a plain click on a folder inside a multi-selection collapses the selection to it and opens its column.
    // browser/view-modes/columns: With single-click previews on, a plain click on a previewable file inside a multi-selection collapses the selection to it and previews it.
    let mut r = Rig::columns();
    r.select(&[1, 2, 4]);
    r.click(1, Spot::Content, PLAIN);
    assert_eq!(r.selected(), [1]);
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: id(1),
            how: ActivateBy::Click
        }]
    );
    r.select(&[1, 4, 5]);
    r.wait(1.0).click(4, Spot::Content, PLAIN);
    assert_eq!(r.selected(), [4]);
    assert_eq!(r.take(), [Outcome::Clicked { id: id(4) }]);
}

#[test]
fn ctrl_and_shift_clicks_on_blank_row_space_toggle_and_range() {
    // browser/selection/pointer-intent: Ctrl+click and Shift+click on blank row space still toggle and range.
    let mut r = Rig::double();
    r.select(&[3]);
    r.down(6, Spot::Gutter, CTRL);
    assert_eq!(r.selected(), [3], "a drag could still make it a marquee");
    r.up();
    assert_eq!(r.selected(), [3, 6]);
    r.wait(1.0).click(8, Spot::Gutter, SHIFT);
    assert_eq!(
        r.selected(),
        [6, 7, 8],
        "ranges from the anchor the toggle moved"
    );
}

#[test]
fn ctrl_drag_from_an_entry_adds_it_and_drags() {
    // browser/selection/pointer-intent: Ctrl-drag from an entry's icon or name copies it and adds it to the selection.
    // Checks the "adds it" half: the copy is the host's, from the modifiers at the drag.
    let mut r = Rig::double();
    r.select(&[3]);
    r.down(5, Spot::Content, CTRL);
    assert_eq!(
        r.selected(),
        [3, 5],
        "an unselected entry joins at the press"
    );
    r.to(0.0, 30.0);
    assert!(r.take().iter().any(drag(&[3, 5])));

    r.select(&[3, 5]);
    r.wait(1.0).down(5, Spot::Content, CTRL);
    r.to(0.0, 30.0);
    r.up_at(Point::new(0.0, 30.0), CTRL, false);
    assert!(r.take().iter().any(drag(&[3, 5])));
    assert_eq!(r.selected(), [3, 5], "a selected entry stays in the drag");

    r.wait(1.0).click(5, Spot::Content, CTRL);
    assert_eq!(
        r.selected(),
        [3],
        "a Ctrl click without a drag toggles it off"
    );
}

#[test]
fn shift_drag_from_an_entry_drags_the_range() {
    // browser/selection/pointer-intent: Shift-drag from an entry's icon or name moves it.
    // Checks the drag of the range; the move is the host's, from the modifiers at the drag.
    let mut r = Rig::double();
    r.click(3, Spot::Content, PLAIN);
    r.wait(1.0).down(5, Spot::Name, SHIFT);
    r.to(-30.0, 0.0);
    assert!(r.take().iter().any(drag(&[3, 4, 5])));
}

#[test]
fn an_alt_drag_from_an_entry_starts_a_marquee() {
    // browser/selection/marquee: Holding Alt starts a marquee even when the press lands on an entry.
    let mut r = Rig::double();
    r.down(5, Spot::Name, ALT);
    r.to(0.0, 30.0);
    let out = r.take();
    assert!(out.iter().any(is_marquee));
    assert!(!out.iter().any(drag(&[5])));
}

#[test]
fn alt_with_ctrl_or_shift_on_an_entry_leaves_the_selection_to_the_band() {
    // browser/selection/marquee: Holding Alt starts a marquee even when the press lands on an entry.
    // How the band then toggles or adds is C1d's. Strata takes the band's selection at press before the row's own press changes anything
    // (`marquee/state.rs`), so a modified Alt press changes nothing until its release.
    for (mods, index) in [
        (Mods { alt: true, ..CTRL }, 5),
        (Mods { alt: true, ..SHIFT }, 7),
    ] {
        let mut r = Rig::double();
        r.click(3, Spot::Content, PLAIN);
        r.take();
        r.wait(1.0).down(index, Spot::Name, mods);
        assert_eq!(r.selected(), [3], "{mods:?}: nothing changes at the press");
        r.to(0.0, 30.0);
        assert_eq!(r.selected(), [3], "{mods:?}: nor when the band starts");
        assert!(
            r.take().contains(&Outcome::BeginMarquee {
                at: Point::new(40.0, index as f32 * ROW + ROW / 2.0),
                mods,
            }),
            "the band gets the modifiers held at the press"
        );
    }
}

#[test]
fn alt_with_ctrl_or_shift_on_an_entry_toggles_or_ranges_on_release() {
    // browser/selection: Ctrl+click toggles one entry in or out of the selection and moves the range anchor to it.
    // browser/selection: Shift+click selects every entry from the range anchor to the clicked entry.
    let mut r = Rig::double();
    r.click(3, Spot::Content, PLAIN);
    r.wait(1.0).down(5, Spot::Name, Mods { alt: true, ..CTRL });
    r.up();
    assert_eq!(r.selected(), [3, 5]);
    r.wait(1.0)
        .down(7, Spot::Content, Mods { alt: true, ..SHIFT });
    r.up();
    assert_eq!(
        r.selected(),
        [5, 6, 7],
        "ranges from the anchor the toggle moved"
    );
}

// Threshold.

#[test]
fn travel_within_the_threshold_stays_a_click() {
    // browser/view-modes/columns: Pressing a row and moving less than GTK's drag threshold starts no file drag.
    let mut r = Rig::double();
    r.down(4, Spot::Content, PLAIN);
    r.to(8.0, 0.0);
    r.to(7.0, 7.0);
    r.up_at(Point::new(7.0, 7.0), PLAIN, false);
    assert_eq!(
        r.take(),
        [Outcome::Clicked { id: id(4) }],
        "each axis stays within 8 pt, as GTK measures it"
    );
}

#[test]
fn a_press_that_crossed_the_threshold_never_previews_or_opens() {
    // browser/selection/pointer-intent: A press that crosses the drag threshold never opens a preview, even after the drop.
    // browser/selection/pointer-intent: A press that crossed the drag threshold does not activate on release, even if the pointer returns to its start.
    let mut r = Rig::new(modes(ClickCount::One, ClickCount::One));
    r.down(4, Spot::Content, PLAIN);
    r.to(0.0, 12.0);
    r.to(0.0, 0.0);
    r.up();
    let out = r.take();
    assert!(out.iter().any(drag(&[4])));
    assert!(!out.iter().any(opens));

    r.wait(0.1).click(4, Spot::Content, PLAIN);
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: id(4),
            how: ActivateBy::Click
        }],
        "the dragged press ends the run"
    );

    r.wait(1.0).down(4, Spot::Content, PLAIN);
    r.up_at(Point::default(), PLAIN, true);
    assert!(r.take().is_empty(), "a cancelled press never opens");
}

#[test]
fn a_press_whose_entry_moved_during_it_does_not_open() {
    // browser/selection/pointer-intent: A press on an entry whose row was recycled during the press does not activate on release.
    let mut r = Rig::new(modes(ClickCount::One, ClickCount::One));
    r.down(1, Spot::Content, PLAIN);
    r.f.src.sort_by(|a, b| b.id.cmp(&a.id));
    r.f.sel.sync(&r.f.src, SyncRule::default());
    r.up();
    assert!(r.take().is_empty());
}

#[test]
fn a_press_after_a_release_that_never_came_starts_fresh() {
    // No bullet of its own: a system drag (P1) can take the release, so the next press must
    // start afresh rather than finish the old gesture.
    let mut r = Rig::new(modes(ClickCount::One, ClickCount::One));
    r.down(4, Spot::Content, PLAIN);
    r.to(0.0, 30.0);
    r.take();
    r.wait(1.0).click(6, Spot::Content, PLAIN);
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: id(6),
            how: ActivateBy::Click
        }]
    );
    assert_eq!(r.selected(), [6]);
}

// Clicks and previews.

#[test]
fn a_held_press_sends_its_click_only_on_release() {
    // browser/selection/pointer-intent: With single-click previews on, a press held on a file opens no preview; release without dragging opens it.
    let mut r = Rig::double();
    r.down(4, Spot::Content, PLAIN);
    r.wait(2.0);
    assert!(r.take().is_empty());
    r.up();
    assert_eq!(r.take(), [Outcome::Clicked { id: id(4) }]);
}

// Click modes.

#[test]
fn with_single_set_one_click_opens_on_release() {
    // browser/selection/click-modes: With Single set, one click on a folder opens it.
    let mut r = Rig::columns();
    r.down(1, Spot::Content, PLAIN);
    assert!(r.take().is_empty(), "nothing opens on the press");
    r.up();
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: id(1),
            how: ActivateBy::Click
        }]
    );
    r.wait(0.1).click(1, Spot::Content, PLAIN);
    assert!(
        r.take().is_empty(),
        "the second click of a run doesn't open it again"
    );
}

#[test]
fn with_double_set_one_click_only_selects_and_two_open() {
    // browser/selection/click-modes: With Double set, one click on a folder only selects it.
    let mut r = Rig::double();
    r.click(1, Spot::Content, PLAIN);
    assert_eq!(r.selected(), [1]);
    assert_eq!(r.take(), [Outcome::Clicked { id: id(1) }]);
    r.wait(0.2).click(1, Spot::Content, PLAIN);
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: id(1),
            how: ActivateBy::DoubleClick
        }]
    );
}

#[test]
fn a_triple_click_opens_once_and_arms_no_rename() {
    // operations/rename: A quick double-click opens the item instead; another click, a drag, any key press, or a selection change before the interval cancels the pending rename.
    // GTK keeps counting a third press (3, 4, …) and Strata acts only on the first and second.
    let mut r = Rig::double();
    for _ in 0..3 {
        r.click(4, Spot::Name, PLAIN);
        r.wait(0.1);
    }
    r.settle();
    assert_eq!(
        r.take(),
        [
            Outcome::Clicked { id: id(4) },
            Outcome::Activate {
                id: id(4),
                how: ActivateBy::DoubleClick
            }
        ]
    );
}

#[test]
fn a_triple_click_with_single_set_opens_once() {
    // browser/selection/click-modes: With Single set, one click on a folder opens it.
    let mut r = Rig::columns();
    for _ in 0..3 {
        r.click(1, Spot::Name, PLAIN);
        r.wait(0.1);
    }
    r.settle();
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: id(1),
            how: ActivateBy::Click
        }]
    );
}

#[test]
fn two_clicks_slower_than_the_interval_select_without_opening() {
    // browser/selection/click-modes: With Double set, two clicks slower than the double-click interval select without opening.
    let mut r = Rig::double();
    r.click(1, Spot::Content, PLAIN);
    // 0.45 s apart: Rune's fixed 0.5 s would pair them; the 0.4 s setting doesn't.
    r.wait(0.4).click(1, Spot::Content, PLAIN);
    let out = r.take();
    assert!(!out.iter().any(|o| matches!(o, Outcome::Activate { .. })));
    assert_eq!(r.selected(), [1]);
}

#[test]
fn ctrl_and_shift_clicks_change_the_selection_without_opening() {
    // browser/selection/click-modes: With Single set, Ctrl+click or Shift+click on an entry changes the selection without opening it.
    let mut r = Rig::new(modes(ClickCount::One, ClickCount::One));
    r.select(&[4]);
    r.click(1, Spot::Content, CTRL);
    assert_eq!(r.selected(), [1, 4]);
    r.wait(1.0).click(6, Spot::Name, SHIFT);
    assert_eq!(r.selected(), [1, 2, 3, 4, 5, 6]);
    assert!(r.take().is_empty());
}

#[test]
fn an_alt_click_opens_with_single_set() {
    // browser/selection/click-modes: With Single set, one click on a folder opens it.
    // Alt is not a selection modifier: Strata's `plain_single` reads only Ctrl and Shift.
    let mut r = Rig::columns();
    r.click(1, Spot::Content, ALT);
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: id(1),
            how: ActivateBy::Click
        }]
    );
}

#[test]
fn a_changed_click_count_applies_to_the_next_click() {
    // browser/selection/click-modes: A changed click count applies to open windows without restarting and persists across restarts.
    // Checks the "without restarting" half: persistence is the host's preferences.
    let mut r = Rig::double();
    r.click(1, Spot::Content, PLAIN);
    assert_eq!(r.take(), [Outcome::Clicked { id: id(1) }]);
    r.f.settings.modes.folders = ClickCount::One;
    r.wait(1.0).click(2, Spot::Content, PLAIN);
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: id(2),
            how: ActivateBy::Click
        }]
    );
}

// Slow-click rename.

#[test]
fn a_slow_click_on_the_sole_selections_name_renames_after_the_interval() {
    // operations/rename: Clicking the name text of the only selected item again, after the double-click interval, opens the editor.
    let mut r = Rig::double();
    r.click(4, Spot::Name, PLAIN);
    assert_eq!(r.take(), [Outcome::Clicked { id: id(4) }]);
    assert!(!r.m.rename_pending(), "the first click only selects");
    r.wait(1.0);
    r.arm_again(4);
    r.wait(INTERVAL - 0.05);
    assert!(!r.tick(), "not before the interval");
    r.take();
    r.wait(0.06);
    assert!(r.tick());
    assert_eq!(r.take(), [Outcome::BeginRename { id: id(4) }]);
}

#[test]
fn an_early_tick_asks_to_be_woken_when_the_rename_is_due() {
    // operations/rename: Clicking the name text of the only selected item again, after the double-click interval, opens the editor.
    // A tick can come early (another controller's wake-up); the rename still needs its own.
    let mut r = Rig::double();
    r.arm(4);
    r.wait(0.1);
    assert!(!r.tick());
    match r.take()[..] {
        [Outcome::WakeAfter(d)] => {
            assert!((d.as_secs_f64() - (INTERVAL - 0.1)).abs() < 1e-9, "{d:?}")
        }
        ref other => panic!("{other:?}"),
    }
    r.settle();
    assert_eq!(r.take().last(), Some(&Outcome::BeginRename { id: id(4) }));
}

impl Rig {
    /// A second click on an already selected `index`'s name, which arms a rename.
    fn arm_again(&mut self, index: usize) {
        self.click(index, Spot::Name, PLAIN);
        assert!(self.m.rename_pending());
        self.take();
    }
}

#[test]
fn an_automatic_pick_counts_as_the_only_selection() {
    // operations/rename: Clicking the name text of the only selected item again, after the double-click interval, opens the editor.
    let mut r = Rig::double();
    r.f.sel.auto_select(&r.f.src, 0);
    r.wait(1.0).click(0, Spot::Name, PLAIN);
    r.settle();
    assert_eq!(r.take().last(), Some(&Outcome::BeginRename { id: id(0) }));
}

#[test]
fn a_quick_double_click_opens_instead_of_renaming() {
    // operations/rename: A quick double-click opens the item instead; another click, a drag, any key press, or a selection change before the interval cancels the pending rename.
    let mut r = Rig::double();
    r.arm(4);
    r.wait(0.1).click(4, Spot::Name, PLAIN);
    r.settle();
    assert_eq!(
        r.take(),
        [Outcome::Activate {
            id: id(4),
            how: ActivateBy::DoubleClick
        }]
    );
}

#[test]
fn a_press_key_or_selection_change_cancels_a_pending_rename() {
    // operations/rename: A quick double-click opens the item instead; another click, a drag, any key press, or a selection change before the interval cancels the pending rename.
    type Cancel = fn(&mut Rig);
    let cases: [(&str, Cancel); 6] = [
        ("a click elsewhere", |r| r.click(7, Spot::Gutter, PLAIN)),
        ("a drag", |r| {
            r.down(4, Spot::Content, PLAIN);
            r.to(0.0, 30.0);
        }),
        ("a key", |r| r.m.interrupt()),
        ("a reset: another folder, or a press elsewhere", |r| {
            r.m.reset()
        }),
        ("a selection change", |r| {
            r.f.sel
                .move_cursor(&r.f.src, 5, crate::collection::selection::AnchorRule::Keep);
        }),
        ("a selection change that ends where it began", |r| {
            r.f.sel.select_all(&r.f.src);
            r.f.sel.select_only(&r.f.src, 4);
        }),
    ];
    for (what, cancel) in cases {
        let mut r = Rig::double();
        r.arm(4);
        r.wait(0.1);
        cancel(&mut r);
        r.settle();
        assert!(
            !r.take().contains(&Outcome::BeginRename { id: id(4) }),
            "{what} cancels it"
        );
    }
}

#[test]
fn clicking_pausing_then_double_clicking_opens_without_renaming() {
    // operations/rename: With Double click mode set, clicking a folder, pausing, then double-clicking it opens the folder and leaves no rename editor.
    let mut r = Rig::double();
    r.click(1, Spot::Name, PLAIN);
    r.wait(1.0).click(1, Spot::Name, PLAIN);
    r.wait(0.15).click(1, Spot::Name, PLAIN);
    r.settle();
    let out = r.take();
    assert!(out.contains(&Outcome::Activate {
        id: id(1),
        how: ActivateBy::DoubleClick
    }));
    assert!(!out.iter().any(|o| matches!(o, Outcome::BeginRename { .. })));
}

#[test]
fn a_slow_click_off_the_name_does_not_rename() {
    // operations/rename: A slow click on the icon, row padding, another List column, or a chevron does not rename; in Icons only the caption band does.
    for spot in [Spot::Content, Spot::Gutter] {
        let mut r = Rig::double();
        r.select(&[4]);
        r.wait(1.0).click(4, spot, PLAIN);
        r.settle();
        assert_eq!(r.take(), [Outcome::Clicked { id: id(4) }], "{spot:?}");
    }
}

#[test]
fn a_slow_click_doesnt_arm_with_renaming_off_several_selected_or_a_modifier() {
    // operations/rename: Slow-click rename does not arm in file choosers, in Trash, with more than one item selected, or with a modifier held.
    let mut r = Rig::new(PointerSettings {
        rename: false,
        ..PointerSettings::default()
    });
    r.select(&[4]);
    r.wait(1.0).click(4, Spot::Name, PLAIN);
    assert!(
        !r.m.rename_pending(),
        "a chooser or Trash turns renaming off"
    );

    let mut r = Rig::double();
    r.select(&[4, 5]);
    r.wait(1.0).click(4, Spot::Name, PLAIN);
    assert_eq!(r.selected(), [4]);
    assert!(
        !r.m.rename_pending(),
        "it wasn't the only selection at the press"
    );

    for mods in [
        ALT,
        SHIFT,
        Mods {
            logo: true,
            ..PLAIN
        },
    ] {
        let mut r = Rig::double();
        r.select(&[4]);
        r.wait(1.0).click(4, Spot::Name, mods);
        assert!(!r.m.rename_pending(), "{mods:?}");
    }
}

#[test]
fn a_slow_click_on_an_open_folders_name_renames_instead_of_reopening() {
    // operations/rename: In Columns, clicking the name of the folder whose child column is open opens its editor instead of collapsing the column.
    // browser/view-modes/columns: A slow second click on the name of a selected, open folder still starts a rename.
    let mut r = Rig::columns();
    r.select(&[1]);
    r.wait(1.0).click(1, Spot::Name, PLAIN);
    assert!(
        r.take().contains(&Outcome::Activate {
            id: id(1),
            how: ActivateBy::Click
        }),
        "a folder that isn't open opens"
    );
    r.f.settings.already_open = Some(id(1));
    r.wait(1.0).click(1, Spot::Name, PLAIN);
    r.settle();
    assert_eq!(
        r.take(),
        [
            Outcome::WakeAfter(Duration::from_secs_f64(INTERVAL)),
            Outcome::BeginRename { id: id(1) }
        ]
    );
}

// Modifiers held at the press.

#[test]
fn releasing_shift_before_the_button_keeps_the_range() {
    // browser/selection: Releasing Shift before the mouse button keeps the pressed range, including a press on blank row space in Columns and List.
    for spot in [Spot::Content, Spot::Gutter] {
        let mut r = Rig::double();
        r.click(3, Spot::Content, PLAIN);
        r.wait(1.0).down(6, spot, SHIFT);
        r.up_with(PLAIN);
        assert_eq!(r.selected(), [3, 4, 5, 6], "{spot:?}");
    }
}

#[test]
fn releasing_ctrl_before_the_button_on_blank_row_space_still_adds_the_row() {
    // browser/selection: Releasing Ctrl before the mouse button on blank row space still adds the row.
    let mut r = Rig::double();
    r.select(&[3]);
    r.down(6, Spot::Gutter, CTRL);
    r.up_with(PLAIN);
    assert_eq!(r.selected(), [3, 6]);
}

// Empty space.

#[test]
fn a_plain_press_on_empty_space_clears_the_selection_at_once() {
    // browser/selection: A plain press on blank pane space clears the selection before the drag intent is known.
    let mut r = Rig::double();
    r.select(&[3, 4]);
    r.down_with(Button::Primary, None, Spot::Gutter, PLAIN);
    assert!(r.selected().is_empty());
    r.to(0.0, -40.0);
    assert!(r.take().iter().any(is_marquee));
}

#[test]
fn a_ctrl_or_shift_press_on_empty_space_keeps_the_selection() {
    // browser/selection: A Ctrl or Shift press on blank pane space keeps the selection.
    for mods in [CTRL, SHIFT] {
        let mut r = Rig::double();
        r.select(&[3, 4]);
        r.down_with(Button::Primary, None, Spot::Gutter, mods);
        r.up_with(mods);
        assert_eq!(r.selected(), [3, 4], "{mods:?}");
        assert!(r.take().is_empty());
    }
}

// Middle clicks.

impl Rig {
    fn middle(&mut self, index: usize, release: Mods) {
        self.down_with(Button::Middle, Some(index), Spot::Content, PLAIN);
        self.wait(0.05);
        self.up_with(release);
    }
}

#[test]
fn a_middle_click_on_a_folder_opens_a_tab_or_with_ctrl_or_shift_a_window() {
    // browser/selection: Middle-clicking a folder row in Columns, Icons, or List opens that folder in a new tab.
    // browser/selection: Ctrl+middle-click or Shift+middle-click on a folder row opens that folder in a new window.
    let mut r = Rig::double();
    r.select(&[5]);
    r.middle(1, PLAIN);
    assert_eq!(
        r.take(),
        [Outcome::OpenInNew {
            id: id(1),
            window: false
        }]
    );
    assert_eq!(r.selected(), [5], "opening in new leaves the selection");
    for mods in [CTRL, SHIFT] {
        r.wait(1.0).middle(2, mods);
        assert_eq!(
            r.take(),
            [Outcome::OpenInNew {
                id: id(2),
                window: true
            }],
            "read at the release: {mods:?}"
        );
    }
}

#[test]
fn a_middle_click_on_a_file_selects_it_and_opens_nothing() {
    // browser/selection: Middle-clicking a file row selects the file in its parent folder and opens nothing.
    let mut r = Rig::double();
    r.select(&[1]);
    r.middle(6, PLAIN);
    assert_eq!(r.selected(), [6]);
    assert!(r.take().is_empty());
}

#[test]
fn a_triple_middle_click_opens_one_tab() {
    // browser/selection: A double middle-click on a folder opens one tab; a middle press dragged past the drag threshold opens nothing.
    let mut r = Rig::double();
    for _ in 0..3 {
        r.middle(1, PLAIN);
        r.wait(0.1);
    }
    assert_eq!(
        r.take(),
        [Outcome::OpenInNew {
            id: id(1),
            window: false
        }]
    );
}

#[test]
fn a_double_middle_click_opens_one_tab_and_a_dragged_one_none() {
    // browser/selection: A double middle-click on a folder opens one tab; a middle press dragged past the drag threshold opens nothing.
    let mut r = Rig::double();
    r.middle(1, PLAIN);
    r.wait(0.1).middle(1, PLAIN);
    assert_eq!(
        r.take(),
        [Outcome::OpenInNew {
            id: id(1),
            window: false
        }]
    );
    r.wait(1.0)
        .down_with(Button::Middle, Some(1), Spot::Content, PLAIN);
    r.to(30.0, 0.0);
    r.up_at(Point::new(30.0, 0.0), PLAIN, false);
    assert!(r.take().is_empty());
}

// Secondary presses.

#[test]
fn a_secondary_press_selects_an_unselected_entry_and_keeps_a_selection_it_lands_in() {
    // browser/context-menu: Right-clicking an unselected entry makes it the only selected entry and the keyboard cursor.
    // browser/context-menu: Right-clicking an entry inside a multi-selection keeps the whole selection.
    let mut r = Rig::double();
    r.select(&[3, 4]);
    r.down_with(Button::Secondary, Some(6), Spot::Content, PLAIN);
    assert_eq!(r.selected(), [6]);
    assert_eq!(r.f.sel.cursor(), Some(id(6)));
    assert!(matches!(
        r.take()[..],
        [Outcome::ContextMenu {
            target: Some(t),
            ..
        }] if t == id(6)
    ));
    r.select(&[3, 4]);
    r.down_with(Button::Secondary, Some(4), Spot::Gutter, PLAIN);
    assert_eq!(r.selected(), [3, 4]);
    r.down_with(Button::Secondary, None, Spot::Gutter, PLAIN);
    assert_eq!(r.selected(), [3, 4]);
    assert!(matches!(
        r.take()[..],
        [_, Outcome::ContextMenu { target: None, .. }]
    ));
}

#[test]
fn ctrl_shift_ranges_by_the_setting() {
    // browser/selection: Shift+click selects every entry from the range anchor to the clicked entry.
    // Ctrl+Shift+click: Replace by default, as Strata's `pointer_selection` does; Add when set.
    let mut r = Rig::double();
    r.select(&[8]);
    r.click(3, Spot::Content, CTRL);
    r.wait(1.0).click(
        5,
        Spot::Content,
        Mods {
            shift: true,
            ..CTRL
        },
    );
    assert_eq!(r.selected(), [3, 4, 5]);

    let mut r = Rig::new(PointerSettings {
        ctrl_shift: Extend::Add,
        ..PointerSettings::default()
    });
    r.select(&[8]);
    r.click(3, Spot::Content, CTRL);
    r.wait(1.0).click(
        5,
        Spot::Content,
        Mods {
            shift: true,
            ..CTRL
        },
    );
    assert_eq!(r.selected(), [3, 4, 5, 8]);
}
