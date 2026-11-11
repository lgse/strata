//! The element in a headless runtime: input routing to the default click and to installed
//! controllers, effects and intents, what the host hears, memoized rows, virtualization,
//! position restore and what assistive technology reads.

mod describe;
mod fixture;
mod hooks;
mod perf;
mod x2;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rune::runtime::ElementInfo;
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col};
use strata_kit::look::Look;

use self::fixture::FastSource;
use super::*;
use crate::collection::selection::{CursorGone, Listing, SyncRule};

const W: f32 = 500.0;
const H: f32 = 400.0;
const SCOPE: &str = "list";

#[derive(Default)]
struct Log {
    pointer: Vec<PointerInput>,
    marquee: Vec<&'static str>,
}

struct Host {
    src: FastSource,
    coll: CollectionState,
    look: Look,
    version: u64,
    cursors: Vec<Option<EntryId>>,
    selections: usize,
    intents: Vec<String>,
    last_intent: Option<Intent>,
    /// Rows built (label slot calls).
    built: Rc<Cell<usize>>,
    per_line: usize,
    app_keys: usize,
    /// A header above the viewport, and an overlay over it.
    parts: bool,
    read_only: bool,
    /// A row height from the state, when set (else the look's).
    row_h: Option<f32>,
    /// A grid whose tiles are this wide, when set.
    tile: Option<f32>,
    /// The collection's width.
    width: f32,
    /// Roles other than the default, when set.
    roles: Option<Roles>,
    /// Takes drops, recording where each went.
    drops: bool,
    dropped: Vec<Option<EntryId>>,
    /// With `drops`, every zone refuses every drop (the host's action says `None`).
    refuse: bool,
    /// What `on_press` and a recording controller heard, in order.
    order: Rc<RefCell<Vec<String>>>,
    /// Hears drags over the drop zones through `on_drag_over`, when set.
    over: Option<Rc<RefCell<Vec<Option<Point>>>>>,
    /// Holds a header press back until it passes the drag threshold.
    defer: bool,
    /// Folder rows report resting (500 and 80 ms), and the row `anchor` names gets a part.
    rest: bool,
    anchor: Option<EntryId>,
    rests: Vec<RestReport>,
    views: Vec<RestView>,
    /// The runtime's clock, from its poll ([`rest_runtime`]).
    t: f64,
    /// Entries marked vanished (`RowMarks::vanished`).
    vanished: Vec<EntryId>,
    /// With it, two `on_scroll` handlers and two `edge` parts, each logging its name and the
    /// offset heard.
    followers: Option<FollowerLog>,
}

/// What each `on_scroll` follower heard: its name and the offset.
type FollowerLog = Rc<RefCell<Vec<(&'static str, f32)>>>;

impl Host {
    fn new(n: usize) -> Self {
        Self {
            src: FastSource::new(n),
            coll: CollectionState::new(),
            look: Look::fallback(false),
            version: 1,
            cursors: Vec::new(),
            selections: 0,
            intents: Vec::new(),
            last_intent: None,
            built: Rc::new(Cell::new(0)),
            per_line: 1,
            app_keys: 0,
            parts: false,
            read_only: false,
            row_h: None,
            tile: None,
            width: W,
            roles: None,
            drops: false,
            dropped: Vec::new(),
            refuse: false,
            order: Rc::default(),
            over: None,
            defer: false,
            rest: false,
            anchor: None,
            rests: Vec::new(),
            views: Vec::new(),
            t: 0.0,
            vanished: Vec::new(),
            followers: None,
        }
    }
}

/// N entries per line, for the grid roles.
struct PerLine(usize);

impl Lines for PerLine {
    fn per_line(&self, _width: f32) -> usize {
        self.0
    }

    fn geometry(&self, len: usize, h: f32, w: f32, vh: f32) -> Box<dyn CollectionGeometry> {
        Box::new(Grid {
            len,
            per: self.0,
            h,
            w,
            vh,
        })
    }
}

/// `per` equal cells to a line, as the element draws a grid's lines.
struct Grid {
    len: usize,
    per: usize,
    h: f32,
    w: f32,
    vh: f32,
}

impl CollectionGeometry for Grid {
    fn rect_of(&self, i: usize) -> crate::contract::Rect {
        let cw = self.w / self.per as f32;
        crate::contract::Rect::new(
            (i % self.per) as f32 * cw,
            (i / self.per) as f32 * self.h,
            cw,
            self.h,
        )
    }
    fn index_at(&self, p: Point) -> Option<usize> {
        if p.x < 0.0 || p.y < 0.0 || p.x >= self.w {
            return None;
        }
        let i = (p.y / self.h) as usize * self.per + (p.x / (self.w / self.per as f32)) as usize;
        (i < self.len).then_some(i)
    }
    fn range_in(&self, _: crate::contract::Rect) -> crate::contract::IndexRanges {
        crate::contract::IndexRanges::new()
    }
    fn neighbor(&self, _: usize, _: crate::contract::Direction) -> Option<usize> {
        None
    }
    fn page_step(&self) -> usize {
        ((self.vh / self.h) as usize).max(1) * self.per
    }
}

use crate::contract::CollectionGeometry;

fn collection(s: &Host) -> Collection<Host> {
    let built = s.built.clone();
    let mut c = Collection::new(
        SCOPE,
        |s: &Host| (&s.src, &s.coll),
        |s: &mut Host| (&s.src, &mut s.coll),
        |s: &Host| &s.look,
        |s: &Host| s.version,
    )
    .name("Files")
    .label(Slot::pure(move |_: &Host, rcx| {
        built.set(built.get() + 1);
        rune::text(rcx.entry.name.to_owned())
    }))
    .on_cursor(|s: &mut Host, id, _| s.cursors.push(id))
    .on_selection(|s: &mut Host, _| s.selections += 1)
    .on_press(|s: &mut Host, input, _| {
        s.order
            .borrow_mut()
            .push(format!("press {:?} {:?}", input.button, input.hit.index));
    })
    .on_intent(|s: &mut Host, intent, _| {
        s.intents.push(format!("{intent:?}"));
        s.last_intent = Some(intent);
    });
    if s.per_line > 1 {
        c = c.lines(PerLine(s.per_line)).roles(Roles::GRID_CELLS);
    }
    if let Some(tile) = s.tile {
        c = c.lines(Tiles(tile)).roles(Roles::GRID_CELLS);
    }
    if s.row_h.is_some() {
        c = c.row_height(|s: &Host, l: &Look| s.row_h.unwrap_or(l.row.list));
    }
    if s.parts {
        c = c
            .header(|_: &Host| {
                Some(
                    rune::row().key("head").w_full().h(40.0).child(
                        rune::el()
                            .key("head-button")
                            .w(60.0)
                            .h(40.0)
                            .on_press(|s: &mut Host| s.app_keys += 1),
                    ),
                )
            })
            .overlay(|_: &Host| Some(rune::el().key("band").w_full().h_full()));
    }
    if s.read_only {
        c = c.read_only().rename(
            |_: &Host| Some(EntryId::new(2)),
            Slot::pure(|_: &Host, _| rune::el().key("editor")),
        );
    }
    if let Some(roles) = s.roles {
        c = c.roles(roles);
    }
    if s.drops {
        c = c.on_drop(|s: &mut Host, target, _, _| s.dropped.push(target));
        if s.refuse {
            c = c.drop_action(|_: &Host, _, _, _| None);
        }
    }
    if s.defer {
        c = c.header_defers_press(
            crate::collection::pointer::PointerSettings::default().drag_threshold,
        );
    }
    if let Some(log) = &s.over {
        let log = log.clone();
        c = c.on_drag_over(move |_: &mut Host, at, _| {
            log.borrow_mut().push(at);
            false
        });
    }
    if s.rest {
        c = c.rest(
            Rest::new(
                |_: &Host, rcx| {
                    rcx.entry.kind.is_folder_like().then_some((
                        std::time::Duration::from_millis(500),
                        std::time::Duration::from_millis(80),
                    ))
                },
                |s: &mut Host, report, _| s.rests.push(report),
            )
            .on_view(|s: &mut Host, view| {
                s.views.push(view);
                false
            })
            .part(
                |s: &Host, rcx| u64::from(s.anchor == Some(rcx.entry.id)),
                |s: &Host, rcx| {
                    (s.anchor == Some(rcx.entry.id)).then(|| {
                        rune::el()
                            .key("anchor")
                            .pin(Some(10.0), Some(0.0), None, Some(0.0))
                            .w(0.0)
                    })
                },
            ),
        );
    }
    if !s.vanished.is_empty() {
        c = c.marks(|s: &Host, id| RowMarks {
            vanished: s.vanished.contains(&id),
            ..RowMarks::default()
        });
    }
    if let Some(log) = &s.followers {
        for name in ["first", "second"] {
            let log = log.clone();
            c = c
                .on_scroll(move |p| log.borrow_mut().push((name, p.offset[1])))
                .edge(move |_: &Host| Some(rune::el().key(format!("edge-{name}")).size(4.0, 4.0)));
        }
    }
    c.settings(7u32)
}

/// Tiles `0` points wide, as many to a line as the width takes.
struct Tiles(f32);

impl Lines for Tiles {
    fn per_line(&self, width: f32) -> usize {
        ((width / self.0) as usize).max(1)
    }

    fn geometry(&self, len: usize, h: f32, w: f32, vh: f32) -> Box<dyn CollectionGeometry> {
        PerLine(self.per_line(w)).geometry(len, h, w, vh)
    }
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    rune::row()
        .size(W, H)
        .child(col().w(s.width).h_full().child(collection(s).build(s)))
}

fn runtime(host: Host) -> Runtime<Host> {
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn settle(rt: &mut Runtime<Host>) {
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
    }
}

fn element(rt: &Runtime<Host>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
}

/// Presses and releases at the middle of `key`'s visible part.
fn click(rt: &mut Runtime<Host>, key: &str) {
    let e = element(rt, key).unwrap_or_else(|| panic!("no element {key}"));
    let [x, y, w, h] = e.visible;
    click_at(rt, x + w / 2.0, y + h / 2.0);
}

/// Presses and releases on `key`'s name, at the start of its row.
fn click_label(rt: &mut Runtime<Host>, key: &str) {
    let e = element(rt, key).unwrap_or_else(|| panic!("no element {key}"));
    let [x, y, _, h] = e.visible;
    click_at(rt, x + 40.0, y + h / 2.0);
}

fn click_at(rt: &mut Runtime<Host>, x: f32, y: f32) {
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
}

fn press(rt: &mut Runtime<Host>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
}

fn id(i: u64) -> EntryId {
    EntryId::new(i)
}

fn rows(rt: &Runtime<Host>) -> Vec<ElementInfo> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting && e.role == "option")
        .collect()
}

// The default pointer.

#[test]
fn a_press_selects_only_that_entry_and_a_press_on_empty_space_clears() {
    let mut rt = runtime(Host::new(5));
    click(&mut rt, "row-1");
    click(&mut rt, "row-3");
    let s = &rt.state;
    assert_eq!(s.coll.selection.selected_ids(&s.src), [id(3)]);
    assert_eq!(s.coll.selection.cursor(), Some(id(3)));
    assert_eq!(
        s.cursors,
        [Some(id(1)), Some(id(3))],
        "the host hears each cursor move"
    );
    // Below the last of five rows.
    click_at(&mut rt, W / 2.0, H - 10.0);
    let s = &rt.state;
    assert!(s.coll.selection.is_empty());
    assert_eq!(
        s.coll.selection.cursor(),
        Some(id(3)),
        "clearing keeps the cursor"
    );
    assert_eq!(s.selections, 3);
}

#[test]
fn every_scroll_handler_and_edge_part_added_is_kept() {
    // A view's own bar (C3, C4) and a host's follower both set these: neither may drop the other.
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut rt = runtime(Host {
        followers: Some(log.clone()),
        ..Host::new(1_000)
    });
    assert!(element(&rt, "edge-first").is_some() && element(&rt, "edge-second").is_some());
    log.borrow_mut().clear();
    rt.wheel(W / 2.0, H / 2.0, -120.0);
    rt.frame(1.0 / 60.0);
    let heard = log.borrow().clone();
    let at = rt.state.coll.scroll_offset();
    assert!(at > 0.0);
    assert!(
        heard.contains(&("first", at)) && heard.contains(&("second", at)),
        "{heard:?}"
    );
}

#[test]
fn a_press_after_scrolling_hits_the_row_under_the_pointer() {
    // Rune rebases a virtual stack's on_drag y; the element must not use it.
    let mut rt = runtime(Host::new(10_000));
    for _ in 0..80 {
        rt.wheel(W / 2.0, H / 2.0, -300.0);
        rt.frame(1.0 / 60.0);
    }
    settle(&mut rt);
    assert!(rt.state.coll.scroll_offset() > 10_000.0);
    let target = rows(&rt)
        .into_iter()
        .find(|e| e.visible[1] > 100.0 && e.visible[1] < H - 60.0 && e.visible[3] > 20.0)
        .expect("a row in view");
    let key = target.key.clone().unwrap();
    click(&mut rt, &key);
    let cursor = rt.state.coll.selection.cursor().unwrap();
    assert_eq!(row_key(cursor), key);
}

#[test]
fn a_vanished_row_keeps_its_slot_but_draws_and_announces_nothing_and_takes_no_press() {
    let mut rt = runtime(Host::new(5));
    let before = element(&rt, "row-3").unwrap().visible;
    rt.state.vanished = vec![id(2)];
    rt.invalidate();
    rt.frame(1.0 / 60.0);
    settle(&mut rt);
    let slot = element(&rt, "row-2").expect("the slot stays, keyed");
    assert!(slot.visible[3] > 0.0, "it keeps its height");
    assert_eq!(
        element(&rt, "row-3").unwrap().visible,
        before,
        "the rows after it stay put"
    );
    assert!(a11y_node(&rt, "row-2").is_none(), "it leaves the tree");
    assert!(a11y_node(&rt, "row-3").is_some());
    click(&mut rt, "row-1");
    click(&mut rt, "row-2");
    let s = &rt.state;
    assert!(
        s.coll.selection.is_empty(),
        "a press on it is on empty space, which clears"
    );
    assert_eq!(s.coll.selection.cursor(), Some(id(1)));
}

#[test]
fn a_vanished_cursor_row_is_not_the_active_descendant() {
    // AccessKit's own update: the JSON dump leaves out a reference it can't resolve.
    let dangling = |rt: &Runtime<Host>| {
        let tree = rt.accessibility_tree();
        let present = |a| tree.nodes.iter().any(|(id, _)| *id == a);
        tree.nodes
            .iter()
            .filter_map(|(_, n)| n.active_descendant())
            .filter(|&a| !present(a))
            .count()
    };
    let mut rt = runtime(Host::new(5));
    click(&mut rt, "row-2");
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains(r#""activeDescendant":"row-2""#), "{tree}");
    rt.state.vanished = vec![id(2)];
    rt.invalidate();
    rt.frame(1.0 / 60.0);
    settle(&mut rt);
    assert_eq!(
        dangling(&rt),
        0,
        "nothing points at the cursor's row once it leaves the tree"
    );
    rt.state.vanished.clear();
    rt.invalidate();
    rt.frame(1.0 / 60.0);
    settle(&mut rt);
    let tree = rt.accessibility_json().to_string();
    assert!(
        tree.contains(r#""activeDescendant":"row-2""#),
        "it is back once the row shows: {tree}"
    );
}

// Controllers.

struct Pointer(Rc<RefCell<Log>>);

impl PointerController for Pointer {
    fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
        self.0.borrow_mut().pointer.push(*input);
        if input.phase == PointerPhase::Down
            && let Some(i) = input.hit.index
        {
            if input.button == rune::PointerButton::Secondary {
                let id = cx.source.id_at(i);
                cx.emit(Effect::Intent(Intent::ContextMenu {
                    target: Some(id),
                    anchor: crate::contract::Rect::new(input.window.x, input.window.y, 0.0, 0.0),
                }));
            } else if input.mods.shift {
                cx.emit(Effect::BeginMarquee {
                    at: input.point,
                    mods: input.mods,
                });
            } else {
                cx.apply(|sel, src| sel.toggle(src, i));
                let id = cx.source.id_at(i);
                cx.emit(Effect::Intent(Intent::Activate {
                    id,
                    how: Activation::Click,
                }));
            }
        }
        Flow::Handled
    }
}

struct Keys;

impl KeyController for Keys {
    fn key(&mut self, input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow {
        match input.event.key {
            Key::End => {
                let last = cx.source.len() - 1;
                cx.apply(|sel, src| sel.select_only(src, last));
                Flow::Handled
            }
            _ => Flow::Pass,
        }
    }
}

struct Band(Rc<RefCell<Log>>, bool);

impl MarqueeController for Band {
    fn begin(&mut self, _at: Point, _mods: Mods, _cx: &mut Ctx<'_>) {
        self.1 = true;
        self.0.borrow_mut().marquee.push("begin");
    }
    fn moved(&mut self, _at: Point, _cx: &mut Ctx<'_>) {
        self.0.borrow_mut().marquee.push("moved");
    }
    fn end(&mut self, _cx: &mut Ctx<'_>) {
        self.1 = false;
        self.0.borrow_mut().marquee.push("end");
    }
    fn cancel(&mut self, _cx: &mut Ctx<'_>) -> bool {
        let was = std::mem::replace(&mut self.1, false);
        if was {
            self.0.borrow_mut().marquee.push("cancel");
        }
        was
    }
    fn active(&self) -> bool {
        self.1
    }
    fn band(&self) -> Option<crate::contract::Rect> {
        None
    }
    fn reset(&mut self) {
        self.1 = false;
    }
    fn overlay(&self, _look: &Look) -> Option<El> {
        self.1.then(|| {
            rune::el()
                .key("drawn-band")
                .absolute(0.0, 100.0)
                .w(60.0)
                .h(60.0)
        })
    }
}

fn with_controllers(n: usize) -> (Runtime<Host>, Rc<RefCell<Log>>) {
    let log = Rc::new(RefCell::new(Log::default()));
    let mut host = Host::new(n);
    host.coll.controllers.pointer = Some(Box::new(Pointer(log.clone())));
    host.coll.controllers.keyboard = Some(Box::new(Keys));
    host.coll.controllers.marquee = Some(Box::new(Band(log.clone(), false)));
    (runtime(host), log)
}

#[test]
fn an_installed_pointer_controller_replaces_the_default_and_its_intents_reach_the_host() {
    let (mut rt, log) = with_controllers(20);
    click_label(&mut rt, "row-2");
    click_label(&mut rt, "row-4");
    let s = &rt.state;
    assert_eq!(
        s.coll.selection.selected_ids(&s.src),
        [id(2), id(4)],
        "the controller toggles where the default would replace"
    );
    assert_eq!(s.intents.len(), 2);
    assert!(s.intents[1].starts_with("Activate { id: EntryId(4)"));
    let log = log.borrow();
    let downs: Vec<_> = log
        .pointer
        .iter()
        .filter(|p| p.phase == PointerPhase::Down)
        .collect();
    assert_eq!(downs.len(), 2);
    assert_eq!(downs[1].hit.index, Some(4));
    assert_eq!(
        downs[1].hit.region,
        Region::Name,
        "a press on the name is on the name"
    );
    assert!(log.pointer.iter().any(|p| p.phase == PointerPhase::Up));
}

#[test]
fn a_press_on_blank_row_space_is_in_the_gutter() {
    let (mut rt, log) = with_controllers(20);
    let row = element(&rt, "row-3").unwrap();
    let [x, y, w, h] = row.visible;
    click_at(&mut rt, x + w - 30.0, y + h / 2.0);
    let log = log.borrow();
    let down = log
        .pointer
        .iter()
        .find(|p| p.phase == PointerPhase::Down)
        .unwrap();
    assert_eq!(
        down.hit,
        Hit {
            index: Some(3),
            region: Region::Gutter
        }
    );
}

#[test]
fn a_secondary_press_reaches_the_controller_and_its_menu_intent_the_host() {
    let (mut rt, _log) = with_controllers(20);
    let e = element(&rt, "row-5").unwrap();
    let [ex, ey, ew, eh] = e.visible;
    let (x, y) = (ex + ew / 2.0, ey + eh / 2.0);
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.intents.len(), 1);
    assert!(rt.state.intents[0].starts_with("ContextMenu { target: Some(EntryId(5))"));
}

#[test]
fn a_marquee_takes_the_gesture_it_begins_and_escape_cancels_it() {
    let (mut rt, log) = with_controllers(40);
    let e = element(&rt, "row-2").unwrap();
    let [ex, ey, _, eh] = e.visible;
    let (x, y) = (ex + 30.0, ey + eh / 2.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    for k in 1..4 {
        rt.pointer_move(x, y + 30.0 * k as f32);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(x, y + 90.0);
    rt.frame(1.0 / 60.0);
    assert_eq!(
        log.borrow().marquee,
        ["begin", "moved", "moved", "moved", "end"]
    );
    let moves_to_pointer = log
        .borrow()
        .pointer
        .iter()
        .filter(|p| p.phase == PointerPhase::Move)
        .count();
    assert_eq!(
        moves_to_pointer, 0,
        "the band's moves don't reach the pointer controller"
    );

    rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    press(&mut rt, Key::Escape);
    assert_eq!(log.borrow().marquee.last(), Some(&"cancel"));
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
}

#[test]
fn a_key_the_controller_takes_moves_the_cursor_into_view_and_turns_keyboard_mode_on() {
    let (mut rt, _log) = with_controllers(5_000);
    click(&mut rt, "row-1");
    press(&mut rt, Key::Tab);
    press(&mut rt, Key::End);
    settle(&mut rt);
    let s = &rt.state;
    assert_eq!(s.coll.selection.cursor(), Some(id(4_999)));
    assert!(s.coll.keyboard_mode());
    let last = element(&rt, "row-4999").expect("the cursor's row is mounted");
    assert!(last.visible[3] > 20.0, "and shows");
    assert_eq!(rt.state.cursors.last(), Some(&Some(id(4_999))));
}

#[test]
fn a_key_no_controller_takes_goes_on_to_the_app() {
    let mut host = Host::new(5);
    host.coll.controllers.keyboard = Some(Box::new(Keys));
    let mut rt = runtime(host);
    rt.on_key = Some(|s: &mut Host, _: &KeyEvent, _: &mut rune::Tx| {
        s.app_keys += 1;
        false
    });
    click(&mut rt, "row-1");
    press(&mut rt, Key::Function(5));
    assert_eq!(rt.state.app_keys, 1);
    assert!(
        !rt.state.coll.keyboard_mode(),
        "an unhandled key leaves keyboard mode off"
    );
    press(&mut rt, Key::End);
    assert_eq!(
        rt.state.app_keys, 1,
        "a handled key stops at the collection"
    );
}

// What the host hears and what rows rebuild.

#[test]
fn a_selection_change_rebuilds_only_the_rows_whose_state_changed() {
    let mut rt = runtime(Host::new(200));
    settle(&mut rt);
    click(&mut rt, "row-3");
    settle(&mut rt);
    let before = rt.state.built.get();
    click(&mut rt, "row-6");
    settle(&mut rt);
    assert_eq!(
        rt.state.built.get() - before,
        2,
        "the row that lost the selection and the one that gained it"
    );
    let before = rt.state.built.get();
    rt.invalidate();
    rt.frame(1.0 / 60.0);
    assert_eq!(
        rt.state.built.get(),
        before,
        "a commit that changes nothing rebuilds nothing"
    );
}

#[test]
fn scrolling_runs_no_view() {
    thread_local! { static VIEWS: Cell<usize> = const { Cell::new(0) }; }
    fn counted(s: &Host, cx: &mut Cx<Host>) -> El {
        VIEWS.with(|v| v.set(v.get() + 1));
        view(s, cx)
    }
    let mut rt = Runtime::new(Host::new(20_000), counted, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    let before = VIEWS.with(Cell::get);
    for _ in 0..120 {
        rt.wheel(W / 2.0, H / 2.0, -40.0);
        rt.frame(1.0 / 60.0);
    }
    assert!(rt.state.coll.scroll_offset() > 1_000.0);
    assert_eq!(VIEWS.with(Cell::get), before);
}

#[test]
fn a_hundred_thousand_entries_build_only_what_shows() {
    let rt = runtime(Host::new(100_000));
    let built = rows(&rt).len();
    assert!(
        (5..200).contains(&built),
        "{built} rows built for a {H} pt viewport"
    );
}

// Keeping in step, and position restore.

#[test]
fn a_refresh_keeps_the_cursor_row_where_it_shows() {
    let mut rt = runtime(Host::new(3_000));
    for _ in 0..20 {
        rt.wheel(W / 2.0, H / 2.0, -200.0);
        rt.frame(1.0 / 60.0);
    }
    settle(&mut rt);
    let target = rows(&rt)
        .into_iter()
        .find(|e| e.visible[1] > 150.0)
        .and_then(|e| e.key)
        .unwrap();
    click(&mut rt, &target);
    let shown = element(&rt, &target).unwrap().visible;
    rt.update(|s, tx| {
        s.src.insert_top(25);
        s.coll
            .sync(SCOPE, &s.src, SyncRule::complete(CursorGone::Neighbor), tx);
    });
    settle(&mut rt);
    assert_eq!(element(&rt, &target).unwrap().visible, shown);
}

#[test]
fn a_position_comes_back_with_its_scroll_in_the_same_layout_else_the_cursor_shows() {
    let mut rt = runtime(Host::new(3_000));
    rt.update(|s, tx| {
        s.coll.selection.select_only(&s.src, 2_000);
        s.coll.reveal(SCOPE, &s.src, 2_000, tx);
    });
    settle(&mut rt);
    let offset = rt.state.coll.scroll_offset();
    assert!(offset > 1_000.0);
    let position = rt.state.coll.position(&rt.state.src);
    assert_eq!(position.cursor, Some(id(2_000)));

    // Leave, then come back.
    rt.update(|s, tx| {
        s.coll.enter(SCOPE, &s.src, None, Listing::Complete, tx);
    });
    settle(&mut rt);
    assert_eq!(rt.state.coll.scroll_offset(), 0.0);
    let p = position.clone();
    rt.update(move |s, tx| {
        s.coll.enter(SCOPE, &s.src, Some(&p), Listing::Complete, tx);
    });
    settle(&mut rt);
    assert_eq!(rt.state.coll.scroll_offset(), offset);
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(2_000)));

    // In another layout, the offset doesn't apply: the cursor scrolls into view.
    rt.update(|s, tx| {
        s.coll.enter(SCOPE, &s.src, None, Listing::Complete, tx);
        s.coll.set_layout_key(7);
    });
    settle(&mut rt);
    let p = position.clone();
    rt.update(move |s, tx| {
        s.coll.enter(SCOPE, &s.src, Some(&p), Listing::Complete, tx);
    });
    settle(&mut rt);
    let row = element(&rt, "row-2000").expect("the cursor's row is mounted");
    assert!(row.visible[3] > 20.0, "and shows");
}

#[test]
fn a_restore_waits_for_its_listing_and_a_press_before_then_cancels_it() {
    let mut rt = runtime(Host::new(50));
    let position = {
        let s = &mut rt.state;
        s.coll.selection.select_only(&s.src, 7);
        s.coll.position(&s.src)
    };
    let p = position.clone();
    rt.update(move |s, tx| {
        s.coll.enter(SCOPE, &s.src, Some(&p), Listing::Loading, tx);
    });
    rt.frame(1.0 / 60.0);
    assert!(rt.state.coll.restore_pending());
    rt.update(|s, tx| {
        s.coll
            .sync(SCOPE, &s.src, SyncRule::complete(CursorGone::Neighbor), tx);
    });
    rt.frame(1.0 / 60.0);
    assert!(!rt.state.coll.restore_pending());
    assert_eq!(
        rt.state.coll.selection.cursor(),
        Some(id(7)),
        "it applies once complete"
    );

    let p = position.clone();
    rt.update(move |s, tx| {
        s.coll.selection.clear();
        s.coll.enter(SCOPE, &s.src, Some(&p), Listing::Loading, tx);
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, "row-2");
    assert!(!rt.state.coll.restore_pending(), "the press cancelled it");
    rt.update(|s, tx| {
        s.coll
            .sync(SCOPE, &s.src, SyncRule::complete(CursorGone::Neighbor), tx);
    });
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(2)));
}

#[test]
fn a_re_sort_with_a_new_listing_version_shows_the_new_order() {
    let mut rt = runtime(Host::new(30));
    rt.update(|s, _| {
        s.src.reverse();
        s.version += 1;
    });
    settle(&mut rt);
    let first = rows(&rt)
        .into_iter()
        .min_by(|a, b| a.rect[1].total_cmp(&b.rect[1]))
        .and_then(|e| e.key);
    assert_eq!(first.as_deref(), Some("row-29"));
}

// Accessibility and keys.

#[test]
fn the_cursor_row_is_the_active_descendant_and_rows_say_they_are_selected() {
    let mut rt = runtime(Host::new(12));
    click(&mut rt, "row-4");
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains(r#""activeDescendant":"row-4""#), "{tree}");
    let selected: Vec<_> = rows(&rt).into_iter().filter(|e| e.item_selected).collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].key.as_deref(), Some("row-4"));
}

#[test]
fn a_grid_packs_entries_into_lines_of_cells() {
    let mut host = Host::new(10);
    host.per_line = 4;
    let mut rt = runtime(host);
    settle(&mut rt);
    let all = rt.elements();
    assert_eq!(all.iter().filter(|e| e.role == "gridcell").count(), 10);
    assert_eq!(all.iter().filter(|e| e.role == "row").count(), 3);
    assert!(all.iter().any(|e| e.role == "grid"));
}

// The seam requests from C1b's plan (N1–N7).

/// A pointer controller that records every input and key it hears, and does what `on_move`
/// says on the first move of each gesture.
struct Recorder {
    log: Rc<RefCell<Log>>,
    keys: Rc<Cell<usize>>,
    on_move: fn(&PointerInput, &mut Ctx<'_>),
    moved: bool,
}

impl PointerController for Recorder {
    fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
        self.log.borrow_mut().pointer.push(*input);
        match input.phase {
            PointerPhase::Down => self.moved = false,
            PointerPhase::Move if !self.moved => {
                self.moved = true;
                (self.on_move)(input, cx);
            }
            _ => {}
        }
        Flow::Handled
    }

    fn key_seen(&mut self, _input: &KeyInput<'_>) {
        self.keys.set(self.keys.get() + 1);
    }
}

fn recorder(
    n: usize,
    on_move: fn(&PointerInput, &mut Ctx<'_>),
) -> (Runtime<Host>, Rc<RefCell<Log>>, Rc<Cell<usize>>) {
    let log = Rc::new(RefCell::new(Log::default()));
    let keys = Rc::new(Cell::new(0));
    let mut host = Host::new(n);
    host.coll.controllers.pointer = Some(Box::new(Recorder {
        log: log.clone(),
        keys: keys.clone(),
        on_move,
        moved: false,
    }));
    host.coll.controllers.keyboard = Some(Box::new(Keys));
    host.coll.controllers.marquee = Some(Box::new(Band(log.clone(), false)));
    (runtime(host), log, keys)
}

/// N1: the name, the rest of the content (the icon) and the gutter are told apart.
#[test]
fn a_press_on_the_icon_is_content_and_on_the_name_is_the_name() {
    let (mut rt, log, _) = recorder(20, |_, _| {});
    let [x, y, w, h] = element(&rt, "row-3").unwrap().visible;
    let look = Look::fallback(false);
    let icon_mid =
        strata_kit::slots::collection::row_inset(&look) + look.space.xs + look.icon_size.md / 2.0;
    for px in [x + icon_mid, x + 40.0, x + w - 30.0] {
        click_at(&mut rt, px, y + h / 2.0);
    }
    let regions: Vec<_> = log
        .borrow()
        .pointer
        .iter()
        .filter(|p| p.phase == PointerPhase::Down)
        .map(|p| p.hit.region)
        .collect();
    assert_eq!(regions, [Region::Content, Region::Name, Region::Gutter]);
    assert!(Region::Content.on_entry() && Region::Name.on_entry() && !Region::Gutter.on_entry());
}

/// N2: the default pointer finishes a plain click with `Clicked`; a drag or a modified click
/// doesn't.
#[test]
fn a_plain_click_finishes_with_a_clicked_intent_and_a_drag_does_not() {
    let mut rt = runtime(Host::new(20));
    click(&mut rt, "row-4");
    assert!(matches!(rt.state.last_intent, Some(Intent::Clicked { id }) if id == EntryId::new(4)));

    rt.state.last_intent = None;
    let [x, y, w, h] = element(&rt, "row-6").unwrap().visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y + 20.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y + 20.0);
    rt.frame(1.0 / 60.0);
    assert!(
        rt.state.last_intent.is_none(),
        "a press that travelled isn't a click"
    );

    rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    click_at(&mut rt, x, y);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    assert!(
        rt.state.last_intent.is_none(),
        "a modified click isn't a plain click"
    );
}

/// N3: the pointer controller hears every key, taken or not.
#[test]
fn the_pointer_controller_sees_every_key_the_collection_hears() {
    let (mut rt, _, keys) = recorder(20, |_, _| {});
    click(&mut rt, "row-2");
    press(&mut rt, Key::End);
    press(&mut rt, Key::Function(2));
    assert_eq!(
        keys.get(),
        2,
        "one key the keyboard controller took, one it passed"
    );
}

/// N4: drop zones on folder rows don't take presses, so the controller still hears them.
#[test]
fn presses_on_rows_with_drop_zones_reach_the_controller() {
    fn with_drops(s: &Host, _: &mut Cx<Host>) -> El {
        col()
            .size(W, H)
            .child(collection(s).on_drop(|_: &mut Host, _, _, _| {}).build(s))
    }
    let mut rt = Runtime::new(Host::new(20), with_drops, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    // Entry 10 is a folder: its row carries a drop zone.
    click(&mut rt, "row-10");
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(10)));
}

/// N5: a controller can be tested with a `Ctx` and no runtime.
#[test]
fn a_controller_runs_against_a_ctx_without_a_runtime() {
    let src = FastSource::new(10);
    let geometry = OnePerLine.geometry(10, 26.0, 300.0, 200.0);
    let mut sel = crate::collection::selection::Selection::new();
    let mut cx = Ctx::new(
        &src,
        &*geometry,
        crate::contract::Rect::new(0.0, 0.0, 300.0, 200.0),
        0.0,
        &mut sel,
    );
    let hit = Hit {
        index: Some(3),
        region: Region::Name,
    };
    let down = PointerInput::new(
        PointerPhase::Down,
        rune::PointerButton::Primary,
        Point::new(40.0, 90.0),
        hit,
    );
    let up = PointerInput {
        phase: PointerPhase::Up,
        ..down
    };
    default_pointer(&down, &mut cx);
    default_pointer(&up, &mut cx);
    assert!(cx.changed().selection && cx.changed().cursor);
    assert!(
        matches!(cx.effects(), [Effect::Intent(Intent::Clicked { id })] if *id == EntryId::new(3))
    );
    drop(cx);
    assert_eq!(sel.cursor(), Some(EntryId::new(3)));
}

/// N6: a band begun by a move takes that move too.
#[test]
fn a_marquee_begun_by_a_move_hears_that_move() {
    let (mut rt, log, _) = recorder(40, |input, cx| {
        cx.emit(Effect::BeginMarquee {
            at: input.point,
            mods: input.mods,
        });
    });
    let [x, y, w, h] = element(&rt, "row-2").unwrap().visible;
    let (x, y) = (x + w - 20.0, y + h / 2.0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y + 40.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y + 40.0);
    rt.frame(1.0 / 60.0);
    assert_eq!(log.borrow().marquee, ["begin", "moved", "end"]);
}

/// N7: a drag intent carries the pointer's window point.
#[test]
fn a_drag_intent_carries_the_window_point() {
    let (mut rt, _, _) = recorder(40, |input, cx| {
        if let Some(i) = input.hit.index {
            let ids = vec![cx.source.id_at(i)];
            cx.emit(Effect::Intent(Intent::BeginDrag {
                ids,
                window: input.window,
            }));
        }
    });
    let [x, y, _, h] = element(&rt, "row-5").unwrap().visible;
    let (x, y) = (x + 40.0, y + h / 2.0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x + 12.0, y + 3.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x + 12.0, y + 3.0);
    rt.frame(1.0 / 60.0);
    match &rt.state.last_intent {
        Some(Intent::BeginDrag { ids, window }) => {
            assert_eq!(ids, &[EntryId::new(5)]);
            assert_eq!(*window, Point::new(x + 12.0, y + 3.0));
        }
        other => panic!("no drag intent: {other:?}"),
    }
}

// Review round 1.

/// Two panes over one folder, each with its own state in one `Vec`, reached through indices
/// the accessors capture (as C2's columns are).
struct Panes {
    src: FastSource,
    colls: Vec<CollectionState>,
    look: Look,
}

fn pane(i: usize) -> Collection<Panes> {
    Collection::new(
        ["a", "b"][i],
        move |s: &Panes| (&s.src, &s.colls[i]),
        move |s: &mut Panes| (&s.src, &mut s.colls[i]),
        |s: &Panes| &s.look,
        |_: &Panes| 1,
    )
}

fn panes_view(s: &Panes, _: &mut Cx<Panes>) -> El {
    rune::row()
        .size(W, H)
        .child(col().w(W / 2.0).h_full().child(pane(0).build(s)))
        .child(col().w(W / 2.0).h_full().child(pane(1).build(s)))
}

fn panes(n: usize) -> Runtime<Panes> {
    let state = Panes {
        src: FastSource::new(n),
        colls: vec![CollectionState::new(), CollectionState::new()],
        look: Look::fallback(false),
    };
    let mut rt = Runtime::new(state, panes_view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
    }
    rt
}

/// `key`'s element under the collection keyed `scope`.
fn element_in<S: 'static>(rt: &Runtime<S>, scope: &str, key: &str) -> Option<ElementInfo> {
    rt.elements().into_iter().find(|e| {
        !e.exiting && e.key.as_deref() == Some(key) && e.path.contains(&format!("/{scope}/"))
    })
}

/// The accessibility node keyed `key`, as JSON text without its children.
fn a11y_node<S: 'static>(rt: &Runtime<S>, key: &str) -> Option<String> {
    let mut stack = vec![rt.accessibility_json()];
    while let Some(mut v) = stack.pop() {
        if let Some(c) = v["children"].as_array() {
            stack.extend(c.iter().cloned());
        }
        if v["key"] == key {
            v["children"].take();
            return Some(v.to_string());
        }
    }
    None
}

/// B1: one host builds two collections over its own states, picked by captured indices.
#[test]
fn collections_with_captured_accessors_keep_their_own_state() {
    let mut rt = panes(30);
    let [x, y, w, h] = element_in(&rt, "b", "row-3").unwrap().visible;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.colls[1].selection.cursor(), Some(id(3)));
    assert_eq!(rt.state.colls[0].selection.cursor(), None);
}

/// S8: the active descendant names the row under its own collection: a second pane on the
/// same folder that shows the cursor's row doesn't answer for the first.
#[test]
fn the_active_descendant_is_the_row_under_its_own_collection() {
    let mut rt = panes(3_000);
    rt.update(|s, tx| {
        for (i, scope) in ["a", "b"].iter().enumerate() {
            s.colls[i].selection.select_only(&s.src, 2_000);
            if i == 1 {
                s.colls[i].reveal(scope, &s.src, 2_000, tx);
            }
        }
    });
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
    }
    assert!(element_in(&rt, "b", "row-2000").is_some());
    assert!(element_in(&rt, "a", "row-2000").is_none());
    let a = a11y_node(&rt, "a").unwrap();
    let b = a11y_node(&rt, "b").unwrap();
    assert!(
        !a.contains("activeDescendant"),
        "a's cursor row isn't mounted: {a}"
    );
    assert!(b.contains(r#""activeDescendant":"row-2000""#), "{b}");
}

/// A keyboard controller for the round-1 checks: F3 is taken and moves nothing, F4 moves the
/// cursor but turns keyboard mode off, `mod+a` (a layout-aware chord) selects all, Up at the
/// top is an edge, and its tick runs a queued command.
#[derive(Default)]
struct Nav {
    pointers: Rc<Cell<usize>>,
    queued: Rc<Cell<Option<usize>>>,
    settings: Rc<Cell<Option<u32>>>,
}

impl KeyController for Nav {
    fn key(&mut self, input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow {
        self.settings.set(cx.settings::<u32>().copied());
        let all = rune::Chord::parse("mod+a").unwrap();
        if input.matches(&all) {
            cx.apply(|sel, src| sel.select_all(src));
            return Flow::Handled;
        }
        match input.event.key {
            Key::Function(3) => Flow::Handled,
            Key::Function(4) => {
                cx.apply(|sel, src| sel.select_only(src, 0));
                cx.emit(Effect::KeyboardMode(false));
                Flow::Handled
            }
            Key::Up if cx.selection().cursor_index(cx.source) == Some(0) => {
                cx.emit(Effect::Intent(Intent::Edge(crate::contract::Direction::Up)));
                Flow::Handled
            }
            _ => Flow::Pass,
        }
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        match self.queued.take() {
            Some(i) => {
                cx.apply(|sel, src| sel.select_only(src, i));
                cx.emit(Effect::KeyboardMode(true));
                Flow::Handled
            }
            None => Flow::Pass,
        }
    }

    fn pointer_seen(&mut self, _input: &PointerInput) {
        self.pointers.set(self.pointers.get() + 1);
    }
}

/// What a test reads back from [`Nav`]: the pointer inputs it saw, its command queue and the
/// settings it read.
struct NavProbe {
    pointers: Rc<Cell<usize>>,
    queued: Rc<Cell<Option<usize>>>,
    settings: Rc<Cell<Option<u32>>>,
}

fn with_nav(n: usize) -> (Runtime<Host>, NavProbe) {
    let nav = Nav::default();
    let probe = NavProbe {
        pointers: nav.pointers.clone(),
        queued: nav.queued.clone(),
        settings: nav.settings.clone(),
    };
    let mut host = Host::new(n);
    host.coll.controllers.keyboard = Some(Box::new(nav));
    (runtime(host), probe)
}

/// S1: keyboard mode follows a key only when the cursor moved and the controller didn't say.
#[test]
fn keyboard_mode_turns_on_only_for_a_key_that_moves_the_cursor() {
    let (mut rt, ..) = with_nav(20);
    click(&mut rt, "row-5");
    press(&mut rt, Key::Function(3));
    assert!(!rt.state.coll.keyboard_mode(), "a key that moves nothing");
    press(&mut rt, Key::Function(4));
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(0)));
    assert!(!rt.state.coll.keyboard_mode(), "the controller said off");
}

/// C1c: an edge intent, the pointer seen by the keyboard controller, a layout-aware chord,
/// the host's settings, and a queued command run by the host's tick.
#[test]
fn the_keyboard_controller_hears_pointers_chords_settings_and_ticks() {
    let (mut rt, probe) = with_nav(500);
    let NavProbe {
        pointers,
        queued,
        settings,
    } = probe;
    click(&mut rt, "row-0");
    assert_eq!(pointers.get(), 2, "the press and the release");
    press(&mut rt, Key::Up);
    assert!(matches!(
        rt.state.last_intent,
        Some(Intent::Edge(crate::contract::Direction::Up))
    ));
    assert_eq!(settings.get(), Some(7));
    rt.key(KeyEvent {
        key: Key::Command('a'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.coll.selection.len(), 500, "mod+a matched");

    queued.set(Some(450));
    rt.update(|s, tx| {
        let c = collection(s);
        c.tick(s, tx);
    });
    settle(&mut rt);
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(450)));
    assert!(rt.state.coll.keyboard_mode());
    let row = element(&rt, "row-450").expect("the command's cursor is mounted");
    assert!(row.visible[3] > 20.0, "and shows");
}

/// S2 and S6: a press on a header part that takes presses itself isn't the collection's, nor
/// are its moves and release; the overlay sits over the viewport and takes no input.
#[test]
fn a_header_button_press_is_not_the_collections_and_the_overlay_passes_presses_through() {
    let log = Rc::new(RefCell::new(Log::default()));
    let mut host = Host::new(20);
    host.parts = true;
    host.coll.controllers.pointer = Some(Box::new(Pointer(log.clone())));
    let mut rt = runtime(host);
    let [hx, hy, hw, hh] = element(&rt, "head-button").unwrap().visible;
    rt.pointer_down(hx + hw / 2.0, hy + hh / 2.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(hx + hw / 2.0, hy + hh + 80.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(hx + hw / 2.0, hy + hh / 2.0);
    rt.frame(1.0 / 60.0);
    assert!(
        log.borrow().pointer.is_empty(),
        "{:?}",
        log.borrow().pointer.len()
    );
    assert_eq!(rt.state.app_keys, 1, "the button kept its press");

    let band = element(&rt, "band").unwrap();
    let viewport = element(&rt, VIEWPORT).unwrap();
    assert_eq!(
        band.rect, viewport.rect,
        "the overlay covers the viewport, below the header"
    );
    click_label(&mut rt, "row-2");
    assert_eq!(
        rt.state.coll.selection.cursor(),
        Some(id(2)),
        "through the overlay"
    );
}

/// A press on the header's blank space is the collection's: on empty space, at the viewport's
/// top edge, and its moves and release follow (C4's heading strip).
#[test]
fn a_press_on_blank_header_space_is_on_empty_space_at_the_viewports_top() {
    let log = Rc::new(RefCell::new(Log::default()));
    let mut host = Host::new(20);
    host.parts = true;
    host.coll.controllers.pointer = Some(Box::new(Pointer(log.clone())));
    let mut rt = runtime(host);
    let [hx, hy, hw, hh] = element(&rt, "head").unwrap().visible;
    let viewport = element(&rt, VIEWPORT).unwrap().visible;
    let x = hx + hw - 40.0;
    rt.pointer_down(x, hy + hh / 2.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, hy + hh + 80.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, hy + hh + 80.0);
    rt.frame(1.0 / 60.0);
    let log = log.borrow();
    let phases: Vec<_> = log.pointer.iter().map(|p| p.phase).collect();
    assert_eq!(
        phases,
        [PointerPhase::Down, PointerPhase::Move, PointerPhase::Up]
    );
    let down = log.pointer[0];
    assert_eq!(down.hit.index, None, "on no entry");
    assert_eq!(down.hit.region, Region::Gutter);
    assert_eq!(down.point.y, 0.0, "at the top of the content shown");
    assert_eq!(down.point.x, x - viewport[0]);
    assert_eq!(rt.state.app_keys, 0);
}

/// Only a primary press on blank header space is the collection's: the rule is for a band, and
/// a middle press there (C24's autoscroll, an open-in-new-tab) is no press on the collection.
#[test]
fn a_middle_press_on_blank_header_space_is_not_the_collections() {
    let log = Rc::new(RefCell::new(Log::default()));
    let mut host = Host::new(20);
    host.parts = true;
    host.coll.controllers.pointer = Some(Box::new(Pointer(log.clone())));
    let mut rt = runtime(host);
    let [hx, hy, hw, hh] = element(&rt, "head").unwrap().visible;
    let (x, y) = (hx + hw - 40.0, hy + hh / 2.0);
    rt.button_down(x, y, rune::PointerButton::Middle);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y + 80.0);
    rt.frame(1.0 / 60.0);
    rt.button_up(x, y + 80.0, rune::PointerButton::Middle);
    rt.frame(1.0 / 60.0);
    assert!(
        log.borrow().pointer.is_empty(),
        "{} inputs reached the controller",
        log.borrow().pointer.len()
    );
    assert!(
        rt.state.order.borrow().is_empty(),
        "nor the press hook: {:?}",
        rt.state.order.borrow()
    );
}

/// A build with no header forgets where the last one was, so a later press there isn't taken
/// for one on its blank space.
#[test]
fn a_build_with_no_header_forgets_its_rect() {
    let mut host = Host::new(20);
    host.parts = true;
    let mut rt = runtime(host);
    assert!(rt.state.coll.header.get().h > 0.0, "laid out");
    rt.update(|s, _| s.parts = false);
    settle(&mut rt);
    assert_eq!(rt.state.coll.header.get(), crate::contract::Rect::default());
}

/// With the real pointer and marquee controllers, a drag from blank header space into the rows
/// draws a band that selects the rows it crosses.
#[test]
fn a_drag_from_blank_header_space_bands_the_rows_it_crosses() {
    let mut host = Host::new(20);
    host.parts = true;
    host.coll.controllers.pointer =
        Some(Box::new(crate::collection::pointer::PointerIntent::new()));
    host.coll.controllers.marquee = Some(Box::new(crate::collection::marquee::Marquee::new()));
    let mut rt = runtime(host);
    let [hx, hy, hw, hh] = element(&rt, "head").unwrap().visible;
    let x = hx + hw - 40.0;
    let row_h = rt.state.look.row.list;
    rt.pointer_down(x, hy + hh / 2.0);
    rt.frame(1.0 / 60.0);
    for k in 1..=5 {
        rt.pointer_move(x - 5.0 * k as f32, hy + hh + row_h * 0.5 * k as f32);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(x - 25.0, hy + hh + row_h * 2.5);
    settle(&mut rt);
    let s = &rt.state;
    assert_eq!(
        s.coll.selection.selected_ids(&s.src),
        [id(0), id(1), id(2)],
        "the band ran from the viewport's top to the release"
    );
}

/// S3: moves of a drag that changes nothing run no view.
#[test]
fn drag_moves_that_change_nothing_run_no_view() {
    thread_local! { static VIEWS: Cell<usize> = const { Cell::new(0) }; }
    fn counted(s: &Host, cx: &mut Cx<Host>) -> El {
        VIEWS.with(|v| v.set(v.get() + 1));
        view(s, cx)
    }
    // Twice: with Rune's debug check of `live_only` (which commits one event a quarter second
    // after a commit to compare what it builds), then without it, counting views.
    for check in [true, false] {
        let mut rt = Runtime::new(Host::new(50), counted, (W, H), Env::new(false, 1.0));
        rt.check_live_only(check);
        rt.frame(0.0);
        settle(&mut rt);
        let [x, y, w, h] = element(&rt, "row-3").unwrap().visible;
        let (x, y) = (x + w / 2.0, y + h / 2.0);
        rt.pointer_down(x, y);
        rt.frame(1.0 / 60.0);
        let before = VIEWS.with(Cell::get);
        for k in 1..=20 {
            rt.pointer_move(x, y + k as f32 * 3.0);
            rt.frame(1.0 / 60.0);
        }
        let after_moves = VIEWS.with(Cell::get);
        rt.pointer_up(x, y + 60.0);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.live_only_warnings().is_empty(),
            "{:?}",
            rt.live_only_warnings()
        );
        if !check {
            assert_eq!(after_moves, before, "moves ran the view");
        }
    }
}

/// S5: in a grid, a sync keeps the cursor's cell where it shows, by its path under its line.
#[test]
fn a_sync_keeps_the_cursor_cell_where_it_shows_in_a_grid() {
    let mut host = Host::new(4_000);
    host.per_line = 4;
    let mut rt = runtime(host);
    settle(&mut rt);
    for _ in 0..20 {
        rt.wheel(W / 2.0, H / 2.0, -200.0);
        rt.frame(1.0 / 60.0);
    }
    settle(&mut rt);
    let cells: Vec<_> = rt
        .elements()
        .into_iter()
        .filter(|e| {
            !e.exiting && e.role == "gridcell" && e.visible[1] > 200.0 && e.visible[3] > 20.0
        })
        .collect();
    let target = cells[0].key.clone().unwrap();
    click_label(&mut rt, &target);
    assert_eq!(
        rt.state.coll.selection.cursor().map(row_key),
        Some(target.clone())
    );
    let shown = element(&rt, &target).unwrap().visible;
    // Remove a whole line between the top of the view and the cursor: the lines below keep
    // their keys, and the cursor's cell holds its place while the rows above it close up.
    rt.update(|s, tx| {
        let first = s.coll.visible_range().start + 4;
        s.src.remove(first..first + 4);
        s.coll
            .sync(SCOPE, &s.src, SyncRule::complete(CursorGone::Neighbor), tx);
    });
    settle(&mut rt);
    assert_eq!(element(&rt, &target).unwrap().visible, shown);
}

/// S7: the status line reads the selection only when an operation changed it.
#[test]
fn the_status_line_changes_with_the_selection_not_the_listing() {
    let (mut rt, ..) = with_nav(20);
    let status = |rt: &Runtime<Host>| element(rt, STATUS).and_then(|e| e.text).unwrap_or_default();
    click(&mut rt, "row-1");
    assert_eq!(status(&rt), "");
    rt.key(KeyEvent {
        key: Key::Command('a'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    rt.frame(1.0 / 60.0);
    assert_eq!(status(&rt), "20 items selected");
    rt.update(|s, tx| {
        s.src.insert_top(5);
        s.coll
            .sync(SCOPE, &s.src, SyncRule::complete(CursorGone::Neighbor), tx);
    });
    rt.frame(1.0 / 60.0);
    assert_eq!(
        status(&rt),
        "20 items selected",
        "a listing change announces nothing"
    );
}

/// S10: a user scroll, or a new layout, cancels a restore that waits for its listing.
#[test]
fn a_scroll_or_a_layout_switch_cancels_a_waiting_restore() {
    for by_scroll in [true, false] {
        let mut rt = runtime(Host::new(3_000));
        let position = {
            let s = &mut rt.state;
            s.coll.selection.select_only(&s.src, 7);
            s.coll.position(&s.src)
        };
        rt.update(move |s, tx| {
            s.coll
                .enter(SCOPE, &s.src, Some(&position), Listing::Loading, tx);
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.state.coll.restore_pending());
        if by_scroll {
            for _ in 0..5 {
                rt.wheel(W / 2.0, H / 2.0, -200.0);
                rt.frame(1.0 / 60.0);
            }
        } else {
            rt.state.coll.set_layout_key(9);
        }
        assert!(!rt.state.coll.restore_pending());
    }
}

/// S11: a read-only collection draws no rename editor and tells controllers so.
#[test]
fn a_read_only_collection_draws_no_rename_and_says_so() {
    struct Probe(Rc<Cell<Option<bool>>>);
    impl PointerController for Probe {
        fn pointer(&mut self, _: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
            self.0.set(Some(cx.read_only()));
            Flow::Handled
        }
    }
    let seen = Rc::new(Cell::new(None));
    let mut host = Host::new(10);
    host.read_only = true;
    host.coll.controllers.pointer = Some(Box::new(Probe(seen.clone())));
    let mut rt = runtime(host);
    assert!(element(&rt, "editor").is_none());
    click(&mut rt, "row-4");
    assert_eq!(seen.get(), Some(true));
}

/// S12: an entry's window rect, for a keyboard menu at the cursor, is where its row shows.
#[test]
fn an_entry_window_rect_is_where_its_row_shows() {
    let mut rt = runtime(Host::new(500));
    for _ in 0..10 {
        rt.wheel(W / 2.0, H / 2.0, -200.0);
        rt.frame(1.0 / 60.0);
    }
    settle(&mut rt);
    let row = rows(&rt)
        .into_iter()
        .find(|e| e.visible[1] > 100.0 && e.visible[3] > 20.0)
        .unwrap();
    let index = rt
        .state
        .src
        .index_of(EntryId::new(
            row.key.as_ref().unwrap()[4..].parse().unwrap(),
        ))
        .unwrap();
    let rect = collection(&rt.state).window_rect_of(&rt.state, index);
    assert_eq!([rect.x, rect.y, rect.w, rect.h], row.rect);
}

/// S13: a grid's count per line follows the viewport's width, once the first layout gives it.
#[test]
fn a_grid_reflows_to_the_viewport_width() {
    struct ByWidth;
    impl Lines for ByWidth {
        fn per_line(&self, width: f32) -> usize {
            ((width / 100.0) as usize).max(1)
        }
        fn geometry(&self, len: usize, h: f32, w: f32, vh: f32) -> Box<dyn CollectionGeometry> {
            OnePerLine.geometry(len.div_ceil(self.per_line(w)), h, w, vh)
        }
    }
    fn grid(s: &Host, _: &mut Cx<Host>) -> El {
        col().size(W, H).child(
            collection(s)
                .lines(ByWidth)
                .roles(Roles::GRID_CELLS)
                .build(s),
        )
    }
    let mut rt = Runtime::new(Host::new(23), grid, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    settle(&mut rt);
    let lines = rt.elements().iter().filter(|e| e.role == "row").count();
    assert_eq!(lines, 5, "23 entries, 5 a line at {W} pt");
}

/// N4: showing another folder ends an open band and keyboard mode.
#[test]
fn entering_a_folder_ends_gestures_and_keyboard_mode() {
    let (mut rt, log) = with_controllers(40);
    rt.update(|s, _| {
        s.coll.keyboard_moved();
    });
    let [x, y, _, h] = element(&rt, "row-2").unwrap().visible;
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_down(x + 30.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    assert_eq!(log.borrow().marquee, ["begin"]);
    rt.update(|s, tx| {
        s.coll.enter(SCOPE, &s.src, None, Listing::Complete, tx);
    });
    rt.frame(1.0 / 60.0);
    assert!(!rt.state.coll.controllers.marquee.as_ref().unwrap().active());
    assert!(!rt.state.coll.keyboard_mode());
    rt.pointer_up(x + 30.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
}

/// C1d G1: a glide effect scrolls on its spring.
#[test]
fn a_glide_effect_scrolls_the_viewport() {
    struct Glider;
    impl PointerController for Glider {
        fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
            if input.phase == PointerPhase::Down {
                cx.emit(Effect::Glide {
                    y: 2_000.0,
                    spring: rune::Spring::new(0.2, 0.0),
                });
            }
            Flow::Handled
        }
    }
    let mut host = Host::new(500);
    host.coll.controllers.pointer = Some(Box::new(Glider));
    let mut rt = runtime(host);
    click(&mut rt, "row-2");
    rt.frame(1.0 / 60.0);
    let early = rt.state.coll.scroll_offset();
    settle(&mut rt);
    assert!(early < 2_000.0, "it glides: {early}");
    assert_eq!(rt.state.coll.scroll_offset(), 2_000.0);
}

/// C1d G2: the marquee's own overlay draws in content coordinates, rides the scroll and takes
/// no input. G4: the content's extent.
#[test]
fn the_marquees_overlay_rides_the_scroll_and_passes_presses_through() {
    let (mut rt, _log) = with_controllers(200);
    let [x, y, _, h] = element(&rt, "row-2").unwrap().visible;
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_down(x + 30.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    rt.frame(1.0 / 60.0);
    let viewport = element(&rt, VIEWPORT).unwrap().rect;
    let band = element(&rt, "drawn-band").expect("the band draws").rect;
    assert_eq!(band[1], viewport[1] + 100.0, "at content y 100, unscrolled");
    rt.pointer_up(x + 30.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);

    // Open it again, scroll, and it moves with the content.
    rt.update(|s, tx| {
        s.coll.controllers.marquee.as_mut().unwrap().reset();
        tx.scroll_to_instant(viewport_path(SCOPE), 0.0, 40.0);
    });
    settle(&mut rt);
    let [x, y, _, h] = rows(&rt)
        .into_iter()
        .find(|e| e.visible[1] > 60.0 && e.visible[3] > 20.0)
        .unwrap()
        .visible;
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_down(x + 30.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    rt.frame(1.0 / 60.0);
    let band = element(&rt, "drawn-band").unwrap().rect;
    assert_eq!(
        band[1],
        viewport[1] + 100.0 - 40.0,
        "scrolled 40 pt up with the content"
    );
    rt.pointer_up(x + 30.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
}

#[test]
fn the_content_extent_ends_at_the_last_entry() {
    let src = FastSource::new(10);
    let geometry = OnePerLine.geometry(10, 26.0, 300.0, 200.0);
    let mut sel = crate::collection::selection::Selection::new();
    let cx = Ctx::new(
        &src,
        &*geometry,
        crate::contract::Rect::new(0.0, 0.0, 300.0, 200.0),
        0.0,
        &mut sel,
    );
    assert_eq!(cx.content_extent(), 260.0);
}

/// C1d G3: showing a folder stops an auto-scroll glide, by instant scrolls only.
#[test]
fn entering_a_folder_stops_a_glide() {
    for remembered in [false, true] {
        let mut rt = runtime(Host::new(3_000));
        let position = {
            let s = &mut rt.state;
            s.coll.selection.select_only(&s.src, 3);
            s.coll.set_layout_key(1);
            s.coll.position(&s.src)
        };
        rt.update(|_, tx| {
            tx.scroll_to_with(
                viewport_path(SCOPE),
                0.0,
                30_000.0,
                rune::Spring::new(1.0, 0.0),
            )
        });
        for _ in 0..5 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.state.coll.scroll_offset() > 0.0, "gliding");
        rt.update(move |s, tx| {
            // In another layout, so the cursor (in view at the top) is revealed, not an offset.
            s.coll.set_layout_key(2);
            let p = remembered.then_some(&position);
            s.coll.enter(SCOPE, &s.src, p, Listing::Complete, tx);
        });
        settle(&mut rt);
        let at = rt.state.coll.scroll_offset();
        assert!(
            at < 400.0,
            "the glide stopped (remembered {remembered}): {at}"
        );
    }
}

// Review round 2.

/// A marquee as C1d builds it: its band rides a live value, so moves change nothing the view
/// reads; it repaints only to show and hide; each tick glides and asks for the next frame.
struct AutoBand {
    on: bool,
    moves: Rc<Cell<usize>>,
}

impl MarqueeController for AutoBand {
    fn begin(&mut self, _at: Point, _mods: Mods, cx: &mut Ctx<'_>) {
        self.on = true;
        cx.repaint();
    }
    fn moved(&mut self, _at: Point, _cx: &mut Ctx<'_>) {
        self.moves.set(self.moves.get() + 1);
    }
    fn end(&mut self, cx: &mut Ctx<'_>) {
        self.on = false;
        cx.repaint();
    }
    fn cancel(&mut self, _cx: &mut Ctx<'_>) -> bool {
        std::mem::replace(&mut self.on, false)
    }
    fn active(&self) -> bool {
        self.on
    }
    fn band(&self) -> Option<crate::contract::Rect> {
        None
    }
    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        if self.on {
            cx.emit(Effect::Glide {
                y: cx.visible.y + 40.0,
                spring: rune::Spring::new(0.3, 0.0),
            });
            cx.emit(Effect::WakeAfter(std::time::Duration::ZERO));
        }
        Flow::Pass
    }
}

thread_local! { static AUTO_VIEWS: Cell<usize> = const { Cell::new(0) }; }

fn auto_view(s: &Host, cx: &mut Cx<Host>) -> El {
    AUTO_VIEWS.with(|v| v.set(v.get() + 1));
    view(s, cx)
}

fn auto_poll(s: &mut Host, _t: f64, tx: &mut rune::Tx) -> bool {
    collection(s).tick(s, tx)
}

/// B1: an open band's moves and its auto-scroll ticks run no view, while the glide still
/// scrolls. Run with Rune's debug `live_only` check (no warnings), then without it, counting.
#[test]
fn a_band_that_rides_live_values_runs_no_view_while_it_moves_and_scrolls() {
    for check in [true, false] {
        let moves = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Log::default()));
        let mut host = Host::new(100_000);
        host.coll.controllers.pointer = Some(Box::new(Pointer(log)));
        host.coll.controllers.marquee = Some(Box::new(AutoBand {
            on: false,
            moves: moves.clone(),
        }));
        let mut rt = Runtime::new(host, auto_view, (W, H), Env::new(false, 1.0));
        rt.check_live_only(check);
        rt.poll = Some(auto_poll);
        rt.frame(0.0);
        settle(&mut rt);
        let [x, y, _, h] = element(&rt, "row-2").unwrap().visible;
        rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
            shift: true,
            ..Mods::default()
        }));
        rt.pointer_down(x + 30.0, y + h / 2.0);
        rt.frame(1.0 / 60.0);
        rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
        assert!(rt.state.coll.controllers.marquee.as_ref().unwrap().active());

        let before = AUTO_VIEWS.with(Cell::get);
        for k in 1..=30 {
            rt.pointer_move(x + 30.0, y + h / 2.0 + k as f32 * 2.0);
            rt.frame(1.0 / 60.0);
        }
        let after_moves = AUTO_VIEWS.with(Cell::get);
        let from = rt.state.coll.scroll_offset();
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        let after_ticks = AUTO_VIEWS.with(Cell::get);
        assert!(moves.get() >= 30, "the band heard its moves");
        assert!(
            rt.state.coll.scroll_offset() > from + 100.0,
            "and the glide scrolled"
        );
        assert!(
            rt.live_only_warnings().is_empty(),
            "{:?}",
            rt.live_only_warnings()
        );
        if !check {
            assert_eq!(after_moves, before, "band moves ran the view");
            assert_eq!(after_ticks, after_moves, "auto-scroll ticks ran the view");
        }
        rt.pointer_up(x + 30.0, y + 200.0);
        rt.frame(1.0 / 60.0);
        assert!(
            AUTO_VIEWS.with(Cell::get) > after_ticks,
            "hiding the band repaints"
        );
    }
}

/// A band that selects the entries above the pointer, as C1d's does.
struct Selecting(bool);
impl MarqueeController for Selecting {
    fn begin(&mut self, _: Point, _: Mods, cx: &mut Ctx<'_>) {
        self.0 = true;
        cx.apply(|sel, _| sel.begin_band(crate::collection::selection::BandMode::Replace));
    }
    fn moved(&mut self, at: Point, cx: &mut Ctx<'_>) {
        let crossed = cx
            .geometry
            .range_in(crate::contract::Rect::new(0.0, 0.0, 10.0, at.y));
        cx.apply(|sel, src| sel.band(src, &crossed));
    }
    fn end(&mut self, cx: &mut Ctx<'_>) {
        self.0 = false;
        cx.apply(|sel, _| {
            sel.end_band();
            crate::collection::selection::Changed::NONE
        });
    }
    fn cancel(&mut self, _: &mut Ctx<'_>) -> bool {
        std::mem::replace(&mut self.0, false)
    }
    fn active(&self) -> bool {
        self.0
    }
    fn band(&self) -> Option<crate::contract::Rect> {
        None
    }
}

/// Nit: an open band's selection is announced when its gesture ends, not at each step.
#[test]
fn a_band_announces_its_count_when_it_ends() {
    let log = Rc::new(RefCell::new(Log::default()));
    let mut host = Host::new(40);
    host.coll.controllers.pointer = Some(Box::new(Pointer(log)));
    host.coll.controllers.marquee = Some(Box::new(Selecting(false)));
    let mut rt = runtime(host);
    let status = |rt: &Runtime<Host>| element(rt, STATUS).and_then(|e| e.text).unwrap_or_default();
    let [x, y, _, h] = element(&rt, "row-1").unwrap().visible;
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_down(x + 30.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    for k in 1..=6 {
        rt.pointer_move(x + 30.0, y + h / 2.0 + k as f32 * 26.0);
        rt.frame(1.0 / 60.0);
        assert_eq!(status(&rt), "", "no count while the band moves");
    }
    assert!(rt.state.coll.selection.len() > 3);
    rt.pointer_up(x + 30.0, y + h / 2.0 + 6.0 * 26.0);
    rt.frame(1.0 / 60.0);
    let n = rt.state.coll.selection.len();
    assert_eq!(status(&rt), format!("{n} items selected"));
}

/// A listing change synced while a band is open doesn't announce the band's count: it waits
/// for the band to end (C1d review S3).
#[test]
fn a_sync_during_an_open_band_keeps_the_status_line_quiet() {
    let log = Rc::new(RefCell::new(Log::default()));
    let mut host = Host::new(40);
    host.coll.controllers.pointer = Some(Box::new(Pointer(log)));
    host.coll.controllers.marquee = Some(Box::new(Selecting(false)));
    let mut rt = runtime(host);
    let status = |rt: &Runtime<Host>| element(rt, STATUS).and_then(|e| e.text).unwrap_or_default();
    let [x, y, _, h] = element(&rt, "row-1").unwrap().visible;
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_down(x + 30.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    for k in 1..=4 {
        rt.pointer_move(x + 30.0, y + h / 2.0 + k as f32 * 26.0);
        rt.frame(1.0 / 60.0);
    }
    assert!(rt.state.coll.selection.len() > 2);
    rt.update(|s, tx| {
        s.src.insert_top(2);
        s.coll
            .sync(SCOPE, &s.src, SyncRule::complete(CursorGone::Neighbor), tx);
    });
    rt.frame(1.0 / 60.0);
    assert_eq!(status(&rt), "", "no count mid-band");
    rt.pointer_up(x + 30.0, y + h / 2.0 + 4.0 * 26.0);
    rt.frame(1.0 / 60.0);
    let n = rt.state.coll.selection.len();
    assert_eq!(status(&rt), format!("{n} items selected"), "once it ends");
}

/// Controllers read whether motion is on from the look the element builds with (C1d review
/// S2).
#[test]
fn controllers_read_whether_animations_are_on() {
    struct Reads(Rc<RefCell<Vec<bool>>>);
    impl PointerController for Reads {
        fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
            if input.phase == PointerPhase::Down {
                self.0.borrow_mut().push(cx.animations());
            }
            Flow::Handled
        }
    }
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut host = Host::new(5);
    host.coll.controllers.pointer = Some(Box::new(Reads(seen.clone())));
    let mut rt = runtime(host);
    click(&mut rt, "row-1");
    rt.update(|s, _| s.look.animations = false);
    settle(&mut rt);
    click(&mut rt, "row-2");
    assert_eq!(*seen.borrow(), [true, false]);
}

/// B2: a row height that changes through the state draws rows at it, and presses hit the
/// entries drawn there.
#[test]
fn a_row_height_from_the_state_redraws_rows_and_routes_presses() {
    let mut host = Host::new(50);
    host.row_h = Some(26.0);
    let mut rt = runtime(host);
    settle(&mut rt);
    click(&mut rt, "row-3");
    rt.update(|s, _| s.row_h = Some(40.0));
    settle(&mut rt);
    for key in ["row-5", "row-8"] {
        click(&mut rt, key);
        assert_eq!(
            rt.state.coll.selection.cursor().map(row_key).as_deref(),
            Some(key)
        );
    }
}

/// B3: a tile size changed at the same width reflows the grid at once: the lines and where
/// presses land follow it.
#[test]
fn a_tile_size_change_at_the_same_width_reflows() {
    let mut host = Host::new(23);
    host.tile = Some(100.0);
    let mut rt = runtime(host);
    settle(&mut rt);
    let lines = |rt: &Runtime<Host>| {
        rt.elements()
            .iter()
            .filter(|e| !e.exiting && e.role == "row")
            .count()
    };
    assert_eq!(lines(&rt), 5, "5 tiles a line at 500 pt");
    rt.update(|s, _| s.tile = Some(250.0));
    settle(&mut rt);
    assert_eq!(lines(&rt), 12, "2 tiles a line");
    for key in ["row-7", "row-14"] {
        click_label(&mut rt, key);
        assert_eq!(
            rt.state.coll.selection.cursor().map(row_key).as_deref(),
            Some(key)
        );
    }
}

/// S2: a reflow, by a narrower viewport or a new tile size, scrolls the cursor into view.
#[test]
fn a_reflow_scrolls_the_cursor_into_view() {
    for by_width in [true, false] {
        let mut host = Host::new(4_000);
        host.tile = Some(100.0);
        let mut rt = runtime(host);
        settle(&mut rt);
        for _ in 0..20 {
            rt.wheel(W / 4.0, H / 2.0, -200.0);
            rt.frame(1.0 / 60.0);
        }
        settle(&mut rt);
        let key = rt
            .elements()
            .into_iter()
            .find(|e| {
                !e.exiting && e.role == "gridcell" && e.visible[1] > 150.0 && e.visible[3] > 20.0
            })
            .and_then(|e| e.key)
            .unwrap();
        click_label(&mut rt, &key);
        if by_width {
            rt.update(|s, _| s.width = 250.0);
        } else {
            rt.update(|s, tx| {
                s.tile = Some(250.0);
                collection(s).reflow(s, tx);
            });
        }
        settle(&mut rt);
        let shown = element(&rt, &key)
            .expect("the cursor's cell is mounted")
            .visible;
        assert!(
            shown[3] > 20.0,
            "and shows whole (by width {by_width}): {shown:?}"
        );
    }
}

// C1e round-3 carry-overs (C1f).

/// SF1: a new grid entered with a remembered position before its first layout keeps the
/// remembered scroll: the first width isn't a reflow that reveals the cursor instead.
#[test]
fn a_fresh_grid_keeps_a_restored_scroll_through_its_first_layout() {
    let grid = || {
        let mut host = Host::new(4_000);
        host.tile = Some(100.0);
        host
    };
    let mut rt = runtime(grid());
    settle(&mut rt);
    click_label(&mut rt, "row-7");
    for _ in 0..20 {
        rt.wheel(W / 4.0, H / 2.0, -200.0);
        rt.frame(1.0 / 60.0);
    }
    settle(&mut rt);
    let offset = rt.state.coll.scroll_offset();
    assert!(offset > 1_000.0, "scrolled away from the cursor: {offset}");
    let position = rt.state.coll.position(&rt.state.src);

    for before_first_frame in [true, false] {
        let mut rt = Runtime::new(grid(), view, (W, H), Env::new(false, 1.0));
        if !before_first_frame {
            rt.frame(0.0);
        }
        let p = position.clone();
        rt.update(move |s, tx| {
            s.coll.enter(SCOPE, &s.src, Some(&p), Listing::Complete, tx);
        });
        rt.frame(0.0);
        settle(&mut rt);
        assert_eq!(
            rt.state.coll.scroll_offset(),
            offset,
            "entered before the first frame: {before_first_frame}"
        );
        assert_eq!(rt.state.coll.selection.cursor(), Some(id(7)));
    }
}

/// SF1: a new grid entered before its first layout with a position whose scroll was measured
/// in another layout shows the cursor's cell, revealed once the lines are known.
#[test]
fn a_fresh_grid_restore_in_another_layout_reveals_the_cursor() {
    let position = {
        let mut host = Host::new(4_000);
        host.tile = Some(100.0);
        let mut rt = runtime(host);
        settle(&mut rt);
        rt.update(|s, _| {
            s.coll.selection.select_only(&s.src, 2_000);
        });
        rt.state.coll.position(&rt.state.src)
    };
    let mut host = Host::new(4_000);
    host.tile = Some(100.0);
    host.coll.set_layout_key(5);
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    rt.update(move |s, tx| {
        s.coll
            .enter(SCOPE, &s.src, Some(&position), Listing::Complete, tx);
    });
    rt.frame(0.0);
    settle(&mut rt);
    let key = row_key(id(2_000));
    let shown = element(&rt, &key)
        .expect("the cursor's cell is mounted")
        .visible;
    assert!(shown[3] > 20.0, "and shows whole: {shown:?}");
}

/// A keyboard controller whose tick says keyboard mode is on, and changes nothing else.
struct ModeOn;

impl KeyController for ModeOn {
    fn key(&mut self, _input: &KeyInput<'_>, _cx: &mut Ctx<'_>) -> Flow {
        Flow::Pass
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        cx.emit(Effect::KeyboardMode(true));
        Flow::Handled
    }
}

/// A keyboard-mode effect commits only when it changes the mode.
#[test]
fn a_keyboard_mode_effect_that_changes_nothing_commits_nothing() {
    let mut host = Host::new(20);
    host.coll.controllers.keyboard = Some(Box::new(ModeOn));
    let mut rt = runtime(host);
    let mut ticks = Vec::new();
    for _ in 0..2 {
        let shows = rt.update_if(|s, tx| collection(s).tick(s, tx));
        ticks.push(shows);
    }
    assert!(rt.state.coll.keyboard_mode());
    assert_eq!(ticks, [true, false], "on, then on again");
}

/// Rows built for one builder setting are built again when it changes: the roles, and
/// read-only (which takes the rows' drop zones away).
#[test]
fn switching_roles_or_read_only_rebuilds_the_rows_that_show() {
    let mut host = Host::new(20);
    host.drops = true;
    let mut rt = runtime(host);
    assert_eq!(element(&rt, "row-3").unwrap().role, "option");
    rt.update(|s, _| s.roles = Some(Roles::GRID_ROWS));
    settle(&mut rt);
    assert_eq!(element(&rt, "row-3").unwrap().role, "row");

    // Entry 0 is a folder: a file drag over it hovers its row.
    let [x, y, w, h] = element(&rt, "row-0").unwrap().visible;
    let at = (x + w / 2.0, y + h / 2.0);
    rt.drag_files(vec!["/tmp/a.txt".into()], Some(at));
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.coll.drop_over(), Some(DropOver::Row(id(0))));
    rt.drag_files_leave();
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| {
        s.read_only = true;
        s.coll.set_drop_over(None);
    });
    settle(&mut rt);
    rt.drag_files(vec!["/tmp/a.txt".into()], Some(at));
    rt.frame(1.0 / 60.0);
    assert_eq!(
        rt.state.coll.drop_over(),
        None,
        "a read-only row takes no drop"
    );
}

/// Each controller reads its own type from the builder's settings, the last value of a type
/// winning.
#[test]
fn each_controller_reads_its_own_settings_from_the_builder() {
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct A(u32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct B(u32);
    type Seen = Rc<RefCell<Vec<String>>>;
    struct ReadsA(Seen);
    impl PointerController for ReadsA {
        fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
            if input.phase == PointerPhase::Down {
                let read = cx.settings::<A>().copied();
                self.0.borrow_mut().push(format!("pointer {read:?}"));
            }
            Flow::Handled
        }
    }
    struct ReadsB(Seen);
    impl KeyController for ReadsB {
        fn key(&mut self, _input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow {
            let read = cx.settings::<B>().copied();
            self.0.borrow_mut().push(format!("keys {read:?}"));
            Flow::Handled
        }
    }
    fn with_settings(s: &Host, _: &mut Cx<Host>) -> El {
        col().size(W, H).child(
            collection(s)
                .settings(A(1))
                .settings(B(2))
                .settings(A(3))
                .build(s),
        )
    }
    let seen: Seen = Rc::default();
    let mut host = Host::new(10);
    host.coll.controllers.pointer = Some(Box::new(ReadsA(seen.clone())));
    host.coll.controllers.keyboard = Some(Box::new(ReadsB(seen.clone())));
    let mut rt = Runtime::new(host, with_settings, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    click(&mut rt, "row-2");
    press(&mut rt, Key::Down);
    assert_eq!(*seen.borrow(), ["pointer Some(A(3))", "keys Some(B(2))"]);
}

// Group header lines (C1f).

/// `n` entries in groups of `size`, keyed by their number and labelled "Group {g}".
fn grouped(n: usize, size: usize) -> Host {
    let mut host = Host::new(n);
    host.coll.set_groups(groups_of(n, size, "Group"));
    host
}

fn groups_of(n: usize, size: usize, label: &str) -> Groups {
    Groups::new(
        (0..n.div_ceil(size))
            .map(|g| Group::new(g as u64, format!("{label} {g}"), size.min(n - g * size))),
    )
}

/// Scrolls the viewport to content y `y` and lets it settle.
fn scroll_to(rt: &mut Runtime<Host>, y: f32) {
    rt.update(move |_, tx| tx.scroll_to_instant(keys::viewport_path(SCOPE), 0.0, y));
    settle(rt);
}

fn row_h(rt: &Runtime<Host>) -> f32 {
    rt.state.look.row.list
}

/// The stuck header's label and its part's visible rect.
fn stuck(rt: &Runtime<Host>) -> (Option<String>, [f32; 4]) {
    let elements = rt.elements();
    let part = elements
        .iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(STUCK))
        .expect("a stuck header");
    let label = elements
        .iter()
        .find(|e| e.path.ends_with(&format!("{STUCK}/{LABEL}")))
        .and_then(|e| e.text.clone());
    (label, part.visible)
}

#[test]
fn grouped_lines_put_a_heading_above_each_group_and_rows_keep_their_entry_positions() {
    let mut rt = runtime(grouped(100, 10));
    click(&mut rt, "row-12");
    let head = a11y_node(&rt, &header_key(1)).expect("group 1's header is in the tree");
    assert!(head.contains(r#""role":"heading""#), "{head}");
    assert!(head.contains(r#""name":"Group 1""#), "{head}");
    let row = a11y_node(&rt, "row-12").unwrap();
    assert!(
        row.contains(r#""position":13"#),
        "its entry's place, not its line's: {row}"
    );
    let rows = a11y_node(&rt, ROWS).unwrap();
    assert!(
        rows.contains(r#""setSize":100"#),
        "the entries, not the lines: {rows}"
    );
    let list = a11y_node(&rt, SCOPE).unwrap();
    assert!(list.contains(r#""activeDescendant":"row-12""#), "{list}");
    assert!(
        a11y_node(&rt, STUCK).is_none(),
        "the stuck copy is hidden from assistive technology"
    );
}

#[test]
fn a_press_on_a_header_line_or_the_stuck_header_is_on_no_entry() {
    let mut rt = runtime(grouped(100, 10));
    click(&mut rt, "row-3");
    assert_eq!(rt.state.coll.selection.len(), 1);
    click(&mut rt, &header_key(1));
    assert!(
        rt.state.coll.selection.is_empty(),
        "a header line is empty space"
    );

    // Entry 4's line at the top, under the stuck header: a press there is on no entry.
    let h = row_h(&rt);
    scroll_to(&mut rt, 5.0 * h);
    click(&mut rt, "row-6");
    assert_eq!(rt.state.coll.selection.len(), 1);
    let vp = rt.state.coll.viewport();
    click_at(&mut rt, vp.x + 40.0, vp.y + h / 2.0);
    assert!(rt.state.coll.selection.is_empty(), "the stuck header");
}

#[test]
fn a_press_just_below_the_stuck_header_hits_the_entry_drawn_there() {
    let mut rt = runtime(grouped(100, 10));
    let h = row_h(&rt);
    scroll_to(&mut rt, 5.0 * h);
    let vp = rt.state.coll.viewport();
    click_at(&mut rt, vp.x + 40.0, vp.y + h + h / 2.0);
    assert_eq!(
        rt.state.coll.selection.cursor(),
        Some(id(5)),
        "line 6: entry 5"
    );
    let shown = element(&rt, "row-5").unwrap().visible;
    assert!(
        shown[1] >= vp.y + h - 0.5,
        "drawn below the stuck header: {shown:?}"
    );
}

/// Up and Down move the cursor one entry; the element reveals it.
struct Step;

impl KeyController for Step {
    fn key(&mut self, input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow {
        let Some(i) = cx.selection().cursor_index(cx.source) else {
            return Flow::Pass;
        };
        let to = match input.event.key {
            Key::Up => i.saturating_sub(1),
            Key::Down => (i + 1).min(cx.source.len() - 1),
            _ => return Flow::Pass,
        };
        cx.apply(|sel, src| sel.select_only(src, to));
        Flow::Handled
    }
}

#[test]
fn revealing_an_entry_keeps_it_clear_of_the_stuck_header() {
    let mut rt = runtime(grouped(1_000, 10));
    let h = row_h(&rt);
    let vh = rt.state.coll.viewport().h;
    let line_of = |i: usize| (i + i / 10 + 1) as f32;
    let reveal = |rt: &mut Runtime<Host>, i: usize| {
        rt.update(move |s, tx| s.coll.reveal(SCOPE, &s.src, i, tx));
        settle(rt);
        rt.state.coll.scroll_offset()
    };
    scroll_to(&mut rt, 900.0 * h);
    assert_eq!(
        reveal(&mut rt, 500),
        (line_of(500) - 1.0) * h,
        "its header at the top"
    );
    scroll_to(&mut rt, 900.0 * h);
    assert_eq!(
        reveal(&mut rt, 503),
        (line_of(503) - 1.0) * h,
        "the entry above at the top"
    );
    assert_eq!(reveal(&mut rt, 0), 0.0, "the first header shows");
    assert_eq!(
        reveal(&mut rt, 80) + vh,
        (line_of(80) + 1.0) * h,
        "going down, the entry at the bottom"
    );

    // A key that moves the cursor up onto the line the stuck header covers.
    let mut host = grouped(1_000, 10);
    host.coll.controllers.keyboard = Some(Box::new(Step));
    let mut rt = runtime(host);
    scroll_to(&mut rt, line_of(504) * h);
    let vp = rt.state.coll.viewport();
    click_at(&mut rt, vp.x + 40.0, vp.y + h + h / 2.0);
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(505)));
    press(&mut rt, Key::Up);
    settle(&mut rt);
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(504)));
    let shown = element(&rt, "row-504").unwrap().visible;
    assert!(
        shown[1] >= vp.y + h - 0.5 && shown[3] > h - 0.5,
        "whole, below the stuck header: {shown:?}"
    );
}

/// The stuck header shows the top group's header, pushed up by the next group's: the push
/// rides the scroll, and a new group's label costs one view run per group crossed (until it
/// lands the stuck header hides, and the new group's own header line shows at the top).
#[test]
fn the_stuck_header_follows_the_top_group_and_the_next_pushes_it_out() {
    thread_local! {
        static VIEWS: Cell<usize> = const { Cell::new(0) };
    }
    fn counted(s: &Host, cx: &mut Cx<Host>) -> El {
        VIEWS.with(|v| v.set(v.get() + 1));
        view(s, cx)
    }
    let mut rt = Runtime::new(grouped(1_000, 10), counted, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    settle(&mut rt);
    let h = row_h(&rt);
    let vp = rt.state.coll.viewport();
    assert_eq!(stuck(&rt).0.as_deref(), Some("Group 0"));
    let views = VIEWS.with(Cell::get);
    let (mut pushed, mut hidden) = (0, 0);
    let mut groups = std::collections::BTreeSet::new();
    for _ in 0..200 {
        rt.wheel(W / 2.0, H / 2.0, -7.0);
        rt.frame(1.0 / 60.0);
        let offset = rt.state.coll.scroll_offset();
        let top = (offset / h) as usize / 11;
        let next = ((top + 1) * 11) as f32 * h;
        let extent = (next - offset).clamp(0.0, h);
        let (label, rect) = stuck(&rt);
        groups.insert(top);
        if rect[3] < 0.5 {
            hidden += 1;
            continue;
        }
        assert_eq!(label, Some(format!("Group {top}")), "at {offset}");
        assert!(
            (rect[1] + rect[3] - (vp.y + extent)).abs() < 0.6,
            "pushed to {extent} at {offset}: {rect:?}"
        );
        pushed += usize::from(extent < h - 0.5);
    }
    let crossed = groups.len() - 1;
    assert!(crossed >= 3, "crossed groups: {groups:?}");
    assert!(pushed > 0, "seen pushed");
    assert!(
        hidden <= crossed,
        "hidden {hidden} frames for {crossed} crossings"
    );
    assert_eq!(
        VIEWS.with(Cell::get) - views,
        crossed,
        "a view run per group crossed, none per frame"
    );
}

/// A regroup at the same listing version and the same number of lines draws the new groups.
#[test]
fn a_regroup_rekeys_the_lines() {
    let mut rt = runtime(grouped(100, 10));
    assert!(element(&rt, &header_key(1)).is_some());
    rt.update(|s, _| {
        s.coll.set_groups(Groups::new((0..10).map(|g| {
            Group::new(g, format!("Big {g}"), if g % 2 == 0 { 5 } else { 15 })
        })));
    });
    settle(&mut rt);
    let head = a11y_node(&rt, &header_key(0)).unwrap();
    assert!(head.contains(r#""name":"Big 0""#), "{head}");
    assert_eq!(stuck(&rt).0.as_deref(), Some("Big 0"), "without a scroll");
    let top = element(&rt, &header_key(0)).unwrap().visible[1];
    let [_, y, _, _] = element(&rt, &header_key(1)).unwrap().visible;
    assert!(
        (y - top - 6.0 * row_h(&rt)).abs() < 0.5,
        "group 1 now 6 lines down: {y} {top}"
    );
}

#[test]
fn groups_that_dont_cover_the_listing_draw_it_ungrouped() {
    let mut host = Host::new(100);
    host.coll.set_groups(groups_of(50, 10, "Group"));
    let rt = runtime(host);
    assert!(element(&rt, &header_key(0)).is_none());
    assert!(element(&rt, STUCK).is_none());
    assert!(a11y_node(&rt, "row-3").unwrap().contains(r#""position":4"#));
}

#[test]
fn a_drop_over_the_stuck_header_goes_to_the_folder_shown() {
    let mut host = grouped(100, 10);
    host.drops = true;
    let mut rt = runtime(host);
    let h = row_h(&rt);
    // Entry 0, a folder, on the top line, under the stuck header.
    scroll_to(&mut rt, h);
    let vp = rt.state.coll.viewport();
    rt.drag_files(
        vec!["/tmp/a.txt".into()],
        Some((vp.x + 60.0, vp.y + h / 2.0)),
    );
    rt.frame(1.0 / 60.0);
    rt.drag_files_to(Some((vp.x + 61.0, vp.y + h / 2.0)));
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.coll.drop_over(), Some(DropOver::Background));
    rt.drop_files(
        vec!["/tmp/a.txt".into()],
        Some((vp.x + 61.0, vp.y + h / 2.0)),
    );
    rt.frame(1.0 / 60.0);
    // Folder 10's row shows below: a drop there goes to it.
    let [x, y, w, rh] = element(&rt, "row-10").unwrap().visible;
    let at = (x + w / 2.0, y + rh / 2.0);
    rt.drag_files(vec!["/tmp/a.txt".into()], Some(at));
    rt.frame(1.0 / 60.0);
    rt.drag_files_to(Some((at.0 + 1.0, at.1)));
    rt.frame(1.0 / 60.0);
    rt.drop_files(vec!["/tmp/a.txt".into()], Some((at.0 + 1.0, at.1)));
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.dropped, [None, Some(id(10))]);
}

/// Records each press it hears in the host's order log, and takes it, changing nothing.
struct Hears(Rc<RefCell<Vec<String>>>);

impl PointerController for Hears {
    fn pointer(&mut self, input: &PointerInput, _cx: &mut Ctx<'_>) -> Flow {
        if input.phase == PointerPhase::Down {
            self.0.borrow_mut().push("controller".to_owned());
        }
        Flow::Handled
    }
}

#[test]
fn every_press_reaches_on_press_before_the_controllers() {
    let mut host = Host::new(5);
    host.parts = true;
    host.coll.controllers.pointer = Some(Box::new(Hears(host.order.clone())));
    let mut rt = runtime(host);
    // Ctrl on the empty space below the rows: nothing changes.
    let vp = rt.state.coll.viewport();
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods {
        ctrl: true,
        ..Mods::default()
    }));
    click_at(&mut rt, vp.x + 40.0, vp.y + vp.h - 10.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    let [x, y, w, h] = element(&rt, "row-2").unwrap().visible;
    rt.secondary_down(x + w / 2.0, y + h / 2.0);
    rt.secondary_up(x + w / 2.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    rt.button_down(x + w / 2.0, y + h / 2.0, rune::PointerButton::Middle);
    rt.button_up(x + w / 2.0, y + h / 2.0, rune::PointerButton::Middle);
    rt.frame(1.0 / 60.0);
    // A part above the viewport that takes its own presses isn't the collection's.
    click(&mut rt, "head-button");
    assert!(rt.state.coll.selection.is_empty());
    assert_eq!(
        *rt.state.order.borrow(),
        [
            "press Primary None",
            "controller",
            "press Secondary Some(2)",
            "controller",
            "press Middle Some(2)",
            "controller",
        ]
    );
}

// C1f review round 1.

/// B1: entering a folder with new groups, from another layout's position, reveals the cursor
/// among the new folder's header lines: clear of the stuck header.
#[test]
fn a_regrouped_enter_reveals_the_cursor_clear_of_the_stuck_header() {
    let position = {
        let mut rt = runtime(grouped(990, 10));
        rt.update(|s, _| {
            s.coll.selection.select_only(&s.src, 300);
        });
        rt.state.coll.position(&rt.state.src)
    };
    let mut rt = runtime(grouped(1_000, 10));
    settle(&mut rt);
    let h = row_h(&rt);
    scroll_to(&mut rt, 900.0 * h);
    rt.update(move |s, tx| {
        s.coll.set_layout_key(5);
        s.src = FastSource::new(990);
        s.version += 1;
        s.coll.set_groups(groups_of(990, 10, "New"));
        s.coll
            .enter(SCOPE, &s.src, Some(&position), Listing::Complete, tx);
    });
    settle(&mut rt);
    let vp = rt.state.coll.viewport();
    let shown = element(&rt, "row-300")
        .expect("the cursor's row is mounted")
        .visible;
    assert!(
        shown[1] >= vp.y + h - 0.5 && shown[3] > h - 0.5,
        "whole, below the stuck header: {shown:?}"
    );
}

/// B1: a restore that waits for a loading listing drawn ungrouped decides whether the cursor
/// shows by the complete listing's header lines, which push it below the view here.
#[test]
fn a_restore_completing_into_groups_places_the_cursor_among_their_headers() {
    let position = {
        let mut host = Host::new(30);
        host.coll.set_layout_key(7);
        let mut rt = runtime(host);
        rt.update(|s, _| {
            s.coll.selection.select_only(&s.src, 12);
        });
        rt.state.coll.position(&rt.state.src)
    };
    let mut rt = runtime(Host::new(0));
    rt.update(move |s, tx| {
        s.src = FastSource::new(13);
        s.coll
            .enter(SCOPE, &s.src, Some(&position), Listing::Loading, tx);
    });
    settle(&mut rt);
    assert!(rt.state.coll.restore_pending());
    // Ungrouped, entry 12 would sit on line 12, inside the 400 pt view; in groups of 2 it is
    // on line 19, below it.
    rt.update(|s, tx| {
        s.src = FastSource::new(30);
        s.coll.set_groups(groups_of(30, 2, "Pair"));
        s.coll
            .sync(SCOPE, &s.src, SyncRule::complete(CursorGone::Neighbor), tx);
    });
    settle(&mut rt);
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(12)));
    let vp = rt.state.coll.viewport();
    let shown = element(&rt, "row-12")
        .expect("the cursor's row is mounted")
        .visible;
    assert!(
        shown[3] > row_h(&rt) - 0.5 && shown[1] + shown[3] <= vp.y + vp.h + 0.5,
        "whole, in the view: {shown:?}"
    );
}

/// S1: a grid collapsed to no width and back at another width reflows, and reveals its cursor.
#[test]
fn a_grid_collapsed_and_back_at_another_width_reveals_its_cursor() {
    let mut host = Host::new(4_000);
    host.tile = Some(100.0);
    let mut rt = runtime(host);
    settle(&mut rt);
    for _ in 0..20 {
        rt.wheel(W / 4.0, H / 2.0, -200.0);
        rt.frame(1.0 / 60.0);
    }
    settle(&mut rt);
    let key = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.role == "gridcell" && e.visible[1] > 150.0 && e.visible[3] > 20.0)
        .and_then(|e| e.key)
        .unwrap();
    click_label(&mut rt, &key);
    rt.update(|s, _| s.width = 0.0);
    settle(&mut rt);
    rt.update(|s, _| s.width = 250.0);
    settle(&mut rt);
    let shown = element(&rt, &key)
        .expect("the cursor's cell is mounted")
        .visible;
    assert!(shown[3] > 20.0, "and shows whole: {shown:?}");
}

/// In the frame after a far jump into another group, before the build that relabels the stuck
/// header, it is hidden: a press on the top line drawn there is on that line's entry.
#[test]
fn a_press_while_the_stuck_header_hides_is_on_the_row_drawn_there() {
    let mut rt = runtime(grouped(1_000, 10));
    let h = row_h(&rt);
    // Entry 503 (line 554) at the top of the view, in group 50.
    rt.update(move |_, tx| tx.scroll_to_instant(keys::viewport_path(SCOPE), 0.0, 554.0 * h));
    rt.frame(1.0 / 60.0);
    assert!(stuck(&rt).1[3] < 0.5, "hidden until the relabelling build");
    let vp = rt.state.coll.viewport();
    click_at(&mut rt, vp.x + 40.0, vp.y + h / 2.0);
    assert_eq!(rt.state.coll.selection.cursor(), Some(id(503)));
}

// Keys from the host (C2's request E1).

/// Two collections keyed by their host under one focus stop that routes keys to the active one.
struct Strip {
    src: FastSource,
    colls: Vec<CollectionState>,
    look: Look,
    active: usize,
}

fn strip_pane(i: usize) -> Collection<Strip> {
    Collection::new(
        ["a", "b"][i],
        move |s: &Strip| (&s.src, &s.colls[i]),
        move |s: &mut Strip| (&s.src, &mut s.colls[i]),
        |s: &Strip| &s.look,
        |_: &Strip| 1,
    )
    .keys_by_host()
}

fn strip_view(s: &Strip, _: &mut Cx<Strip>) -> El {
    let mut strip = rune::row()
        .key("strip")
        .role("group")
        .focusable(true)
        .size(W, H)
        .on_key(|s: &mut Strip, e, tx| strip_pane(s.active).key(s, e, tx))
        .child(col().w(W / 2.0).h_full().child(strip_pane(0).build(s)))
        .child(col().w(W / 2.0).h_full().child(strip_pane(1).build(s)));
    if let Some(path) = strip_pane(s.active).cursor_path(s) {
        strip = strip.active_descendant(path);
    }
    strip
}

fn strip(n: usize) -> Runtime<Strip> {
    let mut colls = vec![CollectionState::new(), CollectionState::new()];
    for c in &mut colls {
        c.controllers.keyboard = Some(Box::new(Keys));
    }
    let state = Strip {
        src: FastSource::new(n),
        colls,
        look: Look::fallback(false),
        active: 1,
    };
    let mut rt = Runtime::new(state, strip_view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    for _ in 0..10 {
        rt.frame(1.0 / 60.0);
    }
    rt
}

/// E1: collections keyed by their host are no focus stops: a press in one focuses the host's
/// stop, which hands the key to the collection it picks; the stop names the cursor's row.
#[test]
fn a_collection_keyed_by_its_host_takes_keys_through_the_host() {
    let mut rt = strip(20);
    let lists: Vec<_> = rt
        .elements()
        .into_iter()
        .filter(|e| e.role == "listbox")
        .collect();
    assert_eq!(lists.len(), 2);
    assert!(
        lists.iter().all(|e| !e.focusable),
        "the collections aren't focus stops"
    );
    let [x, y, w, h] = element_in(&rt, "a", "row-3").unwrap().visible;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    let focused: Vec<_> = rt
        .elements()
        .into_iter()
        .filter(|e| e.focused)
        .filter_map(|e| e.key)
        .collect();
    assert_eq!(focused, ["strip"], "a press focuses the host's stop");
    assert_eq!(rt.state.colls[0].selection.cursor(), Some(id(3)));
    // The host routes End to b, not to a, which the press was in.
    rt.key(KeyEvent {
        key: Key::End,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.colls[1].selection.cursor(), Some(id(19)));
    assert_eq!(rt.state.colls[0].selection.cursor(), Some(id(3)));
    let strip = a11y_node(&rt, "strip").unwrap();
    assert!(strip.contains(r#""activeDescendant":"row-19""#), "{strip}");
    for scope in ["a", "b"] {
        let node = a11y_node(&rt, scope).unwrap();
        assert!(!node.contains("activeDescendant"), "{scope}: {node}");
    }
    // A key no controller takes isn't taken.
    let taken = rt.update_if(|s, tx| {
        strip_pane(1).key(
            s,
            &KeyEvent {
                key: Key::Home,
                mods: Mods::default(),
            },
            tx,
        )
    });
    assert!(!taken);
}

// The drag-over hook (C2's request E2).

/// Drags `at` a point on a folder row and then near the viewport's bottom edge, for 20 frames;
/// the scroll offset after.
fn drag_near_the_bottom(rt: &mut Runtime<Host>) -> f32 {
    let [x, y, w, h] = element(rt, "row-0").unwrap().visible;
    rt.drag_files(vec!["/tmp/a.txt".into()], Some((x + w / 2.0, y + h / 2.0)));
    rt.frame(1.0 / 60.0);
    for i in 0..20 {
        rt.drag_files_to(Some((x + w / 2.0, H - 4.0 - (i % 2) as f32)));
        rt.frame(1.0 / 60.0);
    }
    settle(rt);
    let offset = rt.state.coll.scroll_offset();
    rt.drag_files_leave();
    rt.frame(1.0 / 60.0);
    offset
}

/// E2: with `on_drag_over` the host hears an accepted drag over the rows and the background,
/// with its window point, and its leaving; the element's own edge scroll then stays still.
#[test]
fn the_drag_over_hook_hears_the_drag_and_replaces_the_edge_scroll() {
    let mut host = Host::new(2_000);
    host.drops = true;
    let mut rt = runtime(host);
    assert!(
        drag_near_the_bottom(&mut rt) > 0.0,
        "without the hook, the element's edge scroll moves"
    );

    let mut host = Host::new(2_000);
    host.drops = true;
    let log = Rc::new(RefCell::new(Vec::new()));
    host.over = Some(log.clone());
    let mut rt = runtime(host);
    let [x, y, w, h] = element(&rt, "row-0").unwrap().visible;
    assert_eq!(
        drag_near_the_bottom(&mut rt),
        0.0,
        "the hook scrolls instead"
    );
    let log = log.borrow();
    assert_eq!(
        log.first().copied().flatten(),
        Some(Point::new(x + w / 2.0, y + h / 2.0)),
        "the hover on the folder row says where"
    );
    assert!(
        log.iter()
            .flatten()
            .any(|p| p.y >= H - 5.0 && p.y <= H - 3.0),
        "moves over the background say where: {log:?}"
    );
    assert_eq!(log.last(), Some(&None), "leaving says so");
    assert_eq!(rt.state.coll.drop_over(), None);
}

// The held-back header press (C2's request E3).

/// E3: with `header_defers_press`, a click on the header's blank space leaves the selection
/// alone (the host still hears the press), and a drag from there bands as without it.
#[test]
fn a_held_back_header_press_keeps_the_selection_on_a_click_and_bands_on_a_drag() {
    let mut host = Host::new(20);
    host.parts = true;
    host.defer = true;
    host.coll.controllers.pointer =
        Some(Box::new(crate::collection::pointer::PointerIntent::new()));
    host.coll.controllers.marquee = Some(Box::new(crate::collection::marquee::Marquee::new()));
    let mut rt = runtime(host);
    click(&mut rt, "row-5");
    let [hx, hy, hw, hh] = element(&rt, "head").unwrap().visible;
    let x = hx + hw - 40.0;
    rt.state.order.borrow_mut().clear();
    // A click, and a press that moves less than the threshold.
    click_at(&mut rt, x, hy + hh / 2.0);
    rt.pointer_down(x, hy + hh / 2.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x + 2.0, hy + hh / 2.0 + 2.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x + 2.0, hy + hh / 2.0 + 2.0);
    rt.frame(1.0 / 60.0);
    {
        let s = &rt.state;
        assert_eq!(
            s.coll.selection.selected_ids(&s.src),
            [id(5)],
            "clicks keep it"
        );
        assert_eq!(s.order.borrow().len(), 2, "the host heard both presses");
    }
    // A drag from the same place bands from the viewport's top.
    let row_h = rt.state.look.row.list;
    rt.pointer_down(x, hy + hh / 2.0);
    rt.frame(1.0 / 60.0);
    for k in 1..=5 {
        rt.pointer_move(x - 5.0 * k as f32, hy + hh + row_h * 0.5 * k as f32);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(x - 25.0, hy + hh + row_h * 2.5);
    settle(&mut rt);
    let s = &rt.state;
    assert_eq!(s.coll.selection.selected_ids(&s.src), [id(0), id(1), id(2)]);
}

// A key during a band's edge scroll (C1d's open item S-B; C2's E6).

fn tick_poll(s: &mut Host, _: f64, tx: &mut rune::Tx) -> bool {
    let c = collection(s);
    c.tick(s, tx)
}

/// E6 (S-B): a key that moves the cursor while a band auto-scrolls at the viewport's edge keeps
/// its reveal: the band's glide stops, and the stop doesn't jump the view back to where it was.
#[test]
fn a_key_that_moves_the_cursor_during_a_bands_edge_scroll_keeps_its_reveal() {
    let mut host = Host::new(2_000);
    host.coll.controllers.pointer =
        Some(Box::new(crate::collection::pointer::PointerIntent::new()));
    host.coll.controllers.marquee = Some(Box::new(crate::collection::marquee::Marquee::new()));
    host.coll.controllers.keyboard = Some(Box::new(crate::collection::keyboard::Keyboard::new(
        crate::collection::keyboard::KeyHandle::default(),
    )));
    let mut rt = runtime(host);
    rt.poll = Some(tick_poll);
    let [_, y, _, h] = element(&rt, "row-3").unwrap().visible;
    // Blank space beside row 3: a drag from there bands.
    let x = W - 20.0;
    rt.pointer_down(x, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    for k in 1..=4 {
        rt.pointer_move(x, y + (H - 2.0 - y) * k as f32 / 4.0);
        rt.frame(1.0 / 60.0);
    }
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
    }
    let row_h = rt.state.look.row.list;
    assert!(
        rt.state.coll.scroll_offset() > 20.0 * row_h,
        "the band's edge scroll ran: {}",
        rt.state.coll.scroll_offset()
    );
    let cursor = rt.state.coll.selection.cursor_index(&rt.state.src).unwrap();
    rt.key(KeyEvent {
        key: Key::Down,
        mods: Mods::default(),
    });
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    let s = &rt.state;
    let moved = s.coll.selection.cursor_index(&s.src).unwrap();
    assert_eq!(moved, cursor + 1);
    let top = moved as f32 * row_h;
    let offset = s.coll.scroll_offset();
    assert!(
        top >= offset && top + row_h <= offset + H,
        "the cursor's row ({top}) shows in the view at {offset}"
    );
    rt.pointer_up(x, H - 2.0);
    rt.frame(1.0 / 60.0);
}

// A collection inside a sideways strip (C2's E7).

/// The collection 400 points into a 500-point sideways strip, so it starts clipped.
fn view_in_strip(s: &Host, _: &mut Cx<Host>) -> El {
    col().size(W, H).child(
        col().key("strip").w_full().h_full().scroll_x().child(
            rune::row()
                .h_full()
                .child(col().key("spacer").w(400.0).h_full())
                .child(col().w(300.0).h_full().child(collection(s).build(s))),
        ),
    )
}

/// Reveals entry 50 and reports where the strip's spacer and the row then stand.
fn reveal_in_strip(own: bool) -> (f32, bool) {
    let mut host = Host::new(200);
    host.coll.set_own_viewport_reveals(own);
    let mut rt = Runtime::new(host, view_in_strip, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    settle(&mut rt);
    assert_eq!(element(&rt, "spacer").expect("spacer").rect[0], 0.0);
    rt.update(|s, tx| s.coll.reveal(SCOPE, &s.src, 50, tx));
    rt.frame(1.0 / 60.0);
    let spacer = element(&rt, "spacer").expect("spacer").rect[0];
    let row = element(&rt, "row-50").is_some_and(|r| r.visible[3] >= r.rect[3] - 0.5);
    (spacer, row)
}

#[test]
fn own_viewport_reveals_leave_a_sideways_strip_around_the_collection_where_it_is() {
    let (spacer, shown) = reveal_in_strip(true);
    assert_eq!(spacer, 0.0, "the strip stays");
    assert!(shown, "the row shows in the listing, its full height");
    // Without them, Rune's reveal request also scrolls the strip, at once (Rune draft U-C2-4).
    let (spacer, shown) = reveal_in_strip(false);
    assert!(spacer < -100.0, "the strip jumped: {spacer}");
    assert!(shown);
}

// The rest hook (C22's folder peek).

/// Steps a still window only at the wakes it asks for (and for queued input), never a frame
/// between, for at most `limit` seconds or until `done`; returns the seconds stepped. A wait
/// that asked for no wake never ends here, as in a window at rest.
fn wake_until(rt: &mut Runtime<Host>, limit: f64, done: impl Fn(&Host) -> bool) -> f64 {
    let mut t = 0.0;
    for _ in 0..200 {
        if t > limit || done(&rt.state) {
            break;
        }
        if rt.has_pending_input() {
            rt.frame(1.0 / 60.0);
            t += 1.0 / 60.0;
            continue;
        }
        let now = rt.state.t;
        let Some(deadline) = rt.wake_deadline() else {
            break;
        };
        let dt = (deadline - now).max(0.001);
        if t + dt > limit {
            break;
        }
        rt.frame(dt);
        t += dt;
    }
    t
}

/// A runtime over `n` entries whose folder rows report resting, with its clock in `Host::t`.
fn rest_runtime(n: usize) -> Runtime<Host> {
    let mut host = Host::new(n);
    host.rest = true;
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    rt.poll = Some(|s: &mut Host, t, _| {
        s.t = t;
        false
    });
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn centre_of(rt: &Runtime<Host>, key: &str) -> (f32, f32) {
    let [x, y, w, h] = element(rt, key)
        .unwrap_or_else(|| panic!("no {key}"))
        .visible;
    (x + w / 2.0, y + h / 2.0)
}

/// Presses, a drag and a secondary press on rows, as a host hears them.
fn presses(rt: &mut Runtime<Host>) -> (Vec<EntryId>, Vec<String>) {
    settle(rt);
    click(rt, "row-10");
    click(rt, "row-12");
    let (x, y) = centre_of(rt, "row-5");
    rt.pointer_down(x, y);
    for i in 1..=10 {
        rt.pointer_move(x, y + 6.0 * i as f32);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(x, y + 60.0);
    rt.frame(1.0 / 60.0);
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(1.0 / 60.0);
    let s = &rt.state;
    (s.coll.selection.selected_ids(&s.src), s.intents.clone())
}

#[test]
fn rest_rows_give_their_presses_to_the_collection_as_other_rows_do() {
    let plain = presses(&mut runtime(Host::new(30)));
    let rest = presses(&mut rest_runtime(30));
    assert!(!plain.0.is_empty() && !plain.1.is_empty(), "{plain:?}");
    assert_eq!(rest, plain);
}

#[test]
fn a_folder_row_reports_resting_after_its_delay_and_leaving_after_its_own() {
    {
        let mut rt = rest_runtime(30);
        settle(&mut rt);
        let (x, y) = centre_of(&rt, "row-10");
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        let took = wake_until(&mut rt, 2.0, |s| !s.rests.is_empty());
        assert!((0.48..0.52).contains(&took), "reported after {took:.3} s");
        let r = &rt.state.rests[0];
        assert!(r.inside);
        assert_eq!(r.id, id(10));
        assert_eq!(&*r.collection, SCOPE);
        let drawn = element(&rt, "row-10").unwrap().rect;
        assert_eq!(
            [r.row.x, r.row.y, r.row.w, r.row.h],
            drawn,
            "the row's window rect"
        );
        assert_eq!(r.viewport, rt.state.coll.viewport());

        // Off it, onto a file row: the leave arrives 80 ms later.
        let (x, y) = centre_of(&rt, "row-11");
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        let took = wake_until(&mut rt, 2.0, |s| s.rests.len() > 1);
        assert!((0.06..0.1).contains(&took), "left after {took:.3} s");
        assert!(!rt.state.rests[1].inside);
        // File rows report nothing.
        wake_until(&mut rt, 1.0, |_| false);
        assert_eq!(rt.state.rests.len(), 2);
    }
}

#[test]
fn a_pass_shorter_than_the_delay_reports_nothing_and_leaves_no_wake() {
    let mut rt = rest_runtime(30);
    settle(&mut rt);
    let (x, y) = centre_of(&rt, "row-10");
    rt.pointer_move(x, y);
    rt.frame(0.2);
    let (x, y) = centre_of(&rt, "row-11");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |_| false);
    assert!(rt.state.rests.is_empty());
    assert_eq!(rt.wake_deadline(), None);
}

#[test]
fn after_keyboard_navigation_a_row_reports_resting_only_once_the_pointer_moves() {
    let mut rt = rest_runtime(30);
    settle(&mut rt);
    let (x, y) = centre_of(&rt, "row-10");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| s.coll.keyboard_moved());
    wake_until(&mut rt, 1.0, |_| false);
    assert!(rt.state.rests.is_empty(), "dropped: {:?}", rt.state.rests);
    // The pointer moves on to another folder: it reports again.
    let (x, y) = centre_of(&rt, "row-0");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |s| s.rests.iter().any(|r| r.inside));
    let inside: Vec<_> = rt
        .state
        .rests
        .iter()
        .filter(|r| r.inside)
        .map(|r| r.id)
        .collect();
    assert_eq!(inside, [id(0)]);
}

#[test]
fn a_rested_row_that_leaves_the_tree_reports_its_leave_at_once() {
    let mut rt = rest_runtime(30);
    settle(&mut rt);
    let (x, y) = centre_of(&rt, "row-10");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |s| !s.rests.is_empty());
    rt.update(|s, _| {
        s.src = FastSource::new(5);
        s.version += 1;
    });
    // The commit's frame prunes the row; Rune runs the leave it queued on the next one.
    rt.frame(1.0 / 60.0);
    rt.frame(1.0 / 60.0);
    let last = rt.state.rests.last().unwrap();
    assert_eq!((last.id, last.inside), (id(10), false));
    assert_eq!(last.row, Rect::default(), "the source no longer holds it");
}

#[test]
fn the_rest_part_mounts_in_the_row_that_asks_inert_and_hidden() {
    let mut rt = rest_runtime(30);
    settle(&mut rt);
    assert!(element(&rt, "anchor").is_none());
    rt.update(|s, _| s.anchor = Some(id(3)));
    rt.frame(1.0 / 60.0);
    let anchor = element(&rt, "anchor").expect("the part");
    assert!(anchor.path.ends_with("row-3/anchor"), "{}", anchor.path);
    assert!(anchor.inert);
    let row = element(&rt, "row-3").unwrap().rect;
    assert_eq!(anchor.rect[0], row[0] + 10.0, "pinned by its builder");
    let tree = rt.accessibility_tree();
    assert!(
        !format!("{tree:?}").contains("anchor"),
        "hidden from assistive technology"
    );
}

#[test]
fn a_row_a_drag_ends_on_reports_resting_only_once_the_pointer_moves() {
    let mut rt = rest_runtime(30);
    settle(&mut rt);
    let (x, y) = centre_of(&rt, "row-5");
    let (_, to) = centre_of(&rt, "row-10");
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    for i in 1..=10 {
        rt.pointer_move(x, y + (to - y) * i as f32 / 10.0);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(x, to);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |_| false);
    assert!(
        !rt.state.rests.iter().any(|r| r.inside),
        "dropped: {:?}",
        rt.state.rests
    );
    // The pointer moves on to another folder: it reports again.
    let (x, y) = centre_of(&rt, "row-0");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |s| s.rests.iter().any(|r| r.inside));
    let inside: Vec<_> = rt
        .state
        .rests
        .iter()
        .filter(|r| r.inside)
        .map(|r| r.id)
        .collect();
    assert_eq!(inside, [id(0)]);
}

#[test]
fn a_row_a_scroll_carries_under_a_parked_pointer_reports_nothing() {
    let mut rt = rest_runtime(300);
    settle(&mut rt);
    // Parked on file row 5; a scroll of five rows carries folder row 10 under it.
    let (x, y) = centre_of(&rt, "row-5");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    let h = element(&rt, "row-5").unwrap().rect[3];
    rt.update(|_, tx| tx.scroll_to_instant(viewport_path(SCOPE), 0.0, 5.0 * h));
    rt.frame(1.0 / 60.0);
    rt.frame(1.0 / 60.0);
    let under = element(&rt, "row-10").unwrap().visible;
    assert!(
        under[1] <= y && y < under[1] + under[3],
        "row 10 is under the pointer"
    );
    wake_until(&mut rt, 1.5, |_| false);
    assert!(
        !rt.state.rests.iter().any(|r| r.inside),
        "{:?}",
        rt.state.rests
    );
}

#[test]
fn a_held_collection_drops_the_rest_of_the_row_a_drag_from_elsewhere_ended_on() {
    let mut rt = rest_runtime(30);
    settle(&mut rt);
    // The pointer arrives on folder row 10 (a drag from elsewhere passing over), the host's
    // drag ends there and holds.
    let (x, y) = centre_of(&rt, "row-10");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| s.coll.hold_rest());
    wake_until(&mut rt, 1.0, |_| false);
    assert!(!rt.state.rests.iter().any(|r| r.inside), "dropped");
    // A move inside it is no arrival.
    rt.pointer_move(x + 5.0, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |_| false);
    assert!(!rt.state.rests.iter().any(|r| r.inside), "still dropped");
    // Leaving and coming back is.
    let (fx, fy) = centre_of(&rt, "row-11");
    rt.pointer_move(fx, fy);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 0.3, |_| false);
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |s| s.rests.iter().any(|r| r.inside));
    let inside: Vec<_> = rt
        .state
        .rests
        .iter()
        .filter(|r| r.inside)
        .map(|r| r.id)
        .collect();
    assert_eq!(inside, [id(10)]);
}

#[test]
fn the_rest_hook_hears_the_entries_the_viewport_shows() {
    let mut rt = rest_runtime(300);
    settle(&mut rt);
    let first = rt.state.views.last().expect("a first view").clone();
    assert_eq!(&*first.collection, SCOPE);
    assert_eq!(first.visible.first(), Some(&id(0)));
    let h = element(&rt, "row-5").unwrap().rect[3];
    rt.update(|_, tx| tx.scroll_to_instant(viewport_path(SCOPE), 0.0, 100.0 * h));
    rt.frame(1.0 / 60.0);
    rt.frame(1.0 / 60.0);
    let last = rt.state.views.last().unwrap();
    assert_eq!(last.visible.first(), Some(&id(100)));
    assert!(!last.visible.contains(&id(0)));
}

#[test]
fn back_on_the_same_row_after_leaving_the_collection_is_an_arrival() {
    let mut host = Host::new(30);
    host.rest = true;
    host.width = 300.0;
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    rt.poll = Some(|s: &mut Host, t, _| {
        s.t = t;
        false
    });
    rt.frame(0.0);
    settle(&mut rt);
    // A drag from elsewhere ended on folder row 10: held.
    let (x, y) = centre_of(&rt, "row-10");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| s.coll.hold_rest());
    wake_until(&mut rt, 1.0, |_| false);
    assert!(!rt.state.rests.iter().any(|r| r.inside), "held");
    // Out of the collection for longer than the leave delay, then straight back onto row 10.
    rt.pointer_move(W - 20.0, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 0.3, |_| false);
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |s| s.rests.iter().any(|r| r.inside));
    let inside: Vec<_> = rt
        .state
        .rests
        .iter()
        .filter(|r| r.inside)
        .map(|r| r.id)
        .collect();
    assert_eq!(inside, [id(10)], "an arrival");
}

/// X11 can send a leave and an enter back to back after a drop, with the pointer where it
/// was: a re-enter that soon is no arrival, so a held row stays held. (Out of the window for
/// longer, the same return is one.)
#[test]
fn a_held_row_stays_held_when_the_pointer_leaves_and_reenters_at_once() {
    let mut rt = rest_runtime(30);
    settle(&mut rt);
    let (x, y) = centre_of(&rt, "row-10");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| s.coll.hold_rest());
    rt.pointer_leave();
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |_| false);
    assert!(
        !rt.state.rests.iter().any(|r| r.inside),
        "{:?}",
        rt.state.rests
    );
    // Nor is a move inside the row after it.
    rt.pointer_move(x + 5.0, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |_| false);
    assert!(
        !rt.state.rests.iter().any(|r| r.inside),
        "{:?}",
        rt.state.rests
    );
}

/// Out of the window for longer than a back-to-back leave and enter, the pointer coming back
/// to the very point it left is an arrival.
#[test]
fn back_at_the_same_point_after_leaving_the_window_is_an_arrival() {
    let mut rt = rest_runtime(30);
    settle(&mut rt);
    let (x, y) = centre_of(&rt, "row-10");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| s.coll.hold_rest());
    rt.pointer_leave();
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 0.3, |_| false);
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |s| s.rests.iter().any(|r| r.inside));
    let inside: Vec<_> = rt
        .state
        .rests
        .iter()
        .filter(|r| r.inside)
        .map(|r| r.id)
        .collect();
    assert_eq!(inside, [id(10)], "an arrival");
}

/// A runtime whose folder rows report resting and take drops.
fn rest_drop_runtime() -> Runtime<Host> {
    let mut rt = rest_runtime(30);
    rt.update(|s, _| s.drops = true);
    settle(&mut rt);
    rt
}

/// A file drag (a system drag: the collection sees no pointer moves) over `key` and dropped
/// there, then held by the host, as X2's adapter does when the drag ends.
fn drop_on_and_hold(rt: &mut Runtime<Host>, key: &str) -> (f32, f32) {
    let at = centre_of(rt, key);
    rt.drag_files(vec!["/tmp/a.txt".into()], Some(at));
    rt.frame(1.0 / 60.0);
    rt.drop_files(vec!["/tmp/a.txt".into()], Some(at));
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| s.coll.hold_rest());
    at
}

fn rested(rt: &Runtime<Host>) -> Vec<EntryId> {
    rt.state
        .rests
        .iter()
        .filter(|r| r.inside)
        .map(|r| r.id)
        .collect()
}

/// A system drag moves the pointer onto the row it drops on without the collection seeing a
/// move: the pointer's first move there afterwards (X11's crossing after the ungrab) is no
/// arrival, so the folder dropped on never peeks.
#[test]
fn the_first_move_after_a_system_drop_on_a_row_is_no_arrival() {
    let mut rt = rest_drop_runtime();
    let (x, y) = drop_on_and_hold(&mut rt, "row-10");
    assert_eq!(rt.state.dropped, [Some(id(10))]);
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |_| false);
    assert_eq!(rested(&rt), []);
}

/// Last seen on another row, the pointer leaves and enters again at once on the row a system
/// drag dropped on (X11's ungrab crossing, handled after the host held): no arrival.
#[test]
fn a_crossing_onto_the_row_a_system_drag_dropped_on_is_no_arrival() {
    let mut rt = rest_drop_runtime();
    let (x, y) = centre_of(&rt, "row-5");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    let (x, y) = drop_on_and_hold(&mut rt, "row-10");
    rt.pointer_leave();
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |_| false);
    assert_eq!(rested(&rt), []);
}

/// A window blur during a system drag forgets the row the pointer was on: the drop on that
/// row records it again, so a small move after the drag is no arrival either.
#[test]
fn a_blur_during_a_system_drag_doesnt_make_the_row_dropped_on_an_arrival() {
    let mut rt = rest_drop_runtime();
    let (x, y) = centre_of(&rt, "row-10");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.window_focus(false);
    rt.frame(1.0 / 60.0);
    drop_on_and_hold(&mut rt, "row-10");
    rt.window_focus(true);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x + 3.0, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |_| false);
    assert_eq!(rested(&rt), []);
    // Moving on to another folder still reports it.
    let (x, y) = centre_of(&rt, "row-0");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |s| s.rests.iter().any(|r| r.inside));
    assert_eq!(rested(&rt), [id(0)]);
}

/// A drop a folder row refuses lands on it all the same: no arrival either.
#[test]
fn the_first_move_after_a_refused_system_drop_on_a_row_is_no_arrival() {
    let mut rt = rest_drop_runtime();
    rt.update(|s, _| s.refuse = true);
    settle(&mut rt);
    let (x, y) = drop_on_and_hold(&mut rt, "row-10");
    assert_eq!(rt.state.dropped, []);
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    wake_until(&mut rt, 1.0, |_| false);
    assert_eq!(rested(&rt), []);
}
