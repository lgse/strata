//! The placement rule and the state's transitions.

use std::rc::Rc;

use super::place::{CARD_GAP, MARGIN};
use super::*;
use crate::collection::element::{RestReport, RestView};
use crate::contract::{EntryId, Rect};

/// A viewport at x 200, 600 wide (to 800), in window points.
const VIEWPORT: Rect = Rect::new(200.0, 50.0, 600.0, 500.0);

fn tile(x: f32) -> Rect {
    Rect::new(x, 100.0, 120.0, 96.0)
}

// preview/quick-preview/folder-peek: "The 256 px popover opens 8 px right of the hovered item
// and slides right; when that side lacks room it opens on the left and slides left."
#[test]
fn the_card_opens_right_of_the_item_and_else_left_of_it() {
    let p = place(tile(200.0), VIEWPORT).unwrap();
    assert_eq!((p.side, p.x), (Side::Right, 200.0 + 120.0 + GAP));
    // The card rides an anchor HoverCard puts it 4 pt right of.
    assert_eq!(p.dx, 120.0 + GAP - CARD_GAP);

    let p = place(tile(680.0), VIEWPORT).unwrap();
    assert_eq!((p.side, p.x), (Side::Left, 680.0 - GAP - WIDTH));
}

#[test]
fn the_right_side_needs_rune_s_margin_inside_the_viewport() {
    // Just room by Strata's rule (ends at 800), not with the 8 pt Rune keeps from the window.
    let x = 800.0 - WIDTH - GAP - 120.0;
    assert_ne!(place(tile(x), VIEWPORT).unwrap().side, Side::Right);
    let x = x - MARGIN;
    assert_eq!(place(tile(x), VIEWPORT).unwrap().side, Side::Right);
}

// preview/quick-preview/folder-peek: "When neither side fits, the popover overlays the
// viewport's trailing edge."
#[test]
fn with_no_room_either_side_the_card_covers_the_trailing_edge() {
    let row = Rect::new(200.0, 100.0, 600.0, 24.0);
    let p = place(row, VIEWPORT).unwrap();
    assert_eq!((p.side, p.x), (Side::Over, 800.0 - MARGIN - WIDTH));
    assert_eq!(p.dx, p.x - row.x - CARD_GAP);
}

// preview/quick-preview/folder-peek: "A viewport narrower than 256 px opens no peek."
#[test]
fn a_viewport_narrower_than_the_card_opens_no_peek() {
    let narrow = Rect::new(0.0, 0.0, 250.0, 400.0);
    assert_eq!(place(Rect::new(0.0, 0.0, 250.0, 24.0), narrow), None);
    let fits = Rect::new(0.0, 0.0, 262.0, 400.0);
    assert!(place(Rect::new(0.0, 0.0, 262.0, 24.0), fits).is_some());
}

fn report(id: u64, inside: bool) -> RestReport {
    RestReport {
        collection: Rc::from("files"),
        id: EntryId::new(id),
        inside,
        row: tile(200.0),
        viewport: VIEWPORT,
    }
}

fn on() -> PeekState {
    let mut st = PeekState::new();
    st.set_enabled(true);
    st
}

#[test]
fn a_hover_peek_stays_while_its_folder_or_its_card_holds_the_pointer() {
    let mut st = on();
    assert_eq!(st.row_rest(&report(1, true)), Change::Opened);
    assert!(st.is_anchor("files", EntryId::new(1)));
    assert!(!st.is_anchor("other", EntryId::new(1)));
    // Onto the card before the folder's leave arrives, then the leave.
    assert_eq!(st.card_rest(true), Change::None);
    assert_eq!(st.row_rest(&report(1, false)), Change::None);
    // Back onto the folder, then off the card: still open.
    assert_eq!(st.row_rest(&report(1, true)), Change::None);
    assert_eq!(st.card_rest(false), Change::None);
    // Off both.
    assert_eq!(st.row_rest(&report(1, false)), Change::Closed);
    assert_eq!(st.target(), None);
}

#[test]
fn a_row_under_the_card_opens_nothing_while_the_card_holds_the_pointer() {
    let mut st = on();
    st.row_rest(&report(1, true));
    st.card_rest(true);
    assert_eq!(st.row_rest(&report(2, true)), Change::None);
    assert_eq!(st.target().unwrap().folder, EntryId::new(1));
}

#[test]
fn resting_on_another_folder_replaces_the_peek_with_a_new_serial() {
    let mut st = on();
    st.row_rest(&report(1, true));
    let first = st.target().unwrap().serial;
    assert_eq!(st.row_rest(&report(2, true)), Change::Opened);
    let t = st.target().unwrap();
    assert_eq!((t.folder, t.origin), (EntryId::new(2), Origin::Hover));
    assert!(t.serial > first);
    // The first folder's late leave changes nothing.
    assert_eq!(st.row_rest(&report(1, false)), Change::None);
}

// preview/quick-preview/folder-peek: "Turning Folder peeking off closes an open peek and
// cancels a pending one." (The pending wait ends with the rows' delays: see c22's tests.)
#[test]
fn peeking_off_closes_the_peek_and_hears_no_reports() {
    let mut st = on();
    st.row_rest(&report(1, true));
    assert!(st.set_enabled(false));
    assert_eq!(st.row_rest(&report(2, true)), Change::None);
    assert_eq!(st.target(), None);
    let mut off = PeekState::new();
    assert_eq!(
        off.open(
            "files",
            EntryId::new(1),
            Origin::Keyboard,
            tile(200.0),
            VIEWPORT
        ),
        Change::None,
        "off by default"
    );
}

// integration/10xer-mode: "In Icons and List, `i` on a directory toggles the folder peek
// without moving focus." (The key and the folder check are Strata's.) And "An open folder
// peek closes before any filter, search, highlight, range, or preview step": `close` says
// whether there was one.
#[test]
fn the_keyboard_peek_toggles_ignores_the_pointer_leaving_and_closes_first_on_escape() {
    let mut st = on();
    let id = EntryId::new(1);
    assert_eq!(
        st.toggle("files", id, tile(200.0), VIEWPORT),
        Change::Opened
    );
    assert_eq!(st.target().unwrap().origin, Origin::Keyboard);
    assert_eq!(st.card_rest(false), Change::None);
    assert_eq!(st.row_rest(&report(1, false)), Change::None);
    assert_eq!(
        st.toggle("files", id, tile(200.0), VIEWPORT),
        Change::Closed
    );
    st.toggle("files", id, tile(200.0), VIEWPORT);
    assert!(st.close(), "an Escape step");
    assert!(!st.close(), "nothing left: the next step runs");
}

// Review round 1, B2: rows under the host's overlays (a menu's scrim, a popover, a dialog).
#[test]
fn a_covered_collection_closes_its_peek_and_opens_none() {
    let mut st = on();
    st.row_rest(&report(1, true));
    assert_eq!(st.set_covered(true), Change::Closed);
    assert_eq!(st.row_rest(&report(2, true)), Change::None);
    assert_eq!(st.target(), None);
    assert_eq!(st.set_covered(false), Change::None);
    assert_eq!(st.row_rest(&report(2, true)), Change::Opened);
}

// Review round 1, SF3: a peek whose folder leaves the view closes, a keyboard one too.
#[test]
fn a_peek_whose_folder_leaves_the_view_closes() {
    let view = |collection: &str, ids: &[u64]| RestView {
        collection: Rc::from(collection),
        visible: ids.iter().map(|&i| EntryId::new(i)).collect(),
    };
    let mut st = on();
    st.toggle("files", EntryId::new(1), tile(200.0), VIEWPORT);
    assert_eq!(st.view(&view("files", &[1, 2, 3])), Change::None);
    assert_eq!(
        st.view(&view("other", &[7])),
        Change::None,
        "another collection"
    );
    assert_eq!(st.view(&view("files", &[2, 3, 4])), Change::Closed);
    assert_eq!(st.target(), None);
}

// Review round 2, B1-r2: a peek whose folder's row was drawn moved sideways closes.
#[test]
fn a_peek_whose_anchor_moved_sideways_closes() {
    let mut st = on();
    st.row_rest(&report(1, true));
    let anchor = st.placement().unwrap().x - super::place::CARD_GAP;
    assert_eq!(st.anchor_drawn(anchor), Change::None, "where it opened");
    assert_eq!(st.anchor_drawn(anchor + 0.25), Change::None, "rounding");
    assert_eq!(st.anchor_drawn(anchor - 40.0), Change::Closed);
    assert_eq!(st.target(), None);
}
