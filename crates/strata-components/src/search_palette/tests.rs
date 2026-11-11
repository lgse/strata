use std::cell::Cell;

use rune::{Cx, El, Env, Runtime, Tx};
use strata_kit::look::Look;

use super::*;

thread_local! {
    /// Result rows built on this thread (`view.rs` counts them under test).
    pub(super) static ROW_BUILDS: Cell<usize> = const { Cell::new(0) };
}

fn search() -> PaletteMode {
    PaletteMode::search(&["/home/u"])
}

fn query(p: &Polled) -> PaletteQuery {
    p.intents
        .iter()
        .find_map(|i| match i {
            PaletteIntent::Query(q) => Some(q.clone()),
            _ => None,
        })
        .expect("a query")
}

fn item(id: &str) -> PaletteItem {
    PaletteItem::new(id, id, format!("/home/u/{id}"))
}

fn items(ids: &[&str]) -> Vec<PaletteItem> {
    ids.iter().map(|id| item(id)).collect()
}

fn ids(s: &PaletteState) -> Vec<&str> {
    s.items().iter().map(|it| it.id.as_str()).collect()
}

/// An open search palette with `text` typed, and its query's generation.
fn typed(text: &str) -> (PaletteState, u64) {
    let mut s = PaletteState::new();
    let _ = s.open(search());
    let g = query(&s.set_query(text)).generation;
    (s, g)
}

#[test]
fn opening_asks_for_the_empty_query_and_each_edit_asks_again() {
    let mut s = PaletteState::new();
    let first = query(&s.open(search()));
    assert_eq!(first.text, "");
    let second = query(&s.set_query("no"));
    assert_eq!(second.text, "no");
    assert!(second.generation > first.generation);
}

// browser/search: "On open, the empty query shows 'Type to search Home and mounted local
// drives'; clearing a typed query adds 'Fuzzy matching · try a name or path fragment'."
#[test]
fn an_empty_search_query_is_idle_with_the_first_words_then_the_cleared_ones() {
    let mut s = PaletteState::new();
    let g = query(&s.open(search())).generation;
    let mode = search();
    let idle = mode.idle.clone().unwrap();
    assert_eq!(s.phase(), Some(Phase::Idle(idle.first.clone())));
    assert!(s.busy(), "the walk runs");
    let _ = s.deliver(Answer::new(g, items(&["x"])).done(), &mut Tx::default());
    assert_eq!(
        s.phase(),
        Some(Phase::Idle(idle.first.clone())),
        "no results for ''"
    );
    assert!(!s.busy());
    let g = query(&s.set_query("no")).generation;
    let _ = s.deliver(Answer::new(g, items(&["notes"])).done(), &mut Tx::default());
    assert_eq!(s.phase(), Some(Phase::Results));
    let _ = s.set_query("");
    assert_eq!(s.phase(), Some(Phase::Idle(idle.cleared)));
    assert!(
        s.items().is_empty(),
        "clearing the query clears the results at once"
    );
}

#[test]
fn a_query_loads_then_shows_results_or_no_results() {
    let (mut s, g) = typed("zz");
    assert_eq!(s.phase(), Some(Phase::Loading("Searching…".into())));
    let _ = s.deliver(Answer::new(g, Vec::new()), &mut Tx::default());
    assert_eq!(
        s.phase(),
        Some(Phase::Loading("Searching…".into())),
        "an empty batch with more to come still searches"
    );
    let _ = s.deliver(Answer::new(g, Vec::new()).done(), &mut Tx::default());
    assert_eq!(
        s.phase(),
        Some(Phase::NoResults("No matching files or folders".into()))
    );
}

// browser/search folder jump: "Before any folder is recorded, the palette shows 'No folder
// history yet'"; "A query that matches no visited folder shows 'No matching folders'."
#[test]
fn the_jump_lists_its_empty_query_and_names_its_two_empty_states() {
    let mut s = PaletteState::new();
    let g = query(&s.open(PaletteMode::jump())).generation;
    let _ = s.deliver(Answer::new(g, Vec::new()).done(), &mut Tx::default());
    assert_eq!(
        s.phase(),
        Some(Phase::NoResults("No folder history yet".into()))
    );
    let g = query(&s.set_query("zz")).generation;
    let _ = s.deliver(Answer::new(g, Vec::new()).done(), &mut Tx::default());
    assert_eq!(
        s.phase(),
        Some(Phase::NoResults("No matching folders".into()))
    );
    let g = query(&s.set_query("")).generation;
    let _ = s.deliver(
        Answer::new(g, items(&["a", "b"])).done(),
        &mut Tx::default(),
    );
    assert_eq!(s.phase(), Some(Phase::Results), "an empty jump query lists");
}

#[test]
fn an_answer_for_a_superseded_query_is_dropped_and_the_old_results_stay_until_the_new_land() {
    let (mut s, old) = typed("no");
    let _ = s.deliver(Answer::new(old, items(&["notes"])), &mut Tx::default());
    let new = query(&s.set_query("not")).generation;
    assert_eq!(
        ids(&s),
        ["notes"],
        "the shown results stay while the new query runs"
    );
    let late = s.deliver(
        Answer::new(old, items(&["noise", "nothing"])).done(),
        &mut Tx::default(),
    );
    assert!(!late.changed, "a superseded answer changes nothing");
    assert_eq!(ids(&s), ["notes"]);
    let _ = s.deliver(
        Answer::new(new, items(&["notes", "nothing"])),
        &mut Tx::default(),
    );
    assert_eq!(ids(&s), ["notes", "nothing"]);
    let _ = s.close();
    assert!(
        !s.deliver(Answer::new(new, items(&["x"])), &mut Tx::default())
            .changed,
        "closed"
    );
}

// browser/search: "When the first result is preselected, one Down highlights the second
// result"; "Up and Down move the highlight…".
#[test]
fn a_new_query_highlights_its_first_result_and_arrows_stop_at_the_ends() {
    let (mut s, g) = typed("a");
    let _ = s.deliver(
        Answer::new(g, items(&["a", "b", "c"])).done(),
        &mut Tx::default(),
    );
    assert_eq!(s.highlighted(), Some(0));
    assert!(s.step(1));
    assert_eq!(s.highlighted(), Some(1));
    assert!(s.step(1) && s.step(1));
    assert_eq!(s.highlighted(), Some(2), "Down stops at the last");
    assert!(s.step(-1) && s.step(-1) && s.step(-1));
    assert_eq!(s.highlighted(), Some(0), "Up stops at the first");
    let (mut empty, _) = typed("zz");
    assert!(!empty.step(1), "nothing to move through");
}

// browser/search: "While indexing adds results, the highlighted result stays selected by
// path and unchanged rows are not rebuilt."
#[test]
fn a_same_query_batch_keeps_the_highlight_on_its_result() {
    let (mut s, g) = typed("a");
    let _ = s.deliver(Answer::new(g, items(&["a", "b", "c"])), &mut Tx::default());
    assert!(s.step(1) && s.step(1));
    assert_eq!(s.highlighted_item().unwrap().id, "c");
    let _ = s.deliver(
        Answer::new(g, items(&["new", "a", "b", "c"])),
        &mut Tx::default(),
    );
    assert_eq!(
        s.highlighted_item().unwrap().id,
        "c",
        "by id, at its new place"
    );
    let _ = s.deliver(Answer::new(g, items(&["new", "a"])), &mut Tx::default());
    assert_eq!(s.highlighted(), Some(1), "gone: its old place, clamped");
    // Strata compares the trimmed text: a trailing space is the same query.
    let g = query(&s.set_query("a ")).generation;
    assert!(s.step(-1));
    let _ = s.deliver(Answer::new(g, items(&["new", "a"])), &mut Tx::default());
    assert_eq!(s.highlighted(), Some(0), "the same trimmed query keeps it");
    let g = query(&s.set_query("ab")).generation;
    assert!(s.step(1));
    let _ = s.deliver(Answer::new(g, items(&["new", "a"])), &mut Tx::default());
    assert_eq!(
        s.highlighted(),
        Some(0),
        "a changed query starts on the first"
    );
}

#[test]
fn enter_opens_the_highlighted_result_and_closes() {
    let (mut s, g) = typed("a");
    let _ = s.deliver(
        Answer::new(g, items(&["a", "b"])).done(),
        &mut Tx::default(),
    );
    assert!(s.step(1));
    let mut tx = Tx::default();
    let p = s.open_highlighted(false, &mut tx);
    assert_eq!(
        p.intents,
        [
            PaletteIntent::Open {
                id: "b".into(),
                new_tab: false,
                via: Via::Key
            },
            PaletteIntent::Closed { by: By::Open }
        ]
    );
    assert!(!s.is_open());
    assert!(s.items().is_empty());
}

#[test]
fn enter_with_nothing_to_open_does_nothing() {
    let (mut s, _) = typed("zz");
    let mut tx = Tx::default();
    assert_eq!(s.open_highlighted(false, &mut tx), Polled::default());
    assert!(s.is_open());
}

#[test]
fn the_toggle_closes_whatever_mode_is_open() {
    let mut s = PaletteState::new();
    let _ = s.toggle(search());
    let p = s.toggle(PaletteMode::jump());
    assert_eq!(p.intents, [PaletteIntent::Closed { by: By::Toggle }]);
    assert!(!s.is_open());
    let _ = s.toggle(PaletteMode::jump());
    assert_eq!(
        s.mode().map(|m| m.field_label.as_str()),
        Some("Jump to a folder…")
    );
}

#[test]
fn a_failed_query_says_so_until_the_next_edit() {
    let (mut s, g) = typed("a");
    assert!(!s.fail(g + 1, "nope"), "another generation");
    assert!(s.fail(g, "The index could not be read."));
    assert_eq!(
        s.phase(),
        Some(Phase::Error("The index could not be read.".into()))
    );
    assert!(!s.busy());
    let _ = s.set_query("ab");
    assert_eq!(s.phase(), Some(Phase::Loading("Searching…".into())));
}

#[test]
fn an_unavailable_palette_drops_every_answer_until_it_closes() {
    let mut s = PaletteState::new();
    let g = query(&s.open(search())).generation;
    assert!(s.unavailable("No local search locations available."));
    assert!(
        !s.deliver(Answer::new(g, items(&["a"])).done(), &mut Tx::default())
            .changed
    );
    assert_eq!(
        s.phase(),
        Some(Phase::Unavailable(
            "No local search locations available.".into()
        ))
    );
    assert!(!s.busy());
    let _ = s.close();
    let _ = s.open(search());
    assert!(matches!(s.phase(), Some(Phase::Idle(_))));
}

// The count assistive technology hears (owner ruling): at a query's first results and when
// it is done, not on every batch.
#[test]
fn the_count_is_announced_at_the_first_results_and_when_done() {
    let (mut s, g) = typed("a");
    let _ = s.deliver(Answer::new(g, Vec::new()), &mut Tx::default());
    assert_eq!(s.announced(), None);
    let _ = s.deliver(Answer::new(g, items(&["a", "b"])), &mut Tx::default());
    assert_eq!(s.announced(), Some(2));
    let _ = s.deliver(Answer::new(g, items(&["a", "b", "c"])), &mut Tx::default());
    assert_eq!(s.announced(), Some(2), "not every batch");
    let _ = s.deliver(
        Answer::new(g, items(&["a", "b", "c", "d"])).done(),
        &mut Tx::default(),
    );
    assert_eq!(s.announced(), Some(4));
    let _ = s.set_query("ab");
    assert_eq!(s.announced(), None, "a new query starts over");
}

#[test]
fn partial_causes_follow_the_shown_answer() {
    let (mut s, g) = typed("a");
    let _ = s.deliver(
        Answer::new(g, items(&["a"]))
            .done()
            .partial("Unreadable: ~/x"),
        &mut Tx::default(),
    );
    assert_eq!(s.partial(), Some("Unreadable: ~/x"));
    let _ = s.deliver(Answer::new(g, items(&["a"])).done(), &mut Tx::default());
    assert_eq!(s.partial(), None, "complete results clear it");
}

#[test]
fn a_right_click_highlights_its_result_and_asks_for_the_menu() {
    let (mut s, g) = typed("a");
    let _ = s.deliver(
        Answer::new(g, items(&["a", "b"])).done(),
        &mut Tx::default(),
    );
    let p = s.menu("b", crate::contract::Point::new(3.0, 4.0));
    assert_eq!(s.highlighted(), Some(1));
    assert!(matches!(
        &p.intents[..],
        [PaletteIntent::ResultMenu { id, .. }] if id == "b"
    ));
    let mut tx = Tx::default();
    let p = s.reveal("b", &mut tx);
    assert_eq!(
        p.intents,
        [
            PaletteIntent::Reveal { id: "b".into() },
            PaletteIntent::Closed { by: By::Reveal }
        ]
    );
}

struct Host {
    look: Look,
    palette: PaletteState,
}

fn host_view(h: &Host, _cx: &mut Cx<Host>) -> El {
    let mut layer = Vec::new();
    SearchPalette::new("p", |h: &Host| &h.palette, |h: &mut Host| &mut h.palette)
        .build(&h.look, h, &mut layer);
    rune_kit::root(rune::col().w_full().h_full(), layer)
}

fn builds(rt: &mut Runtime<Host>, f: impl FnOnce(&mut Host)) -> usize {
    let before = ROW_BUILDS.with(Cell::get);
    rt.update(|h, _| f(h));
    rt.frame(1.0 / 60.0);
    ROW_BUILDS.with(Cell::get) - before
}

// browser/search: "While indexing adds results, the highlighted result stays selected by
// path and unchanged rows are not rebuilt."
#[test]
fn rows_build_again_only_when_their_result_or_highlight_changes() {
    let mut palette = PaletteState::new();
    let g = query(&palette.open(search())).generation;
    let _ = palette.set_query("a");
    let g = g + 1;
    let host = Host {
        look: Look::fallback(false),
        palette,
    };
    let mut rt = Runtime::new(host, host_view, (960.0, 640.0), Env::new(false, 1.0));
    rt.frame(0.0);
    let first = builds(&mut rt, |h| {
        let _ = h.palette.deliver(
            Answer::new(g, items(&["a", "b", "c", "d"])),
            &mut Tx::default(),
        );
    });
    assert_eq!(first, 4);
    let grown = builds(&mut rt, |h| {
        let _ = h.palette.deliver(
            Answer::new(g, items(&["new", "a", "b", "c", "d", "e"])),
            &mut Tx::default(),
        );
    });
    assert_eq!(
        grown, 2,
        "only the new rows; the highlight stays on a, unchanged"
    );
    let stepped = builds(&mut rt, |h| assert!(h.palette.step(1)));
    assert_eq!(stepped, 2, "the row left and the row reached");
    let mut changed = items(&["new", "a", "b", "c", "d", "e"]);
    changed[3].matched = std::iter::once(0..1).collect();
    let restyled = builds(&mut rt, |h| {
        let _ = h
            .palette
            .deliver(Answer::new(g, changed), &mut Tx::default());
    });
    assert_eq!(restyled, 1, "a result whose match changed");
}

mod perf;

// Review round 1 (B1, B2, SF1): how each answer scrolls the list, as Strata's restore_scroll
// does with its navigation_started.
#[test]
fn answers_scroll_as_strata_restores_them() {
    use super::model::{Follow, Model};
    let mode = search();
    let mut m = Model::default();
    let g = m.request("a", &mode).generation;
    let first = Answer::new(g, items(&["b0", "b1", "b2", "b3"]));
    assert_eq!(
        m.arrive(first, &mode),
        Some(Follow::Stay),
        "a new list mounts at its top"
    );
    let above = Answer::new(g, items(&["a0", "b0", "b1", "b2", "b3"]));
    assert_eq!(
        m.arrive(above.clone(), &mode),
        Some(Follow::Keep),
        "untouched: keep the offset"
    );
    assert_eq!(
        m.arrive(above, &mode),
        Some(Follow::Stay),
        "nothing changed"
    );
    assert!(m.step(1) && m.step(1));
    let pushed = Answer::new(g, items(&["a0", "a1", "a2", "b0", "b1", "b2", "b3"]));
    assert_eq!(
        m.arrive(pushed, &mode),
        Some(Follow::Reveal(5)),
        "moved: keep b2 in view"
    );
    let g = m.request("ab", &mode).generation;
    let other = Answer::new(g, items(&["c0", "c1"]));
    assert_eq!(
        m.arrive(other, &mode),
        Some(Follow::Top),
        "a new query goes to the top"
    );
    assert_eq!(m.arrive(Answer::new(g + 1, Vec::new()), &mode), None);
}
