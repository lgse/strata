// SPDX-License-Identifier: MIT

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use rune::{Cx, El, Env, Mods, Runtime, Tx, WheelEvent, col, row};
use strata_kit::look::Look;

use crate::collection::element::{Collection, CollectionState};
use crate::collection::selection::{Listing, Position, ScrollMemo};
use crate::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource, ThumbKey};
use crate::icons_grid::{IconsGrid, IconsGridState, IconsParts, SizeCommand, SizeRequest};
use crate::thumbnails::{Outcome, ThumbnailStore, ViewerId};

const DT: f64 = 1.0 / 60.0;
const SCOPE: &str = "grid";

struct Host {
    src: ListSource,
    coll: CollectionState,
    grid: IconsGridState,
    look: Look,
    store: ThumbnailStore,
    viewer: ViewerId,
    thumb_version: u64,
    size: f32,
    details: HashMap<EntryId, Arc<str>>,
    /// Details read while building tiles.
    detail_reads: Rc<Cell<usize>>,
    views: Rc<Cell<usize>>,
    /// Wheels the host's `IconsGrid::on_wheel` heard, and whether it takes them.
    wheels: usize,
    takes_wheels: bool,
    w: f32,
    h: f32,
    /// A second grid over the same size, beside the first.
    second: Option<(CollectionState, IconsGridState)>,
}

fn source(n: usize) -> ListSource {
    ListSource::new(
        (0..n)
            .map(|i| {
                let mut e = ListEntry::new(
                    EntryId::new(i as u64 + 1),
                    format!("img-{i}.jpg"),
                    EntryKind::File,
                    "file_image",
                );
                e.thumbnail = Some(ThumbKey::new(10_000 + i as u64));
                e
            })
            .collect(),
    )
}

fn host(n: usize, w: f32, h: f32) -> Host {
    let mut store = ThumbnailStore::default();
    let viewer = store.viewer();
    Host {
        src: source(n),
        coll: CollectionState::new(),
        grid: IconsGridState::new(),
        look: Look::fallback(false),
        store,
        viewer,
        thumb_version: 1,
        size: 64.0,
        details: HashMap::new(),
        detail_reads: Rc::default(),
        views: Rc::default(),
        wheels: 0,
        takes_wheels: false,
        w,
        h,
        second: None,
    }
}

fn base(_: &Host) -> Collection<Host> {
    Collection::new(
        SCOPE,
        |s: &Host| (&s.src, &s.coll),
        |s: &mut Host| (&s.src, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
}

fn grid(s: &Host) -> IconsGrid<Host> {
    let reads = s.detail_reads.clone();
    IconsGrid::new(
        SCOPE,
        base,
        |s: &Host| IconsParts {
            source: &s.src,
            collection: &s.coll,
            grid: &s.grid,
            size: s.size,
            listing: s.thumb_version,
            look: &s.look,
            thumbs: Some((&s.store, s.viewer)),
        },
        |s: &mut Host| &mut s.grid,
    )
    .thumbs_mut(|s: &mut Host| Some((&mut s.store, s.viewer, &s.src as &dyn EntrySource)))
    .details(
        |s: &Host, rcx| {
            s.details
                .get(&rcx.entry.id)
                .map_or(0, |d| d.len() as u64 + 1)
        },
        move |s: &Host, rcx| {
            reads.set(reads.get() + 1);
            s.details.get(&rcx.entry.id).cloned()
        },
    )
    .on_size(|s: &mut Host, r, _| {
        if let SizeRequest::Step { to, .. } = r {
            s.size = to;
        }
    })
    .on_wheel(|s: &mut Host, _, _| {
        s.wheels += 1;
        s.takes_wheels
    })
}

/// A second grid over the same source and size, with its own states.
fn second(_: &Host) -> IconsGrid<Host> {
    IconsGrid::new(
        "second",
        |_: &Host| {
            Collection::new(
                "second",
                |s: &Host| (&s.src, &s.second.as_ref().expect("second").0),
                |s: &mut Host| (&s.src, &mut s.second.as_mut().expect("second").0),
                |s: &Host| &s.look,
                |_: &Host| 1,
            )
        },
        |s: &Host| {
            let (coll, grid) = s.second.as_ref().expect("second");
            IconsParts {
                source: &s.src,
                collection: coll,
                grid,
                size: s.size,
                listing: 1,
                look: &s.look,
                thumbs: None,
            }
        },
        |s: &mut Host| &mut s.second.as_mut().expect("second").1,
    )
}

fn view(s: &Host, cx: &mut Cx<Host>) -> El {
    s.views.set(s.views.get() + 1);
    let mut r = row().size(s.w, s.h).child(
        col()
            .w(s.w / if s.second.is_some() { 2.0 } else { 1.0 })
            .h_full()
            .child(grid(s).scale(cx.env().scale).build(s)),
    );
    if s.second.is_some() {
        r = r.child(col().w(s.w / 2.0).h_full().child(second(s).build(s)));
    }
    r
}

fn runtime(h: Host, scale: f32) -> Runtime<Host> {
    let (w, hh) = (h.w, h.h);
    let mut rt = Runtime::new(h, view, (w, hh), Env::new(false, scale));
    for _ in 0..4 {
        rt.frame(DT);
    }
    rt
}

/// A worker that finishes every request at once, as a host's poll would hand them over.
fn serve(s: &mut Host) -> usize {
    let mut n = 0;
    while let Some(r) = s.store.pop(0.0) {
        let px = r.step.px();
        s.store.complete(
            r.ticket,
            Outcome::Ready {
                width: px,
                height: px,
                rgba: vec![200; (px * px * 4) as usize],
            },
            0.0,
        );
        n += 1;
    }
    n
}

#[test]
fn a_details_line_landing_rebuilds_its_tile_alone() {
    let mut rt = runtime(host(400, 900.0, 600.0), 1.0);
    let shown = rt.state.coll.visible_range();
    assert!(shown.len() > 8);
    let reads = rt.state.detail_reads.clone();
    reads.set(0);
    let views = rt.state.views.get();
    rt.update(|s: &mut Host, _: &mut Tx| {
        let id = s.src.id_at(shown.start + 3);
        s.details.insert(id, "1.2 MB".into());
    });
    rt.frame(DT);
    assert_eq!(
        rt.state.views.get(),
        views + 1,
        "the host's change commits once"
    );
    assert_eq!(
        reads.get(),
        1,
        "and builds the one tile whose details landed"
    );
}

#[test]
fn thumbnails_landing_run_no_view_and_rebuild_no_tile() {
    let mut rt = runtime(host(400, 900.0, 600.0), 1.0);
    let reads = rt.state.detail_reads.clone();
    reads.set(0);
    let views = rt.state.views.get();
    // Handed over as a host's poll does, which returns false: nothing the view reads changed.
    assert!(serve(&mut rt.state) > 0);
    for _ in 0..10 {
        rt.frame(DT);
    }
    assert_eq!(rt.state.views.get(), views, "no view run");
    assert_eq!(reads.get(), 0, "no tile rebuilt");
}

#[test]
fn two_grids_over_one_size_both_follow_a_step() {
    let mut h = host(200, 1200.0, 600.0);
    h.second = Some((CollectionState::new(), IconsGridState::new()));
    let mut rt = runtime(h, 1.0);
    let per = |rt: &Runtime<Host>| {
        (
            rt.state.coll.per_line(),
            rt.state.second.as_ref().expect("second").0.per_line(),
        )
    };
    let before = per(&rt);
    rt.update(|s: &mut Host, tx: &mut Tx| grid(s).apply_size(s, SizeCommand::Step(6), tx));
    for _ in 0..4 {
        rt.frame(DT);
    }
    let after = per(&rt);
    assert!(
        after.0 < before.0 && after.1 < before.1,
        "{before:?} → {after:?}"
    );
    assert_eq!(after.0, after.1, "both at the one size");
}

#[test]
fn a_scroll_offset_comes_back_only_at_its_grids_width() {
    let mut rt = runtime(host(2_000, 900.0, 600.0), 1.0);
    let key = |s: &Host| IconsGrid::<Host>::layout_key(s.coll.viewport().w, s.size, &s.look);
    let layout = key(&rt.state);
    rt.update(|s: &mut Host, _: &mut Tx| s.coll.set_layout_key(layout));
    // Left at a cursor near the top, scrolled well past it.
    rt.update(|s: &mut Host, _: &mut Tx| {
        s.coll.selection.select_only(&s.src, 2);
    });
    let at = 5_000.0;
    let position = |s: &Host, l: u64| -> Position {
        s.coll
            .selection
            .position(&s.src)
            .with_scroll(ScrollMemo::new(l, at))
    };
    let back = position(&rt.state, layout);
    rt.update(|s: &mut Host, tx: &mut Tx| {
        s.coll
            .enter(SCOPE, &s.src, Some(&back), Listing::Complete, tx);
    });
    for _ in 0..10 {
        rt.frame(DT);
    }
    assert_eq!(
        rt.state.coll.scroll_offset(),
        at,
        "the same width: the offset"
    );
    rt.update(|s: &mut Host, _: &mut Tx| s.w = 400.0);
    for _ in 0..10 {
        rt.frame(DT);
    }
    let other = key(&rt.state);
    assert_ne!(other, layout, "another count a row");
    rt.update(|s: &mut Host, tx: &mut Tx| {
        s.coll.set_layout_key(other);
        s.coll
            .enter(SCOPE, &s.src, Some(&back), Listing::Complete, tx);
    });
    for _ in 0..10 {
        rt.frame(DT);
    }
    assert!(
        rt.state.coll.visible_range().contains(&2),
        "the cursor shows at {}",
        rt.state.coll.scroll_offset()
    );
}

#[test]
fn a_4k_grid_at_scale_2_settles_its_thumbnail_queue() {
    let mut h = host(20_000, 1920.0, 1080.0);
    h.size = 72.0;
    let mut rt = runtime(h, 2.0);
    let mut served = Vec::new();
    for _ in 0..30 {
        served.push(serve(&mut rt.state));
        rt.frame(DT);
    }
    assert!(served[0] > 0);
    assert!(
        served[10..].iter().all(|&n| n == 0),
        "nothing asked again once settled: {served:?}"
    );
    let st = rt.state.store.stats();
    assert!(st.bytes <= st.budget, "{} of {}", st.bytes, st.budget);
}

#[test]
fn the_thumbnail_margin_is_whole_lines_of_tiles() {
    let mut rt = runtime(host(2_000, 900.0, 600.0), 1.0);
    for _ in 0..10 {
        serve(&mut rt.state);
        rt.frame(DT);
    }
    let s = &rt.state;
    let per = s.coll.per_line();
    assert!(per > 1);
    let shown = s.coll.visible_range();
    let loaded: Vec<usize> = (0..s.src.len())
        .filter(|&i| {
            let key = s.src.entry(i).thumbnail.expect("every entry has one");
            matches!(s.store.state(key), crate::thumbnails::State::Ready { .. })
        })
        .collect();
    let end = loaded.last().copied().expect("some loaded") + 1;
    assert_eq!(loaded.len(), end, "loaded from the top without a hole");
    assert!(
        end >= shown.end + per,
        "a line or more past the viewport: {end} vs {shown:?}"
    );
    assert_eq!(end % per, 0, "whole lines of {per}: {end}");
}

#[test]
fn the_hosts_wheel_hook_hears_each_wheel_before_the_size_wheel() {
    let resize = Mods {
        ctrl: true,
        shift: true,
        ..Mods::default()
    };
    // (case, modifiers, the host takes it, scrolls, resizes)
    let cases = [
        ("a plain wheel", Mods::default(), false, true, false),
        ("Ctrl+Shift+wheel", resize, false, false, true),
        (
            "Ctrl+Shift+wheel the host takes",
            resize,
            true,
            false,
            false,
        ),
    ];
    for (case, mods, takes, scrolls, resizes) in cases {
        let mut h = host(400, 900.0, 600.0);
        h.takes_wheels = takes;
        let mut rt = runtime(h, 1.0);
        let v = rt.state.coll.viewport();
        let (x, y) = (v.x + v.w / 2.0, v.y + v.h / 2.0);
        rt.pointer_move(x, y);
        rt.frame(DT);
        rt.wheel_event(WheelEvent::new(0.0, -120.0).at(x, y).mods(mods));
        for _ in 0..10 {
            rt.frame(DT);
        }
        assert_eq!(rt.state.wheels, 1, "{case}: the host heard it");
        assert_eq!(
            rt.state.coll.scroll_offset() > 0.0,
            scrolls,
            "{case}: scrolled"
        );
        assert_eq!(rt.state.size != 64.0, resizes, "{case}: resized");
    }
}

#[test]
fn poll_resyncs_thumbnails_when_listing_version_changes_without_scroll() {
    let mut rt = runtime(host(40, 900.0, 600.0), 1.0);
    rt.poll = Some(|s: &mut Host, _dt: f64, tx: &mut Tx| grid(s).poll(s, tx));
    serve(&mut rt.state);
    rt.frame(DT);

    let new_key = ThumbKey::new(999_999);
    rt.update(|s: &mut Host, _: &mut Tx| {
        let mut entries: Vec<ListEntry> = (0..40)
            .map(|i| {
                let mut e = ListEntry::new(
                    EntryId::new(i as u64 + 1),
                    format!("img-{i}.jpg"),
                    EntryKind::File,
                    "file_image",
                );
                e.thumbnail = Some(if i == 0 {
                    new_key
                } else {
                    ThumbKey::new(10_000 + i as u64)
                });
                e
            })
            .collect();
        s.src = ListSource::new(std::mem::take(&mut entries));
        s.thumb_version += 1;
    });
    rt.frame(DT);

    let served = serve(&mut rt.state);
    assert!(served >= 1, "store must request the replaced thumbnail key");
    assert!(
        matches!(
            rt.state.store.state(new_key),
            crate::thumbnails::State::Ready { .. }
        ),
        "replaced thumbnail key reached Ready without scrolling"
    );
}

#[test]
fn fast_trough_click_rebinds_skipped_thumbnails_on_next_poll() {
    let mut h = host(1_000, 900.0, 600.0);
    h.grid.set_bar_mode(crate::scroll::BarMode::Always);
    let mut rt = runtime(h, 1.0);
    rt.poll = Some(|s: &mut Host, _dt: f64, tx: &mut Tx| grid(s).poll(s, tx));
    serve(&mut rt.state);
    rt.frame(DT);

    let trough = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some("trough"))
        .expect("always-on scrollbar trough");
    let click_x = trough.rect[0] + trough.rect[2] / 2.0;
    let click_y = trough.rect[1] + trough.rect[3] * 0.85;

    // Press and release before any poll runs: Commit::Instant mounts destination rows mid-drag.
    rt.pointer_move(click_x, click_y);
    rt.pointer_down(click_x, click_y);
    rt.pointer_up(click_x, click_y);

    for _ in 0..4 {
        rt.frame(DT);
        serve(&mut rt.state);
    }

    let shown = rt.state.coll.visible_range();
    assert!(
        shown.start > 50,
        "trough click jumped into uncached rows: {shown:?}"
    );
    let elems = rt.elements();
    let first_id = rt.state.src.id_at(shown.start);
    let thumb_suffix = format!(
        "/row-{}/{}/{}",
        first_id.get(),
        crate::collection::element::ICON,
        crate::thumbnails::THUMB
    );
    assert!(
        elems
            .iter()
            .any(|e| !e.exiting && e.path.ends_with(&thumb_suffix)),
        "destination tile must rebind its THUMB element after fast trough click"
    );
    let first_key = rt.state.src.entry(shown.start).thumbnail.expect("key");
    assert!(
        matches!(
            rt.state.store.state(first_key),
            crate::thumbnails::State::Ready { .. }
        ),
        "destination tile thumbnail reached Ready"
    );
}
