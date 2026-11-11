//! Unit tests for C12: the text layer's geometry, the selection, the request queue and find.
//! The view's routing (presses, keys, wheels, focus, what assistive technology hears) is
//! tested through the gallery (`crates/strata-gallery/tests/c12.rs` and the scripts).

use std::rc::Rc;

use super::find::Find;
use super::layer::Layer;
use super::model::{PageReply, PdfDocument, RenderCaps, RenderKind, Shared, TextLayer, Ticket};
use super::queue::{Landed, Queue, Want};
use super::selection::{Selection, Unit};
use super::state::PdfState;
use crate::preview_drawer::HostedPreview;
use rune::Tx;

mod perf;

/// A layer of `lines`, each character a 10 × 16 box on a 20-point line, from (10, 10), every
/// line ending in a break with an empty box.
pub(super) fn layer_of(lines: &[&str]) -> TextLayer {
    let mut text = String::new();
    let mut glyphs = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let top = 10.0 + n as f32 * 20.0;
        for (c, ch) in line.chars().enumerate() {
            let x = 10.0 + c as f32 * 10.0;
            text.push(ch);
            glyphs.push([x, top, x + 10.0, top + 16.0]);
        }
        let end = 10.0 + line.chars().count() as f32 * 10.0;
        text.push('\n');
        glyphs.push([end, top, end, top]);
    }
    TextLayer::new(400.0, 600.0, text, glyphs)
}

fn shared(lines: &[&str]) -> Shared {
    Rc::new(Layer::new(layer_of(lines)))
}

// ---- The layer ----

#[test]
fn an_empty_line_takes_the_height_of_the_line_above() {
    let l = Layer::new(layer_of(&["one", "", "two"]));
    let lines = l.lines();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1].top, lines[0].bottom);
    assert!(lines[1].bottom > lines[1].top, "a caret can land on it");
}

#[test]
fn presses_hit_text_near_ink_and_miss_the_margins() {
    let l = Layer::new(layer_of(&["hello world"]));
    assert!(l.hit_text(30.0, 18.0), "on a glyph");
    assert!(l.hit_text(125.0, 18.0), "a line height past the end");
    assert!(!l.hit_text(300.0, 18.0), "far past the end: a pan");
    assert!(!l.hit_text(30.0, 80.0), "below the text: a pan");
}

#[test]
fn the_caret_lands_between_glyphs_and_before_the_break() {
    let l = Layer::new(layer_of(&["abc", "def"]));
    assert_eq!(l.caret_at(12.0, 18.0), 0, "the left half of a");
    assert_eq!(l.caret_at(18.0, 18.0), 1, "the right half of a");
    assert_eq!(l.caret_at(300.0, 18.0), 3, "past the end: before the break");
    assert_eq!(l.caret_at(300.0, 38.0), 7, "the second line's end");
}

#[test]
fn a_word_is_a_run_of_non_blanks_and_a_line_leaves_its_break_out() {
    let l = Layer::new(layer_of(&["copy the text", "next"]));
    assert_eq!(l.word_range(6), (5, 8));
    assert_eq!(l.word_range(4), (4, 4), "a blank selects nothing");
    assert_eq!(l.line_range(6), (0, 13));
    assert_eq!(l.text_of(0, 13), "copy the text");
}

// preview/preview-panel/pdf: "Highlight bands cover each line's box, but stop short below the
// baseline on runs without descender glyphs."
#[test]
fn bands_stop_short_on_runs_without_descenders() {
    let l = Layer::new(layer_of(&["mmm", "ggg"]));
    let runs = l.selection_runs(0, 7);
    assert_eq!(runs.len(), 2);
    let (plain, low) = (runs[0], runs[1]);
    assert!(plain[3] - plain[1] < low[3] - low[1], "{plain:?} {low:?}");
    assert_eq!(low[3], 46.0, "a descender run takes the whole box");
}

#[test]
fn find_folds_case_does_not_overlap_and_counts_characters() {
    let l = Layer::new(layer_of(&["éclair ÉCLAIR", "aaa"]));
    assert_eq!(l.find("éclair"), vec![(0, 6), (7, 13)]);
    assert_eq!(
        l.find("aa"),
        vec![(14, 16)],
        "\"aa\" in \"aaa\" counts once"
    );
}

// ---- The selection ----

fn layers(n: usize) -> Vec<Shared> {
    (0..n)
        .map(|p| {
            let a = format!("page {p} first line");
            let b = format!("page {p} last line");
            shared(&[a.as_str(), b.as_str()])
        })
        .collect()
}

#[test]
fn a_drag_to_a_later_page_selects_every_page_between() {
    let ls = layers(4);
    let get = |p: usize| ls.get(p);
    let mut s = Selection::default();
    s.press(&ls[0], 0, 7, Unit::Char);
    assert!(s.extend(get, 3, 4));
    let pages: Vec<usize> = s.ranges.keys().copied().collect();
    assert_eq!(pages, [0, 1, 2, 3]);
    assert_eq!(s.ranges[&1], (0, ls[1].len()), "a page between is whole");
    assert!(
        s.text(get)
            .starts_with("first line\npage 0 last line\npage 1")
    );
}

#[test]
fn a_selection_extends_backwards_by_its_unit_and_skips_pages_without_text() {
    let ls = layers(3);
    let get = |p: usize| if p == 1 { None } else { ls.get(p) };
    let mut s = Selection::default();
    // A double-click on "last" of page 2, then a drag back into page 0's "first".
    s.press(&ls[2], 2, 26, Unit::Word);
    assert_eq!(s.text(get), "last");
    s.extend(get, 0, 9);
    assert!(!s.ranges.contains_key(&1), "a scan has nothing to select");
    let text = s.text(get);
    assert!(text.starts_with("first line"), "{text}");
    assert!(
        text.ends_with("page 2 last"),
        "the anchor word stays whole: {text}"
    );
}

#[test]
fn a_triple_click_selects_a_line_without_its_break() {
    let ls = layers(1);
    let mut s = Selection::default();
    s.press(&ls[0], 0, 3, Unit::Line);
    assert_eq!(s.text(|p| ls.get(p)), "page 0 first line");
}

#[test]
fn copied_pages_are_joined_by_one_line_break() {
    let ls = layers(2);
    let get = |p: usize| ls.get(p);
    let mut s = Selection::default();
    s.all(ls.iter().enumerate());
    assert_eq!(
        s.text(get),
        "page 0 first line\npage 0 last line\npage 1 first line\npage 1 last line\n"
    );
}

// ---- The queue ----

fn doc(n: usize) -> PdfDocument {
    PdfDocument::new(n, [600.0, 800.0])
}

fn want(visible: std::ops::Range<usize>, centre: usize) -> Want {
    Want {
        visible,
        centre,
        forward: true,
        width_px: 500.0,
        settled: true,
        text_lane: false,
        search: 0,
    }
}

fn reply(width: u32, text: Option<&str>) -> PageReply {
    let height = width * 4 / 3;
    PageReply::Rendered {
        width,
        height,
        rgba: vec![255; (width * height * 4) as usize],
        text: text.map(|t| layer_of(&[t])),
    }
}

fn queue(n: usize) -> Queue {
    let mut q = Queue::default();
    q.reset(n);
    q
}

fn drain(q: &mut Queue, d: &PdfDocument, w: &Want) -> Vec<(usize, RenderKind)> {
    std::iter::from_fn(|| q.pop(d, w))
        .map(|r| (r.page, r.kind))
        .collect()
}

// preview/preview-panel/pdf: "PDF pages render at the preview pane's width, at most 1280 px".
#[test]
fn visible_pages_ask_from_the_centre_out_then_the_next_page_at_the_drawn_width() {
    let d = doc(20);
    let mut q = queue(20);
    let w = want(4..8, 6);
    let first = q.pop(&d, &w).expect("a request");
    assert_eq!((first.page, first.box_px), (6, [500, 1800]));
    let rest: Vec<usize> = drain(&mut q, &d, &w).into_iter().map(|r| r.0).collect();
    assert_eq!(
        rest,
        [5, 7, 4, 8],
        "then out from the centre, then the margin page"
    );
}

#[test]
fn the_renderer_caps_the_width_asked() {
    let d = doc(1);
    let mut q = queue(1);
    let w = Want {
        width_px: 5000.0,
        ..want(0..1, 0)
    };
    let r = q.pop(&d, &w).expect("a request");
    let caps = RenderCaps::default();
    // 2.5 MP at 4:3 is 1369 wide; 1400 and 1800 tall allow more.
    assert_eq!(r.box_px[0], caps.width_for(5000.0, 800.0 / 600.0));
    assert!(r.box_px[0] as u64 * (r.box_px[0] as u64 * 4 / 3) <= caps.max_pixels);
}

// preview/preview-panel/pdf: "Queued PDF pages that leave the view are cancelled before
// rendering."
#[test]
fn pages_that_leave_the_view_are_never_asked_and_their_requests_are_cancelled() {
    let d = doc(100);
    let mut q = queue(100);
    let near = want(0..3, 1);
    let started = q.pop(&d, &near).expect("a request");
    // Scrolled far away before the renderer was free for the rest.
    let far = want(60..63, 61);
    assert!(q.reconcile(&d, &far));
    assert_eq!(q.take_cancelled(), vec![started.ticket]);
    let asked: Vec<usize> = drain(&mut q, &d, &far).into_iter().map(|r| r.0).collect();
    assert_eq!(asked, [61, 60, 62, 63]);
}

#[test]
fn a_reply_from_an_earlier_document_or_older_than_the_held_image_is_dropped() {
    let mut d = doc(3);
    let mut q = queue(3);
    let w = want(0..1, 0);
    let old = q.pop(&d, &w).unwrap();
    q.reset(3);
    assert_eq!(
        q.complete(&mut d, old.ticket, reply(500, Some("x"))),
        Landed::Dropped
    );

    let a = q.pop(&d, &w).unwrap();
    // The zoom moved on: the page is asked again before the first reply lands.
    let wider = Want {
        width_px: 900.0,
        ..w.clone()
    };
    assert!(q.reconcile(&d, &wider));
    let b = q.pop(&d, &wider).unwrap();
    assert_eq!(
        q.complete(&mut d, b.ticket, reply(900, Some("x"))),
        Landed::Shown
    );
    assert_eq!(
        q.complete(&mut d, a.ticket, reply(500, Some("x"))),
        Landed::Dropped
    );
    assert_eq!(q.slots[0].held.map(|h| h.width), Some(900));
}

#[test]
fn a_sharper_image_lands_in_place_and_a_text_only_reply_never_replaces_an_image() {
    let mut d = doc(1);
    let mut q = queue(1);
    let w = want(0..1, 0);
    let a = q.pop(&d, &w).unwrap();
    assert_eq!(
        q.complete(&mut d, a.ticket, reply(500, Some("x"))),
        Landed::Shown
    );
    let wider = Want {
        width_px: 900.0,
        ..w.clone()
    };
    let b = q.pop(&d, &wider).unwrap();
    assert_eq!(
        q.complete(&mut d, b.ticket, reply(900, Some("x"))),
        Landed::Replaced,
        "the same text: no commit"
    );
    let t = Ticket {
        kind: RenderKind::TextOnly,
        serial: b.ticket.serial + 1,
        ..b.ticket
    };
    q.complete(&mut d, t, reply(64, Some("x")));
    assert_eq!(q.slots[0].held.map(|h| h.width), Some(900));
}

// The work package: "zoom triggers re-render requests for visible pages only".
#[test]
fn a_rested_zoom_asks_again_for_visible_pages_only() {
    let mut d = doc(10);
    let mut q = queue(10);
    let w = want(2..5, 3);
    while let Some(r) = q.pop(&d, &w) {
        q.complete(&mut d, r.ticket, reply(500, None));
    }
    let zooming = Want {
        width_px: 1000.0,
        settled: false,
        ..w.clone()
    };
    assert!(
        q.pop(&d, &zooming).is_none(),
        "nothing while the zoom moves"
    );
    let rested = Want {
        settled: true,
        ..zooming
    };
    let mut asked: Vec<usize> = drain(&mut q, &d, &rested)
        .into_iter()
        .map(|r| r.0)
        .collect();
    asked.sort();
    assert_eq!(asked, [2, 3, 4], "the margin page holds an image already");
}

#[test]
fn find_asks_for_unknown_text_after_the_pages_shown() {
    let d = doc(6);
    let mut q = queue(6);
    let w = Want {
        text_lane: true,
        search: 1,
        ..want(2..3, 2)
    };
    let asked = drain(&mut q, &d, &w);
    assert_eq!(asked[0], (2, RenderKind::Page));
    assert_eq!(asked[1], (3, RenderKind::Page), "the margin page");
    let lane: Vec<usize> = asked[2..].iter().map(|r| r.0).collect();
    assert_eq!(lane, [1, 0, 4, 5]);
    assert!(asked[2..].iter().all(|r| r.1 == RenderKind::TextOnly));
}

#[test]
fn the_budgets_drop_far_pages_first_and_keep_selected_layers() {
    let mut d = doc(10);
    let mut q = queue(10);
    for p in 0..10 {
        let w = want(p..p + 1, p);
        let r = q.pop(&d, &w).unwrap();
        q.complete(&mut d, r.ticket, reply(500, Some("text")));
    }
    let one = 500 * 666 * 4;
    q.image_budget = 4 * one;
    q.text_budget = 0;
    let w = want(8..9, 8);
    assert!(q.enforce(&w, |p| p == 0));
    assert!(q.image_bytes() <= q.image_budget);
    let held: Vec<usize> = (0..10).filter(|&p| q.slots[p].held.is_some()).collect();
    assert!(
        held.contains(&8) && held.contains(&7) && held.contains(&9),
        "{held:?}"
    );
    assert!(!held.contains(&0), "the farthest went first: {held:?}");
    assert!(
        q.slots[0].layer.is_some(),
        "a selected page keeps its layer"
    );
    assert!(q.slots[1].layer.is_none());
}

// ---- Find ----

#[test]
fn matches_join_in_page_order_and_the_current_one_stays() {
    let ls = layers(3);
    let mut f = Find::default();
    f.run("last", [(2, &ls[2])].into_iter());
    assert_eq!(f.current_hit().map(|h| h.page), Some(2));
    assert!(!f.add(0, Some(&ls[0])), "already had a current match");
    assert_eq!(
        f.current_hit().map(|h| h.page),
        Some(2),
        "it stays the same match"
    );
    assert_eq!(f.hits.iter().map(|h| h.page).collect::<Vec<_>>(), [0, 2]);
    assert_eq!(f.step(true).map(|h| h.page), Some(0), "wrapping at the end");
}

#[test]
fn the_first_match_to_arrive_becomes_current() {
    let ls = layers(2);
    let mut f = Find::default();
    f.run("first", std::iter::empty());
    assert_eq!(f.current, None);
    assert!(f.add(1, Some(&ls[1])));
    assert_eq!(f.current_hit().map(|h| h.page), Some(1));
}

// ---- Review 1 ----

/// A queue for `n` pages, each rendered once at 500 pixels with a layer.
fn rendered(n: usize, d: &mut PdfDocument) -> Queue {
    let mut q = queue(n);
    for p in 0..n {
        let w = want(p..p + 1, p);
        let r = q.pop(d, &w).unwrap();
        q.complete(d, r.ticket, reply(500, Some("text")));
    }
    q
}

// B1: a page whose layer the text budget dropped while it kept its image asks again once it's
// back in view, with a `Page` render, so its layer comes with the image it draws on.
#[test]
fn a_page_back_in_view_without_its_layer_asks_again() {
    let mut d = doc(10);
    let mut q = rendered(10, &mut d);
    q.text_budget = 0;
    assert!(q.enforce(&want(8..9, 8), |_| false));
    assert!(q.slots[0].held.is_some() && q.slots[0].layer.is_none());
    let back = want(0..1, 0);
    let r = q.pop(&d, &back).expect("a request for page 1");
    assert_eq!((r.page, r.kind), (0, RenderKind::Page));
    q.complete(&mut d, r.ticket, reply(500, Some("text")));
    assert!(q.slots[0].layer.is_some(), "its text is back");
    // The margin page too.
    let mut q = rendered(10, &mut d);
    q.text_budget = 0;
    q.enforce(&want(8..9, 8), |_| false);
    let asked: Vec<usize> = drain(&mut q, &d, &want(2..4, 2))
        .into_iter()
        .map(|r| r.0)
        .collect();
    assert_eq!(asked, [2, 3, 4], "the visible pages, then the margin page");
}

// A host may report a render the view cancelled as a failure: that says nothing of the page.
#[test]
fn a_failure_for_a_request_no_longer_wanted_is_dropped() {
    let mut d = doc(20);
    let mut q = queue(20);
    let r = q.pop(&d, &want(0..1, 0)).unwrap();
    q.reconcile(&d, &want(10..11, 10));
    assert_eq!(q.take_cancelled(), vec![r.ticket]);
    assert_eq!(
        q.complete(&mut d, r.ticket, PageReply::Failed),
        Landed::Dropped
    );
    assert!(!q.slots[0].failed);
    let again = q.pop(&d, &want(0..1, 0)).expect("page 1 asks again");
    assert_eq!(again.page, 0);
}

// Strata asks again when a row is bound again: a failed page back in view asks once more.
#[test]
fn a_failed_page_asks_once_more_when_it_comes_back() {
    let mut d = doc(20);
    let mut q = queue(20);
    let w = want(0..1, 0);
    let r = q.pop(&d, &w).unwrap();
    assert_eq!(
        q.complete(&mut d, r.ticket, PageReply::Failed),
        Landed::Shown
    );
    assert!(
        q.pop(&d, &w).is_some_and(|r| r.page == 1),
        "the margin, not page 1 again"
    );
    q.take_cancelled();
    let mut q2 = queue(20);
    let r = q2.pop(&d, &w).unwrap();
    q2.complete(&mut d, r.ticket, PageReply::Failed);
    assert!(
        drain(&mut q2, &d, &want(0..1, 0)).iter().all(|r| r.0 != 0),
        "not while it stays"
    );
    q2.reconcile(&d, &want(10..11, 10));
    q2.take_cancelled();
    assert!(q2.reconcile(&d, &w), "back in view");
    let r = q2.pop(&d, &w).expect("asked once more");
    assert_eq!(r.page, 0);
    q2.complete(&mut d, r.ticket, PageReply::Failed);
    assert!(drain(&mut q2, &d, &w).iter().all(|r| r.0 != 0), "once");
}

// R5: a page whose text doesn't end in a line break is still parted from the next by one.
#[test]
fn pages_whose_text_ends_without_a_break_are_parted_by_one() {
    let bare = |t: &str| {
        let glyphs = (0..t.chars().count())
            .map(|i| [i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, 16.0])
            .collect();
        Rc::new(Layer::new(TextLayer::new(400.0, 600.0, t, glyphs)))
    };
    let ls = [bare("one"), bare("two")];
    let mut s = Selection::default();
    s.all(ls.iter().enumerate());
    assert_eq!(s.text(|p| ls.get(p)), "one\ntwo");
}

// Q3: scrolling back, the margin page is the one before the view.
#[test]
fn scrolling_back_the_margin_page_is_the_one_before() {
    let d = doc(20);
    let mut q = queue(20);
    let back = Want {
        forward: false,
        ..want(5..7, 5)
    };
    let asked: Vec<usize> = drain(&mut q, &d, &back).into_iter().map(|r| r.0).collect();
    assert_eq!(asked, [5, 6, 4]);
}

// Q8: no budget drops what's on screen or the page either side.
#[test]
fn the_budgets_never_drop_the_visible_pages_or_the_margin() {
    let mut d = doc(10);
    let mut q = rendered(10, &mut d);
    q.image_budget = 0;
    q.text_budget = 0;
    q.enforce(&want(4..6, 4), |_| false);
    // The visible pages, and the page on either side (the margin either way it scrolls).
    for p in [3, 4, 5, 6] {
        assert!(
            q.slots[p].held.is_some() && q.slots[p].layer.is_some(),
            "page {p}"
        );
    }
    assert!(q.slots[2].held.is_none() && q.slots[7].held.is_none());
}

// Strata's `select_all` leaves the anchor: a Shift-click after Ctrl+A extends from the last
// press, not from the document's start.
#[test]
fn select_all_keeps_the_anchor() {
    let ls = layers(3);
    let get = |p: usize| ls.get(p);
    let mut s = Selection::default();
    s.press(&ls[1], 1, 5, Unit::Char);
    s.all(ls.iter().enumerate());
    assert_eq!(s.ranges.len(), 3);
    s.unit = Unit::Char;
    s.extend(get, 2, 4);
    assert_eq!(s.ranges.keys().copied().collect::<Vec<_>>(), [1, 2]);
    assert!(s.text(get).starts_with("1 first line"), "{}", s.text(get));
}

// ---- Review 2 ----

/// A preview of `n` pages laid out in a 500 × 800 viewport, page 1 visible.
fn state(n: usize) -> PdfState {
    let mut st = PdfState::new("pdf");
    st.open(PdfDocument::new(n, [600.0, 800.0]));
    st.viewport = Some([0.0, 0.0, 500.0, 800.0]);
    st.visible = 0..1;
    st
}

/// Pops and answers every request, each page a scan (an image, no text layer). How many.
fn answer_all_as_scans(st: &mut PdfState, limit: usize) -> usize {
    let mut n = 0;
    while let Some(r) = st.pop() {
        st.complete(r.ticket, reply(r.box_px[0], None));
        n += 1;
        if n == limit {
            break;
        }
    }
    n
}

// Review 2's B1: with find open, a page the renderer says has no text (a scan) is asked once,
// not again for each query, nor again and again for one.
#[test]
fn find_asks_a_page_without_text_once() {
    let mut st = state(3);
    st.set_searchable(true);
    st.find("s");
    let first = answer_all_as_scans(&mut st, 50);
    assert!(first <= 4, "each page once (and the margin page): {first}");
    st.find("st");
    assert_eq!(
        answer_all_as_scans(&mut st, 50),
        0,
        "nothing to ask for the next query"
    );
}

// SF1: new work wakes the host only when its last pop found nothing.
#[test]
fn new_work_wakes_a_host_whose_last_pop_found_none() {
    let mut st = state(20);
    answer_all_as_scans(&mut st, 50);
    assert!(st.pop().is_none());
    let wake = st.wake;
    assert!(st.set_visible(10..11), "the view runs to mount the waker");
    assert_eq!(st.wake, wake + 1);
    // The host pops, and a request is out: its own poll takes what comes next.
    assert!(st.pop().is_some());
    let wake = st.wake;
    st.set_visible(15..16);
    assert_eq!(st.wake, wake, "no wake while the host is popping");
}

// The pointer left: text scrolling under where it was doesn't move the I-beam.
#[test]
fn the_pointer_leaving_stops_the_i_beam_following() {
    let mut st = state(1);
    let r = st.pop().unwrap();
    st.complete(r.ticket, reply(400, Some("hello world")));
    let mut tx = Tx::default();
    let z = st.zoom_gen;
    st.page_laid_out(0, z, [0.0, 0.0, 400.0, 600.0], &mut tx);
    assert!(st.hover([30.0, 18.0]), "over text: the I-beam");
    st.leave();
    // The page scrolls up: margin where the pointer was.
    assert!(!st.page_laid_out(0, z, [0.0, -100.0, 400.0, 600.0], &mut tx));
    assert!(st.over_text, "unchanged until the pointer is back");
}

// A zoom asked for one document doesn't carry into the next: Strata opens each at fit.
#[test]
fn a_zoom_asked_before_an_open_is_dropped() {
    let mut st = state(3);
    st.set_level(2.0);
    st.open(PdfDocument::new(3, [600.0, 800.0]));
    assert!(!st.apply_level_request(&mut Tx::default()));
    assert_eq!(st.level(), 1.0);
}
