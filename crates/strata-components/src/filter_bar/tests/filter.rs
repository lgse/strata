use rune::TextEdit;

use crate::collection::selection::{Position, Selection};
use crate::contract::{EntryId, EntryKind, ListEntry, ListSource};
use crate::filter_bar::filter::{FilterIntent, FilterState, Polled, Results, Shows};

fn source(names: &[&str]) -> ListSource {
    ListSource::new(
        names
            .iter()
            .enumerate()
            .map(|(i, n)| ListEntry::new(EntryId::new(i as u64 + 1), *n, EntryKind::File, "file"))
            .collect(),
    )
}

fn id(n: u64) -> EntryId {
    EntryId::new(n)
}

/// An open bar with `text` typed and its results shown, the field focused.
fn typed(st: &mut FilterState, text: &str) -> Polled {
    let _ = st.open();
    st.edit = TextEdit::new(text);
    st.edit.focused = true;
    let out = st.text_changed_for_tests();
    st.shown(text);
    out
}

impl FilterState {
    fn text_changed_for_tests(&mut self) -> Polled {
        self.text_changed(Position::default)
    }
}

fn r<'a>(src: &'a ListSource, sel: &'a Selection) -> Results<'a> {
    Results {
        source: src,
        selection: sel,
    }
}

// browser/search/filter/enter-target: "In Icons and List, Enter in the field with no pick
// opens the outlined top result", and the outline needs the field's keyboard.
#[test]
fn enter_opens_the_top_result_outlined_while_the_field_has_the_keyboard() {
    let src = source(&["report.pdf", "reports", "repo"]);
    let sel = Selection::new();
    let mut st = FilterState::new();
    let _ = typed(&mut st, "rep");
    assert_eq!(st.target(r(&src, &sel)), Some(id(1)));
    assert_eq!(st.outlined(r(&src, &sel)), Some(id(1)));
    st.edit.focused = false;
    assert_eq!(
        st.outlined(r(&src, &sel)),
        None,
        "focus elsewhere removes the outline"
    );
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(1)),
        "Enter's target stays"
    );
}

// "A pick… removes the outline; Enter then opens the pick": the last click, the cursor (and,
// here, the first selected result too: the modes' rules are below).
#[test]
fn after_a_pick_enter_opens_the_pick() {
    let src = source(&["a", "b", "c", "d"]);
    let mut sel = Selection::new();
    let mut st = FilterState::new();
    let _ = typed(&mut st, "x");
    let _ = sel.select_only(&src, 3);
    let _ = sel.toggle(&src, 1);
    st.pick();
    assert_eq!(sel.cursor(), Some(id(2)), "the cursor is on the last click");
    assert_eq!(st.target(r(&src, &sel)), Some(id(2)));
    assert_eq!(st.outlined(r(&src, &sel)), None);
}

// "Deselecting every result after a pick returns the outline and Enter to the top result."
#[test]
fn deselecting_every_result_after_a_pick_returns_the_top_result() {
    let src = source(&["a", "b"]);
    let mut sel = Selection::new();
    let mut st = FilterState::new();
    let _ = typed(&mut st, "x");
    let _ = sel.select_only(&src, 1);
    st.pick();
    assert_eq!(st.target(r(&src, &sel)), Some(id(2)));
    let _ = sel.clear();
    assert_eq!(st.target(r(&src, &sel)), Some(id(1)));
    assert_eq!(st.outlined(r(&src, &sel)), Some(id(1)));
}

// "In search results, editing the query text drops the pick and the results' selection";
// "Editing the query and typing it back… still counts as a query edit"; "In Trash, Recent,
// or a network share… a query edit keeps the selection but drops the pick"; "A same-query
// update… keeps the pick".
#[test]
fn a_query_edit_drops_the_pick_and_a_same_query_update_keeps_it() {
    let mut st = FilterState::new();
    let _ = typed(&mut st, "duck");
    st.pick();
    assert!(!st.shown("duck"), "a same-query update keeps the pick");
    assert!(st.tracker.picked);
    // Typed back: an edit, though the text is the same.
    st.edit = TextEdit::new("duck");
    let _ = st.text_changed_for_tests();
    assert!(
        st.shown("duck"),
        "results replacing the listing drop their selection"
    );
    assert!(!st.tracker.picked);

    let mut st = FilterState::new();
    st.set_shows(Shows::InPlace);
    let _ = typed(&mut st, "duck");
    st.pick();
    st.edit = TextEdit::new("duck ");
    let _ = st.text_changed_for_tests();
    assert!(
        !st.shown("duck "),
        "in place, the listing keeps its selection"
    );
    assert!(!st.tracker.picked, "but the pick goes");
}

// "With 10xer mode on, no result is outlined and a query edit keeps the selected result." In
// place, so Enter needs a selection (search results: the next test but one).
#[test]
fn tenxer_mode_outlines_nothing_and_keeps_the_selection() {
    let src = source(&["a", "b"]);
    let mut sel = Selection::new();
    let mut st = FilterState::new();
    st.set_tenxer(true);
    st.set_shows(Shows::InPlace);
    let _ = typed(&mut st, "a");
    assert_eq!(st.outlined(r(&src, &sel)), None);
    assert_eq!(
        st.target(r(&src, &sel)),
        None,
        "nothing selected, nothing to open"
    );
    let _ = sel.select_only(&src, 1);
    st.edit = TextEdit::new("ab");
    let _ = st.text_changed_for_tests();
    assert!(!st.shown("ab"));
    assert_eq!(st.target(r(&src, &sel)), Some(id(2)));
}

// "While the field has focus, the list keeps the top result scrolled into view as results
// arrive. A button press, wheel scroll… stops that scrolling; the next query edit scrolls
// back to the top result."
#[test]
fn the_top_result_follows_until_the_results_scroll_and_again_after_an_edit() {
    let src = source(&["a", "b"]);
    let sel = Selection::new();
    let mut st = FilterState::new();
    let _ = typed(&mut st, "a");
    assert_eq!(st.take_reveal(r(&src, &sel)), Some(0));
    assert_eq!(st.take_reveal(r(&src, &sel)), None, "once per batch");
    st.results_scrolled();
    st.shown("a");
    assert_eq!(st.take_reveal(r(&src, &sel)), None, "the person scrolled");
    st.edit.text = "ab".into();
    let _ = st.text_changed_for_tests();
    st.shown("ab");
    assert_eq!(st.take_reveal(r(&src, &sel)), Some(0));
}

// "Escape in the filter field, or clearing the query, restores the normal listing", with the
// listing's own selection when results replaced it.
#[test]
fn closing_or_clearing_gives_back_the_listing_and_its_selection() {
    let before = Position {
        cursor: Some(id(7)),
        selected: vec![id(7)],
        ..Position::default()
    };
    let mut st = FilterState::new();
    let out = st.open_with("r", before.clone());
    assert_eq!(out.intents, vec![FilterIntent::Query { text: "r".into() }]);
    let out = st.close();
    assert_eq!(
        out.intents,
        vec![
            FilterIntent::ShowListing {
                restore: Some(before.clone())
            },
            FilterIntent::Closed
        ]
    );
    assert!(!st.is_open());

    let _ = st.open_with("r", before.clone());
    st.edit = TextEdit::new("");
    let out = st.text_changed_for_tests();
    assert_eq!(
        out.intents,
        vec![FilterIntent::ShowListing {
            restore: Some(before.clone())
        }]
    );
    assert!(st.is_open(), "clearing keeps the field open");

    let mut st = FilterState::new();
    st.set_shows(Shows::InPlace);
    let _ = st.open_with("r", before);
    let out = st.close();
    assert_eq!(
        out.intents,
        vec![
            FilterIntent::ShowListing { restore: None },
            FilterIntent::Closed
        ]
    );
}

// An empty filter closing has no listing to give back.
#[test]
fn closing_an_empty_filter_only_closes() {
    let mut st = FilterState::new();
    let _ = st.open();
    assert_eq!(st.close().intents, vec![FilterIntent::Closed]);
    assert_eq!(st.close(), Polled::default());
}

// The outline shows and goes with the field's keyboard: tick reports the change.
#[test]
fn tick_reports_the_field_gaining_and_losing_the_keyboard() {
    let mut st = FilterState::new();
    let _ = st.open();
    assert!(st.tick().changed);
    assert!(!st.tick().changed);
    st.edit.focused = false;
    assert!(st.tick().changed);
    assert!(st.is_open(), "the filter stays open");
    st.focus_field();
    assert!(st.edit.focused);
}

// Review round 1: Strata's `EnterTarget::restart`. After the host dropped the results (a
// scope or a search limit changed), the same text shown again is a new query: the pick goes.
#[test]
fn a_restart_makes_the_same_query_new() {
    let mut st = FilterState::new();
    let _ = typed(&mut st, "duck");
    st.pick();
    st.restart();
    assert!(st.shown("duck"), "the results' selection goes");
    assert!(!st.tracker.picked);
}

// Down lands on the results' cursor after a pick, as Strata's search results do
// (`inline_search.rs`), not on the first selected result; on the top result while it is
// offered.
#[test]
fn down_lands_on_the_cursor_after_a_pick_and_on_the_top_result_before() {
    let src = source(&["a", "b", "c", "d"]);
    let mut sel = Selection::new();
    let mut st = FilterState::new();
    let _ = typed(&mut st, "x");
    let _ = sel.select_only(&src, 1);
    let _ = sel.toggle(&src, 3);
    assert_eq!(
        st.landing(r(&src, &sel)),
        Some(id(1)),
        "no pick: the top result"
    );
    st.pick();
    assert_eq!(
        st.landing(r(&src, &sel)),
        Some(id(4)),
        "the cursor, not the first selected"
    );
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(4)),
        "search results' Enter opens where Down lands"
    );
}

// Owner's ruling on round 1, item 1: Enter after a pick follows Strata per mode. Search
// results that replace the listing open their cursor (`inline_search.rs`, `current_position`),
// as Down lands there; a listing narrowed in place opens its first selected result in display
// order (`filter_activation_position`).
#[test]
fn after_a_pick_enter_in_search_results_opens_the_cursor() {
    let src = source(&["a", "b", "c", "d"]);
    let mut sel = Selection::new();
    let mut st = FilterState::new();
    let _ = typed(&mut st, "x");
    let _ = sel.select_only(&src, 1);
    let _ = sel.toggle(&src, 3);
    st.pick();
    assert_eq!(sel.cursor(), Some(id(4)));
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(4)),
        "the cursor, not the first selected"
    );
    assert_eq!(
        st.target(r(&src, &sel)),
        st.landing(r(&src, &sel)),
        "where Down lands"
    );
}

#[test]
fn after_a_pick_enter_in_place_opens_the_first_selected_result() {
    let src = source(&["a", "b", "c", "d"]);
    let mut sel = Selection::new();
    let mut st = FilterState::new();
    st.set_shows(Shows::InPlace);
    let _ = typed(&mut st, "x");
    let _ = sel.select_only(&src, 1);
    let _ = sel.toggle(&src, 3);
    st.pick();
    assert_eq!(sel.cursor(), Some(id(4)));
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(2)),
        "the first selected, not the cursor"
    );
}

// In 10xer mode nothing is offered as the top result: search results open their cursor, or
// the top result with no cursor (Strata's `current_position().unwrap_or(0)`), selected or not.
#[test]
fn tenxer_search_results_open_the_cursor_or_the_top_result() {
    let src = source(&["a", "b", "c"]);
    let mut sel = Selection::new();
    let mut st = FilterState::new();
    st.set_tenxer(true);
    let _ = typed(&mut st, "x");
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(1)),
        "no cursor: the top result"
    );
    let _ = sel.move_cursor(&src, 2, crate::collection::selection::AnchorRule::Move);
    assert!(sel.is_empty());
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(3)),
        "the cursor, though nothing is selected"
    );
}

// Review round 2: before any results show for the text (inside the host's debounce, or after
// `restart`), nothing is offered as the top result, so Enter follows Strata's per-mode rule at
// once: search results open their cursor, or the top result; in place, the first selected.
#[test]
fn before_any_results_show_enter_follows_the_mode_rule() {
    let src = source(&["a", "b", "c"]);
    let mut sel = Selection::new();
    let mut st = FilterState::new();
    let _ = st.open_with("x", crate::collection::selection::Position::default());
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(1)),
        "search results: the top result"
    );
    let _ = sel.move_cursor(&src, 2, crate::collection::selection::AnchorRule::Move);
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(3)),
        "search results: the cursor"
    );

    let mut st = FilterState::new();
    st.set_shows(Shows::InPlace);
    let _ = st.open_with("x", crate::collection::selection::Position::default());
    assert_eq!(st.target(r(&src, &sel)), None, "in place, nothing selected");
    let _ = sel.select_only(&src, 1);
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(2)),
        "in place: the first selected"
    );

    // After a restart the same holds.
    let mut st = FilterState::new();
    let _ = typed(&mut st, "x");
    st.restart();
    assert_eq!(
        st.target(r(&src, &sel)),
        Some(id(2)),
        "the cursor, with no query shown"
    );
}
