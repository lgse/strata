//! The pointer controller on the collection element, in a headless runtime: what the seam
//! routes to it, its settings from `Collection::settings`, what the host hears, and the presses
//! with modifiers held in process (Rune's scripts can hold them too since `4805095`:
//! `docs/testing.md`, Scripts).
//! Entries 1–3 are folders and 4–10 files; a row's key is `row-{id}`.

use std::cell::RefCell;
use std::rc::Rc;

use rune::runtime::ElementInfo;
use rune::{Cx, El, Env, Key, KeyEvent, Mods, PhysicalInput, PointerButton, Runtime, col};
use strata_kit::look::Look;

use super::super::{ClickCount, ClickModes, PointerIntent, PointerSettings};
use crate::collection::element::{
    Activation, Collection, CollectionState, Ctx, Intent, MarqueeController,
};
use crate::contract::{EntryId, EntryKind, ListEntry, ListSource, Point, Rect};

const W: f32 = 500.0;
const H: f32 = 400.0;
const INTERVAL: f64 = 0.4;

struct Host {
    src: ListSource,
    coll: CollectionState,
    look: Look,
    settings: PointerSettings,
    read_only: bool,
    intents: Vec<Intent>,
    bands: Rc<RefCell<Vec<(Point, Mods)>>>,
}

/// Records the bands the pointer controller hands it, as C1d's controller would start them.
struct Band(Rc<RefCell<Vec<(Point, Mods)>>>, bool);

impl MarqueeController for Band {
    fn begin(&mut self, at: Point, mods: Mods, _cx: &mut Ctx<'_>) {
        self.0.borrow_mut().push((at, mods));
        self.1 = true;
    }
    fn moved(&mut self, _at: Point, _cx: &mut Ctx<'_>) {}
    fn end(&mut self, _cx: &mut Ctx<'_>) {
        self.1 = false;
    }
    fn cancel(&mut self, _cx: &mut Ctx<'_>) -> bool {
        std::mem::replace(&mut self.1, false)
    }
    fn active(&self) -> bool {
        self.1
    }
    fn band(&self) -> Option<Rect> {
        None
    }
}

impl Host {
    fn new(settings: PointerSettings) -> Self {
        let entries = (1..=10)
            .map(|i| {
                let kind = if i <= 3 {
                    EntryKind::Folder
                } else {
                    EntryKind::File
                };
                ListEntry::new(EntryId::new(i), format!("entry {i}"), kind, "")
            })
            .collect();
        let bands = Rc::new(RefCell::new(Vec::new()));
        let mut coll = CollectionState::new();
        coll.controllers.pointer = Some(Box::new(PointerIntent::new()));
        coll.controllers.marquee = Some(Box::new(Band(bands.clone(), false)));
        Self {
            src: ListSource::new(entries),
            coll,
            look: Look::fallback(false),
            settings,
            read_only: false,
            intents: Vec::new(),
            bands,
        }
    }
}

fn collection(s: &Host) -> Collection<Host> {
    let c = Collection::new(
        "list",
        |s: &Host| (&s.src, &s.coll),
        |s: &mut Host| (&s.src, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
    .on_intent(|s: &mut Host, intent, _| s.intents.push(intent))
    .settings(s.settings);
    if s.read_only { c.read_only() } else { c }
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    col().size(W, H).child(collection(s).build(s))
}

/// Folders open on one click, files on two: Strata's Columns default.
fn columns() -> PointerSettings {
    PointerSettings::default().modes(ClickModes::new(ClickCount::Two, ClickCount::One))
}

fn runtime(settings: PointerSettings) -> Runtime<Host> {
    let mut rt = Runtime::new(Host::new(settings), view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn id(i: u64) -> EntryId {
    EntryId::new(i)
}

/// `part` of entry `i`'s row: the row itself (`""`), its `icon` or its `label`.
fn element(rt: &Runtime<Host>, i: u64, part: &str) -> ElementInfo {
    let row = format!("row-{i}");
    rt.elements()
        .into_iter()
        .find(|e| {
            !e.exiting
                && match part {
                    "" => e.key.as_deref() == Some(row.as_str()),
                    _ => e.key.as_deref() == Some(part) && e.path.contains(&row),
                }
        })
        .unwrap_or_else(|| panic!("no {part} on {row}"))
}

/// The middle of `part` of entry `i`'s row; the row's blank space is near its right end.
fn at(rt: &Runtime<Host>, i: u64, part: &str) -> (f32, f32) {
    let [x, y, w, h] = element(rt, i, part).visible;
    match part {
        "" => (x + w - 20.0, y + h / 2.0),
        _ => (x + w / 2.0, y + h / 2.0),
    }
}

fn hold(rt: &mut Runtime<Host>, mods: Mods) {
    rt.physical_input(PhysicalInput::Modifiers(mods));
}

/// Advances the clock by a frame `secs` long.
fn wait(rt: &mut Runtime<Host>, secs: f64) {
    rt.frame(secs);
}

/// Presses and releases on `part` of entry `i`'s row.
fn click(rt: &mut Runtime<Host>, i: u64, part: &str) {
    let (x, y) = at(rt, i, part);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
}

/// The host's poll: runs the controllers' timeouts.
fn tick(rt: &mut Runtime<Host>) {
    rt.update(|s, tx| {
        collection(s).tick(s, tx);
    });
    rt.frame(1.0 / 60.0);
}

fn selected(rt: &Runtime<Host>) -> Vec<u64> {
    let s = &rt.state;
    s.coll
        .selection
        .selected_ids(&s.src)
        .into_iter()
        .map(EntryId::get)
        .collect()
}

fn heard(rt: &mut Runtime<Host>) -> Vec<String> {
    std::mem::take(&mut rt.state.intents)
        .into_iter()
        .map(|i| match i {
            Intent::Activate { id, how } => format!("activate {} {how:?}", id.get()),
            Intent::Clicked { id } => format!("clicked {}", id.get()),
            Intent::OpenInNew { id, window } => format!("new {} window={window}", id.get()),
            Intent::BeginDrag { ids, .. } => format!("drag {}", ids.len()),
            Intent::BeginRename { id } => format!("rename {}", id.get()),
            Intent::ContextMenu { target, .. } => format!("menu {:?}", target.map(EntryId::get)),
            other => format!("{other:?}"),
        })
        .collect()
}

const CTRL: Mods = Mods {
    ctrl: true,
    cmd: true,
    shift: false,
    alt: false,
    logo: false,
};
const SHIFT: Mods = Mods {
    shift: true,
    cmd: false,
    ctrl: false,
    alt: false,
    logo: false,
};
const ALT: Mods = Mods {
    alt: true,
    cmd: false,
    ctrl: false,
    shift: false,
    logo: false,
};

#[test]
fn single_mode_from_the_hosts_settings_opens_a_folder_on_one_click() {
    // browser/selection/click-modes: With Single set, one click on a folder opens it.
    let mut rt = runtime(columns());
    click(&mut rt, 1, "");
    assert!(matches!(
        rt.state.intents[..],
        [Intent::Activate {
            how: Activation::Click,
            ..
        }]
    ));
    assert_eq!(heard(&mut rt), ["activate 1 Click"]);
    wait(&mut rt, 1.0);
    click(&mut rt, 4, "");
    assert_eq!(heard(&mut rt), ["clicked 4"], "a file still needs two");
}

#[test]
fn ctrl_and_shift_clicks_with_single_set_change_the_selection_without_opening() {
    // browser/selection/click-modes: With Single set, Ctrl+click or Shift+click on an entry changes the selection without opening it.
    let mut rt = runtime(
        PointerSettings::default().modes(ClickModes::new(ClickCount::One, ClickCount::One)),
    );
    hold(&mut rt, CTRL);
    click(&mut rt, 2, "label");
    click(&mut rt, 5, "label");
    hold(&mut rt, SHIFT);
    click(&mut rt, 7, "icon");
    hold(&mut rt, Mods::default());
    assert_eq!(selected(&rt), [5, 6, 7]);
    assert!(heard(&mut rt).is_empty());
}

#[test]
fn releasing_shift_before_the_button_keeps_the_range() {
    // browser/selection: Releasing Shift before the mouse button keeps the pressed range, including a press on blank row space in Columns and List.
    for part in ["label", ""] {
        let mut rt = runtime(PointerSettings::default());
        click(&mut rt, 4, "label");
        wait(&mut rt, 1.0);
        let (x, y) = at(&rt, 7, part);
        hold(&mut rt, SHIFT);
        rt.pointer_down(x, y);
        rt.frame(1.0 / 60.0);
        hold(&mut rt, Mods::default());
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(selected(&rt), [4, 5, 6, 7], "on {part:?}");
    }
}

#[test]
fn releasing_ctrl_before_the_button_on_blank_row_space_still_adds_the_row() {
    // browser/selection: Releasing Ctrl before the mouse button on blank row space still adds the row.
    let mut rt = runtime(PointerSettings::default());
    click(&mut rt, 4, "label");
    let (x, y) = at(&rt, 8, "");
    hold(&mut rt, CTRL);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    assert_eq!(selected(&rt), [4], "a drag could still make it a marquee");
    hold(&mut rt, Mods::default());
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
    assert_eq!(selected(&rt), [4, 8]);
}

#[test]
fn ctrl_or_shift_middle_click_on_a_folder_opens_a_window() {
    // browser/selection: Ctrl+middle-click or Shift+middle-click on a folder row opens that folder in a new window.
    // Strata reads the modifiers at the release.
    let mut rt = runtime(PointerSettings::default());
    for (i, mods) in [(1, CTRL), (2, SHIFT)] {
        let (x, y) = at(&rt, i, "");
        rt.button_down(x, y, PointerButton::Middle);
        rt.frame(1.0 / 60.0);
        hold(&mut rt, mods);
        rt.button_up(x, y, PointerButton::Middle);
        rt.frame(1.0 / 60.0);
        hold(&mut rt, Mods::default());
        wait(&mut rt, 1.0);
    }
    assert_eq!(heard(&mut rt), ["new 1 window=true", "new 2 window=true"]);
}

#[test]
fn an_alt_drag_from_an_entry_hands_the_marquee_its_press() {
    // browser/selection/marquee: Holding Alt starts a marquee even when the press lands on an entry.
    let mut rt = runtime(PointerSettings::default());
    let (x, y) = at(&rt, 5, "label");
    hold(&mut rt, ALT);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    hold(&mut rt, Mods::default());
    rt.pointer_move(x, y + 30.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y + 30.0);
    rt.frame(1.0 / 60.0);
    let bands = rt.state.bands.borrow().clone();
    assert_eq!(bands.len(), 1);
    assert!(
        bands[0].1.alt,
        "the band gets the modifiers held at the press"
    );
    assert!(heard(&mut rt).iter().all(|h| !h.starts_with("drag")));
}

#[test]
fn a_drag_from_a_row_starts_with_the_pointers_window_point() {
    // browser/selection/pointer-intent: On an unselected entry, a drag from the icon or rendered filename starts a file drag.
    let mut rt = runtime(PointerSettings::default());
    let (x, y) = at(&rt, 6, "icon");
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x + 4.0, y + 20.0);
    rt.frame(1.0 / 60.0);
    let drag = rt
        .state
        .intents
        .iter()
        .find_map(|i| match i {
            Intent::BeginDrag { ids, window } => Some((ids.clone(), *window)),
            _ => None,
        })
        .expect("a drag");
    assert_eq!(drag.0, [id(6)]);
    assert_eq!(drag.1, Point::new(x + 4.0, y + 20.0));
    rt.pointer_up(x + 4.0, y + 20.0);
    rt.frame(1.0 / 60.0);
}

#[test]
fn a_slow_click_on_the_name_renames_and_a_key_cancels_it() {
    // operations/rename: Clicking the name text of the only selected item again, after the double-click interval, opens the editor.
    // operations/rename: A quick double-click opens the item instead; another click, a drag, any key press, or a selection change before the interval cancels the pending rename.
    let mut rt = runtime(PointerSettings::default());
    click(&mut rt, 4, "label");
    wait(&mut rt, 1.0);
    click(&mut rt, 4, "label");
    wait(&mut rt, INTERVAL + 0.01);
    tick(&mut rt);
    assert_eq!(heard(&mut rt), ["clicked 4", "rename 4"]);

    wait(&mut rt, 1.0);
    click(&mut rt, 4, "label");
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
    wait(&mut rt, INTERVAL + 0.01);
    tick(&mut rt);
    assert!(heard(&mut rt).is_empty(), "the key cancelled the rename");
}

#[test]
fn a_slow_click_on_the_icon_does_not_rename() {
    // operations/rename: A slow click on the icon, row padding, another List column, or a chevron does not rename; in Icons only the caption band does.
    let mut rt = runtime(PointerSettings::default());
    click(&mut rt, 4, "label");
    for part in ["icon", ""] {
        wait(&mut rt, 1.0);
        click(&mut rt, 4, part);
        wait(&mut rt, INTERVAL + 0.01);
        tick(&mut rt);
    }
    assert_eq!(heard(&mut rt), ["clicked 4", "clicked 4", "clicked 4"]);
}

#[test]
fn a_read_only_collection_neither_drags_nor_renames() {
    // No bullet of its own: C14b's read-only archive preview (`Collection::read_only`) takes
    // no file drag and no rename. Choosers and Trash turn renaming off through the settings.
    let mut rt = runtime(PointerSettings::default());
    rt.update(|s, _| s.read_only = true);
    rt.frame(1.0 / 60.0);
    click(&mut rt, 4, "label");
    wait(&mut rt, 1.0);
    click(&mut rt, 4, "label");
    wait(&mut rt, INTERVAL + 0.01);
    tick(&mut rt);
    let (x, y) = at(&rt, 4, "label");
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y + 30.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y + 30.0);
    rt.frame(1.0 / 60.0);
    assert_eq!(heard(&mut rt), ["clicked 4", "clicked 4"]);
    assert!(rt.state.bands.borrow().is_empty(), "nor a band");

    wait(&mut rt, 1.0);
    hold(&mut rt, CTRL);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y + 30.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y + 30.0);
    rt.frame(1.0 / 60.0);
    hold(&mut rt, Mods::default());
    assert_eq!(
        selected(&rt),
        [4],
        "a Ctrl drag from a selected entry doesn't toggle it"
    );
    assert!(heard(&mut rt).is_empty());
    assert!(rt.state.bands.borrow().is_empty());
}

#[test]
fn entering_a_folder_drops_a_held_press_and_a_pending_rename() {
    // operations/rename: A quick double-click opens the item instead; another click, a drag, any key press, or a selection change before the interval cancels the pending rename.
    let mut rt = runtime(PointerSettings::default());
    click(&mut rt, 4, "label");
    wait(&mut rt, 1.0);
    click(&mut rt, 4, "label");
    heard(&mut rt);
    rt.state.coll.controllers.reset();
    wait(&mut rt, INTERVAL + 0.01);
    tick(&mut rt);
    assert!(heard(&mut rt).is_empty());
}

#[test]
fn a_secondary_press_selects_the_entry_and_anchors_the_menu_at_the_press() {
    // browser/context-menu: Right-clicking an unselected entry makes it the only selected entry and the keyboard cursor.
    let mut rt = runtime(PointerSettings::default());
    click(&mut rt, 4, "label");
    let (x, y) = at(&rt, 7, "");
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(1.0 / 60.0);
    assert_eq!(selected(&rt), [7]);
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(7)));
    match rt.state.intents.last() {
        Some(Intent::ContextMenu { target, anchor }) => {
            assert_eq!(*target, Some(id(7)));
            assert_eq!(*anchor, Rect::new(x, y, 0.0, 0.0));
        }
        other => panic!("no menu: {other:?}"),
    }
}
