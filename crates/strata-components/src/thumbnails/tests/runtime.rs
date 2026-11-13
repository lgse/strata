// SPDX-License-Identifier: MIT

//! In a headless runtime, on C1e's element: thumbnails land, change size and go with no
//! view run and no commit beyond the scroll's own, and the icon slot draws what it should.

use std::cell::Cell;
use std::rc::Rc;

use rune::{Cx, El, Env, Runtime, col, text};
use strata_kit::look::Look;

use super::{image, index, source};
use crate::collection::element::{Collection, CollectionState, RowMarks, Slot};
use crate::contract::ListSource;
use crate::thumbnails::{GLYPH, THUMB, ThumbnailStore, ViewerId, Viewport, icon_slot};

const W: f32 = 400.0;
const H: f32 = 600.0;
const ROW: f32 = 40.0;
const PT: f32 = 32.0;
const DT: f64 = 1.0 / 120.0;

struct Host {
    src: ListSource,
    coll: CollectionState,
    look: Look,
    store: ThumbnailStore,
    viewer: ViewerId,
    views: Rc<Cell<usize>>,
    built: Rc<Cell<usize>>,
    /// The slot dims file icons ([`IconSlot::dim_files`](crate::thumbnails::IconSlot::dim_files)).
    dim: bool,
    /// The entry ids drawn as cut.
    cut: Vec<u64>,
}

impl Host {
    fn new(n: usize) -> Self {
        let mut store = ThumbnailStore::default();
        let viewer = store.viewer();
        Self {
            src: source(n, &[5, 6]),
            coll: CollectionState::new(),
            look: Look::fallback(false),
            store,
            viewer,
            views: Rc::default(),
            built: Rc::default(),
            dim: false,
            cut: Vec::new(),
        }
    }

    fn step(&self) -> super::Step {
        super::Step::for_tile(&self.look, PT, 1.0)
    }

    /// A worker that finishes everything asked for at once, as `poll` would hand it over.
    fn serve(&mut self) -> usize {
        let mut n = 0;
        while let Some(r) = self.store.pop(0.0) {
            self.store.complete(r.ticket, image(r.step), 0.0);
            n += 1;
        }
        n
    }
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    s.views.set(s.views.get() + 1);
    let built = s.built.clone();
    let step = s.step();
    col().size(W, H).child(
        Collection::new(
            "list",
            |s: &Host| (&s.src, &s.coll),
            |s: &mut Host| (&s.src, &mut s.coll),
            |s: &Host| &s.look,
            |_: &Host| 1,
        )
        .row_height(|_, _| ROW)
        .icon(match s.dim {
            true => icon_slot(|s: &Host| &s.store, step, PT).dim_files(),
            false => icon_slot(|s: &Host| &s.store, step, PT),
        })
        .marks(|s: &Host, id| RowMarks {
            cut: s.cut.contains(&id.get()),
            ..RowMarks::default()
        })
        .label(Slot::pure(move |_: &Host, rcx| {
            built.set(built.get() + 1);
            text(rcx.entry.name.to_owned())
        }))
        .on_visible(move |s: &mut Host, r| {
            s.store
                .sync(s.viewer, &s.src, Viewport::new(r, step).listing(1))
        })
        .build(s),
    )
}

fn runtime(n: usize) -> Runtime<Host> {
    runtime_of(Host::new(n))
}

fn runtime_of(host: Host) -> Runtime<Host> {
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    for _ in 0..20 {
        rt.frame(DT);
    }
    rt
}

fn thumbs_drawn(rt: &Runtime<Host>) -> usize {
    rt.elements()
        .iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some(THUMB))
        .count()
}

/// Commits, view runs and rows built over 240 frames of scrolling, serving every request
/// before each frame when `serve`.
fn scroll(serve: bool) -> (u64, usize, usize, usize) {
    let mut rt = runtime(5000);
    let (c, v, b) = (rt.stats.commits, rt.state.views.get(), rt.state.built.get());
    let mut landed = 0;
    for _ in 0..240 {
        rt.wheel(200.0, 300.0, -30.0);
        if serve {
            landed += rt.state.serve();
        }
        rt.frame(DT);
    }
    (
        rt.stats.commits - c,
        rt.state.views.get() - v,
        rt.state.built.get() - b,
        landed,
    )
}

#[test]
fn thumbnails_land_while_scrolling_with_no_commit_of_their_own() {
    let (commits, views, built, landed) = scroll(true);
    let (base_commits, base_views, base_built, _) = scroll(false);
    assert!(landed > 100, "{landed} landed");
    assert_eq!(views, 0, "no view run");
    assert_eq!(views, base_views);
    assert_eq!(commits, base_commits, "only the scroll's own window shifts");
    assert_eq!(built, base_built, "rows build as they mount, no more");
}

#[test]
fn a_landed_thumbnail_draws_over_its_faded_icon_without_building() {
    let mut rt = runtime(200);
    let (v, b) = (rt.state.views.get(), rt.state.built.get());
    let shown = rt.state.coll.visible_range();
    assert!(!shown.is_empty());
    rt.state.serve();
    rt.frame(DT);
    rt.frame(DT);
    assert_eq!((rt.state.views.get(), rt.state.built.get()), (v, b));
    let s = &rt.state;
    for i in shown {
        let entry = crate::contract::EntrySource::entry(&s.src, i);
        match entry.thumbnail {
            Some(k) => {
                let t = s.store.thumb(k);
                assert_eq!(t.icon_alpha().get(), 0.0, "entry {i}'s icon faded");
                assert!(t.pixels().size().0 > 0, "entry {i} holds an image");
                assert_eq!(index(k), i);
            }
            None => assert!(i == 5 || i == 6, "only the folders have no key"),
        }
    }
    assert!(thumbs_drawn(&rt) > 0);
}

#[test]
fn turning_thumbnails_off_builds_the_rows_once_as_icons_and_on_restores_them() {
    let mut rt = runtime(200);
    rt.state.serve();
    rt.frame(DT);
    let drawn = thumbs_drawn(&rt);
    assert!(drawn > 0);
    let v = rt.state.views.get();
    rt.update(|s, _| {
        assert!(s.store.set_enabled(false));
    });
    rt.frame(DT);
    rt.frame(DT);
    assert_eq!(thumbs_drawn(&rt), 0, "icons alone");
    assert_eq!(rt.state.views.get(), v + 1, "one view run");
    rt.update(|s, _| {
        assert!(s.store.set_enabled(true));
    });
    rt.frame(DT);
    assert_eq!(thumbs_drawn(&rt), drawn);
    assert_eq!(rt.state.serve(), 0, "nothing on screen decodes again");
}

/// Each drawn element keyed `key`, by the row it is in, with its opacity as drawn (its row's
/// included).
fn opacities(rt: &Runtime<Host>, key: &str) -> Vec<(u64, f32)> {
    rt.elements()
        .iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some(key))
        .filter_map(|e| {
            let row = e
                .path
                .split('/')
                .find_map(|part| part.strip_prefix("row-"))?;
            Some((row.parse().ok()?, e.opacity))
        })
        .collect()
}

/// Where the slot dims files, a file's placeholder icon draws at `collection.file_icon.opacity`
/// and a folder's, or a cut file's, at full opacity; a landed thumbnail is never dimmed.
#[test]
fn a_dimmed_slot_dims_file_icons_but_no_folder_cut_entry_or_thumbnail() {
    let mut host = Host::new(20);
    host.dim = true;
    // Ids are indices + 1: entries 5 and 6 are folders, ids 6 and 7.
    host.cut = vec![3];
    let mut rt = runtime_of(host);
    let dimmed = strata_kit::slots::collection::file_icon_opacity(&rt.state.look);
    // The cut row fades as a whole; its icon adds no dimming of its own (the mark wins).
    let cut = strata_kit::slots::collection::cut_opacity(&rt.state.look);
    let icon = |id: u64| match id {
        3 => cut,
        6 | 7 => 1.0,
        _ => dimmed,
    };
    assert_drawn(&opacities(&rt, GLYPH), icon, "icon");

    rt.state.serve();
    rt.frame(DT);
    rt.frame(DT);
    let thumbnail = |id: u64| if id == 3 { cut } else { 1.0 };
    assert_drawn(&opacities(&rt, THUMB), thumbnail, "thumbnail");
}

/// Each drawn element has the opacity `expected` gives its row, and there is at least one.
fn assert_drawn(drawn: &[(u64, f32)], expected: impl Fn(u64) -> f32, what: &str) {
    assert!(!drawn.is_empty(), "no {what} drawn");
    for &(id, opacity) in drawn {
        assert_eq!(opacity, expected(id), "entry {id}'s {what}, drawn");
    }
}

/// A slot that does not dim draws every icon at full opacity, as Strata's List rows do.
#[test]
fn an_undimmed_slot_draws_every_icon_at_full_opacity() {
    let rt = runtime(20);
    let glyphs = opacities(&rt, GLYPH);
    assert!(!glyphs.is_empty());
    assert!(
        glyphs.iter().all(|(_, opacity)| *opacity == 1.0),
        "{glyphs:?}"
    );
}
