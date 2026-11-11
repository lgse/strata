//! The hooks C3 builds on: bare items, a line's side inset, the viewport's wheel and the count
//! a line, each held to what the element draws and routes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rune::{Color, Cx, El, Env, Key, KeyEvent, Mods, Runtime, WheelEvent};
use strata_kit::look::Look;

use super::fixture::FastSource;
use crate::collection::element::{Collection, CollectionState, Lines, Roles, Slot};
use crate::contract::{CollectionGeometry, EntryId, EntrySource, IndexRanges, Point, Rect};

const W: f32 = 500.0;
const H: f32 = 400.0;
const TILE: f32 = 100.0;
const INSET: f32 = 50.0;

/// Tiles `TILE` wide inside a side inset of `INSET`.
struct Inset;

impl Lines for Inset {
    fn per_line(&self, width: f32) -> usize {
        (((width - 2.0 * INSET) / TILE) as usize).max(1)
    }
    fn inset(&self) -> f32 {
        INSET
    }
    fn geometry(&self, len: usize, h: f32, w: f32, _vh: f32) -> Box<dyn CollectionGeometry> {
        Box::new(Grid {
            len,
            per: self.per_line(w),
            cw: (w - 2.0 * INSET) / self.per_line(w) as f32,
            h,
        })
    }
}

struct Grid {
    len: usize,
    per: usize,
    cw: f32,
    h: f32,
}

impl CollectionGeometry for Grid {
    fn rect_of(&self, i: usize) -> Rect {
        Rect::new(
            INSET + (i % self.per) as f32 * self.cw,
            (i / self.per) as f32 * self.h,
            self.cw,
            self.h,
        )
    }
    fn index_at(&self, p: Point) -> Option<usize> {
        let x = p.x - INSET;
        if x < 0.0 || p.y < 0.0 || x >= self.cw * self.per as f32 {
            return None;
        }
        let i = (p.y / self.h) as usize * self.per + (x / self.cw) as usize;
        (i < self.len).then_some(i)
    }
    fn range_in(&self, _: Rect) -> IndexRanges {
        IndexRanges::new()
    }
    fn neighbor(&self, i: usize, d: crate::contract::Direction) -> Option<usize> {
        use crate::contract::Direction::*;
        match d {
            Down => Some(i + self.per).filter(|&n| n < self.len),
            Up => i.checked_sub(self.per),
            Left | Right => None,
        }
    }
    fn page_step(&self) -> usize {
        self.per
    }
}

struct Host {
    src: FastSource,
    coll: CollectionState,
    look: Look,
    bare: bool,
    grid: bool,
    wheels: Vec<WheelEvent>,
    /// What the label slot saw of hover on each build.
    hover_seen: Rc<RefCell<Vec<bool>>>,
}

fn host(n: usize) -> Host {
    Host {
        src: FastSource::new(n),
        coll: CollectionState::new(),
        look: Look::fallback(false),
        bare: false,
        grid: false,
        wheels: Vec::new(),
        hover_seen: Rc::default(),
    }
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    let seen = s.hover_seen.clone();
    let mut c = Collection::new(
        "list",
        |s: &Host| (&s.src as &dyn EntrySource, &s.coll),
        |s: &mut Host| (&s.src as &dyn EntrySource, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
    .label(Slot::pure(move |_: &Host, rcx| {
        seen.borrow_mut().push(rcx.hover_live);
        rune::text(rcx.entry.name.to_owned())
    }))
    .on_wheel(|s: &mut Host, e, _| {
        s.wheels.push(e);
        e.mods.ctrl
    });
    if s.bare {
        c = c.bare_items();
    }
    if s.grid {
        c = c.lines(Inset).roles(Roles::GRID_CELLS);
    }
    rune::row().size(W, H).child(c.build(s))
}

fn runtime(h: Host) -> Runtime<Host> {
    let mut rt = Runtime::new(h, view, (W, H), Env::new(false, 1.0));
    for _ in 0..4 {
        rt.frame(1.0 / 60.0);
    }
    rt
}

fn select_first(rt: &mut Runtime<Host>) {
    rt.update(|s, _| {
        s.coll.selection.select_only(&s.src, 0);
    });
    rt.frame(1.0 / 60.0);
}

/// Elements drawn under row 0 with a fill of their own.
fn filled_under_row_0(rt: &Runtime<Host>) -> usize {
    let row = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("row-0"))
        .expect("row 0")
        .path;
    rt.elements()
        .into_iter()
        .filter(|e| e.path.starts_with(&row) && e.fill != Color::TRANSPARENT && e.fill.a > 0.0)
        .count()
}

#[test]
fn bare_items_draw_no_selection_fill_of_their_own() {
    let mut rt = runtime(host(5));
    select_first(&mut rt);
    assert!(filled_under_row_0(&rt) > 0, "a plain selected row fills");

    let mut bare = host(5);
    bare.bare = true;
    let mut rt = runtime(bare);
    select_first(&mut rt);
    assert_eq!(
        filled_under_row_0(&rt),
        0,
        "a bare one leaves it to its slots"
    );
}

#[test]
fn slots_hear_hover_turn_off_after_a_key_moves_the_cursor() {
    let mut h = host(5);
    h.bare = true;
    h.coll.controllers.keyboard = Some(Box::new(crate::collection::keyboard::Keyboard::default()));
    let seen = h.hover_seen.clone();
    let mut rt = runtime(h);
    // A press focuses the collection, so the key reaches it.
    rt.pointer_down(W / 2.0, 10.0);
    rt.pointer_up(W / 2.0, 10.0);
    rt.frame(1.0 / 60.0);
    assert!(
        seen.borrow().iter().all(|&h| h),
        "hover is live before any key"
    );
    seen.borrow_mut().clear();
    rt.key(KeyEvent {
        key: Key::Down,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
    assert!(!seen.borrow().is_empty(), "the moved rows built again");
    assert!(seen.borrow().iter().all(|&h| !h), "and saw hover off");
}

#[test]
fn a_lines_inset_is_where_presses_find_no_entry() {
    let mut h = host(40);
    h.grid = true;
    let mut rt = runtime(h);
    assert_eq!(
        rt.state.coll.per_line(),
        4,
        "(500 - 2·50) / 100 tiles a line"
    );
    // The geometry leaves the inset out, and the element draws the line's cells inside it.
    let picked = |rt: &Runtime<Host>, x: f32| {
        let path = rt.element_at(x, 20.0)?.path().to_owned();
        path.split('/')
            .find(|seg| seg.starts_with("row-"))
            .map(str::to_owned)
    };
    assert_eq!(picked(&rt, INSET - 5.0), None);
    assert_eq!(picked(&rt, INSET + 5.0).as_deref(), Some("row-0"));
    assert_eq!(picked(&rt, W - INSET - 5.0).as_deref(), Some("row-3"));
    assert_eq!(picked(&rt, W - INSET + 5.0), None);
    rt.pointer_down(INSET + 5.0, 20.0);
    rt.pointer_up(INSET + 5.0, 20.0);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.coll.selection.cursor(), Some(EntryId::new(0)));
}

#[test]
fn the_viewport_hears_wheels_first_and_scrolls_those_it_leaves() {
    let mut rt = runtime(host(1_000));
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    let wheel = Cell::new(0);
    let send = |rt: &mut Runtime<Host>, mods| {
        wheel.set(wheel.get() + 1);
        rt.wheel_event(WheelEvent::new(0.0, -120.0).at(W / 2.0, H / 2.0).mods(mods));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
    };
    send(&mut rt, ctrl);
    assert_eq!(rt.state.wheels.len(), 1);
    assert_eq!(
        rt.state.coll.scroll_offset(),
        0.0,
        "a taken wheel doesn't scroll"
    );
    send(&mut rt, Mods::default());
    assert_eq!(rt.state.wheels.len(), 2);
    assert!(
        rt.state.coll.scroll_offset() > 0.0,
        "a wheel left to it scrolls"
    );
}
