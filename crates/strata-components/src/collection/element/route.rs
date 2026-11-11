//! Input routing: Rune's events on the collection, turned into controller inputs, and the
//! controllers' changes and effects applied.
//!
//! Every pointer handler sits on the collection's container, not on rows: a row's handlers
//! would make it a hit target per row, and Rune reports a `.virtualize` stack's `on_drag` y
//! rebased to the mounted rows (Rune draft U-C1e-1). Content coordinates come from the window
//! point, the viewport's window origin (recorded from `on_layout`) and the scroll offset
//! (followed by `on_scroll_offset`), as `contract::geometry` defines them.
//!
//! An input that changes nothing the view reads commits no view: a pointer move with no
//! selection change, no effect and no [`Ctx::repaint`] says `Tx::live_only`, as do the
//! moves and the release of a gesture the collection didn't take.

use std::rc::Rc;

use rune::{Drag, KeyEvent, KeyPlatform, Mods, Phase, PointerButton, PointerEvent, Tx};

use super::lines::{GroupedRows, LineMap};
use super::seam::{
    Ctx, Effect, Flow, Hit, KeyInput, PointerInput, PointerPhase, Region, default_pointer,
};
use super::{Config, keys};
use crate::collection::selection::Changed;
use crate::contract::{CollectionGeometry, EntrySource, Point, Rect};

/// Where an input came from, for what the element does after it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    /// A pointer input; for a move, its content point, which a marquee begun by it hears.
    Pointer(Option<Point>),
    Key,
    Tick,
}

/// How the element lays out `s`'s entries now: the state's count a line, and the host's groups
/// when they cover the source.
pub(super) fn line_map<S: 'static>(cfg: &Config<S>, s: &S) -> LineMap {
    let (src, st) = (cfg.get)(s);
    LineMap::new(st.per_line.get(), Some(&st.groups), src.len())
}

/// Where `s`'s entries sit, mounted or not: the grouped rows, or the lines' own geometry.
pub(super) fn geometry<S: 'static>(cfg: &Config<S>, s: &S) -> Box<dyn CollectionGeometry> {
    let row_height = cfg.row_height_of(s);
    let (src, st) = (cfg.get)(s);
    match line_map(cfg, s) {
        LineMap::Grouped(groups) => Box::new(
            GroupedRows::new(groups, row_height, st.viewport.w).viewport_height(st.viewport.h),
        ),
        LineMap::Packed(_) => {
            cfg.lines
                .geometry(src.len(), row_height, st.viewport.w, st.viewport.h)
        }
    }
}

/// Whether content point `p` is under the stuck group header, which covers the top of the
/// viewport: a press there is on no entry, as on a header line.
///
/// Only while the stuck header shows: in the frame between a scroll into another group and the
/// build that relabels it, it is hidden, and the top line is the one drawn there.
fn in_stuck_header<S: 'static>(cfg: &Config<S>, s: &S, p: Point) -> bool {
    let st = (cfg.get)(s).1;
    let offset = st.scroll.get();
    let h = cfg.row_height_of(s);
    let map = line_map(cfg, s);
    let shown = st.stuck_shown.get();
    if shown.is_none() || map.top_group(offset, h) != shown {
        return false;
    }
    let extent = map.stuck_extent(offset, h);
    p.y >= offset && p.y < offset + extent
}

/// Whether the window point of a drag is under the stuck group header.
pub(super) fn under_stuck_header<S: 'static>(cfg: &Config<S>, s: &S, e: PointerEvent) -> bool {
    let p = (cfg.get)(s)
        .1
        .content_point(Point::new(e.window_x, e.window_y));
    in_stuck_header(cfg, s, p)
}

/// Runs `f` with the controllers and a [`Ctx`], then applies what it changed and asked for.
/// Returns the controller's flow, and whether anything the view reads changed.
fn dispatch<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    tx: &mut Tx,
    from: Source,
    f: impl FnOnce(&mut super::Controllers, &mut Ctx<'_>) -> Flow,
) -> (Flow, bool) {
    let now = tx.clock();
    let animations = (cfg.look)(s).animations;
    let geometry = geometry(cfg, s);
    let (src, st) = (cfg.get_mut)(s);
    let visible = st.visible_content();
    let origin = Point::new(st.viewport.x, st.viewport.y);
    let before = st.selection.cursor();
    let super::CollectionState {
        selection,
        controllers,
        ..
    } = &mut *st;
    let mut cx = Ctx::new(src, &*geometry, visible, now, selection)
        .read_only_mode(cfg.read_only)
        .with_animations(animations)
        .window_origin(origin)
        .with_settings(cfg.settings.clone());
    let flow = f(controllers, &mut cx);
    let (changed, effects, repaint) = cx.finish();
    let moved = before != st.selection.cursor();
    // Only what the view reads needs a commit. Scrolls, reveals and wake-ups go through the
    // transaction, which Rune applies whether or not the view runs (a poll's too, even when it
    // returns false: `runtime/frame.rs`, the poll's `merge`). Keyboard mode counts only when
    // the effects leave it other than it was.
    let mode_was = (st.keyboard, st.hover_live);
    let mode = effects.iter().fold(mode_was, |mode, e| match e {
        Effect::KeyboardMode(on) => (*on, !*on),
        _ => mode,
    });
    let shows = changed.any()
        || repaint
        || mode != mode_was
        || effects
            .iter()
            .any(|e| matches!(e, Effect::BeginMarquee { .. } | Effect::Intent(_)));
    after(cfg, s, tx, from, changed, moved, effects);
    (flow, shows)
}

/// What the element does once a controller handled an input.
fn after<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    tx: &mut Tx,
    from: Source,
    changed: Changed,
    moved: bool,
    effects: Vec<Effect>,
) {
    // A key that moves the cursor takes the view from a band's edge scroll, whose next tick
    // would otherwise glide on, or stop dead where it was, over the key's reveal (C1d's S-B).
    // A glide in flight would keep its speed through a reveal request, and an instant stop
    // would land after it (Rune applies offsets after reveals), so the reveal is one instant
    // scroll then.
    let instant = from == Source::Key && moved && {
        let st = (cfg.get_mut)(s).1;
        st.controllers
            .marquee
            .as_mut()
            .is_some_and(|m| m.yield_scroll())
    };
    let reveal = |s: &mut S, index: usize, tx: &mut Tx| {
        let (src, st) = (cfg.get_mut)(s);
        if instant {
            st.reveal_instant(&cfg.key, src, index, tx);
        } else {
            st.reveal(&cfg.key, src, index, tx);
        }
    };
    let mut intents = Vec::new();
    let mut revealed = false;
    let mut mode_set = false;
    for effect in effects {
        match effect {
            Effect::Reveal(index) => {
                reveal(s, index, tx);
                revealed = true;
            }
            Effect::BeginMarquee { at, mods } => {
                // A band begun by a move also takes that move, so it doesn't lag one behind.
                let then = match from {
                    Source::Pointer(moved_to) => moved_to,
                    _ => None,
                };
                begin_marquee(cfg, s, tx, at, mods, then);
            }
            Effect::ScrollTo { y, glide } => {
                let key = keys::viewport_path(&cfg.key);
                if glide {
                    tx.scroll_to(key, 0.0, y);
                } else {
                    tx.scroll_to_instant(key, 0.0, y);
                }
            }
            Effect::Glide { y, spring } => {
                tx.scroll_to_with(keys::viewport_path(&cfg.key), 0.0, y, spring);
            }
            Effect::KeyboardMode(on) => {
                let st = (cfg.get_mut)(s).1;
                st.keyboard = on;
                st.hover_live = !on;
                mode_set = true;
            }
            Effect::WakeAfter(d) => tx.wake_after(d),
            Effect::Intent(intent) => intents.push(intent),
        }
    }
    // A key that moves the cursor turns keyboard mode on, unless the controller said
    // otherwise; a key that changes nothing leaves it as it was.
    if from == Source::Key && moved && !mode_set {
        let st = (cfg.get_mut)(s).1;
        st.keyboard = true;
        st.hover_live = false;
    }
    // A key or a tick that moves the cursor keeps it in view; a pointer moves it only to what
    // it shows.
    if moved && !matches!(from, Source::Pointer(_)) && !revealed {
        let index = {
            let (src, st) = (cfg.get)(s);
            st.selection.cursor_index(src)
        };
        if let Some(index) = index {
            reveal(s, index, tx);
        }
    }
    // The status line reads a band's count once its gesture ends, not at every step.
    {
        let (src, st) = (cfg.get_mut)(s);
        let banding = st.controllers.marquee.as_ref().is_some_and(|m| m.active());
        if !banding {
            st.note_selection(src);
        }
    }
    if changed.cursor
        && let Some(f) = &cfg.on_cursor
    {
        let cursor = (cfg.get)(s).1.selection.cursor();
        f(s, cursor, tx);
    }
    if changed.selection
        && let Some(f) = &cfg.on_selection
    {
        f(s, tx);
    }
    if let Some(f) = &cfg.on_intent {
        for intent in intents {
            f(s, intent, tx);
        }
    }
}

fn begin_marquee<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    tx: &mut Tx,
    at: Point,
    mods: Mods,
    then: Option<Point>,
) {
    let has = (cfg.get)(s).1.controllers.marquee.is_some();
    if !has {
        return;
    }
    dispatch(cfg, s, tx, Source::Pointer(None), |c, cx| {
        if let Some(m) = c.marquee.as_mut() {
            m.begin(at, mods, cx);
            if let Some(p) = then {
                m.moved(p, cx);
            }
        }
        Flow::Handled
    });
}

/// The entry under content point `p`, and the region it's in when `regions` (presses and
/// releases); a move reuses the press's region over the pressed entry.
fn hit<S: 'static>(cfg: &Rc<Config<S>>, s: &S, p: Point, press: Option<Hit>) -> Hit {
    let src = (cfg.get)(s).0;
    let geometry = geometry(cfg, s);
    // A vanished row (`RowMarks::vanished`) takes no input: a press there is on empty space.
    let vanished = |i: usize| {
        cfg.marks
            .as_ref()
            .is_some_and(|f| f(s, src.id_at(i)).vanished)
    };
    let index = geometry
        .index_at(p)
        .filter(|&i| !in_stuck_header(cfg, s, p) && !vanished(i));
    let Some(index) = index else {
        return Hit {
            index: None,
            region: Region::Gutter,
        };
    };
    let region = match press {
        Some(press) if press.index == Some(index) => press.region,
        Some(_) => Region::Gutter,
        None => {
            let rect = geometry.rect_of(index);
            let local = Point::new(p.x - rect.x, p.y - rect.y);
            match &cfg.regions {
                Some(f) => f(s, index, local),
                None => default_region(cfg, s, src, index, local),
            }
        }
    };
    Hit {
        index: Some(index),
        region,
    }
}

/// The default row's regions: its icon (and the gap after it) is content, the drawn width of
/// its name is the name, as Strata counts a label's rendered text (not the space it fills) as
/// the entry; the rest is gutter.
fn default_region<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &S,
    src: &dyn EntrySource,
    index: usize,
    local: Point,
) -> Region {
    let look = (cfg.look)(s);
    let entry = src.entry(index);
    let (label_w, _) = rune::text::measure(entry.name, &[], look.kit.ty.ui_compact, None);
    let left = strata_kit::slots::collection::row_inset(look) + look.space.xs;
    let name = left + look.icon_size.md + look.space.sm;
    if local.x >= name && local.x < name + label_w {
        Region::Name
    } else if local.x >= left && local.x < name {
        Region::Content
    } else {
        Region::Gutter
    }
}

/// `on_drag` on the container.
pub(super) fn on_drag<S: 'static>(cfg: &Rc<Config<S>>, s: &mut S, d: &mut Drag, tx: &mut Tx) {
    // A host's system drag owns the phases after it starts (`Collection::on_drag_phase`).
    let taken = cfg
        .on_drag_phase
        .as_ref()
        .is_some_and(|f| f(s, d, &cfg.key, tx));
    if taken && d.phase != Phase::Up {
        tx.live_only();
        return;
    }
    let [wx, wy] = d.window_point();
    let window = Point::new(wx, wy);
    let phase = match d.phase {
        Phase::Down => PointerPhase::Down,
        Phase::Move => PointerPhase::Move,
        Phase::Up => PointerPhase::Up,
    };
    if phase == PointerPhase::Up && cfg.rest.is_some() {
        (cfg.get_mut)(s).1.hold_rest();
    }
    let st = (cfg.get)(s).1;
    let press = st.gesture;
    // A primary press on the header part's blank space (where no part of it takes presses
    // itself) is on empty space at the viewport's top edge, as Strata starts a band from a List
    // heading strip (C4). Only a primary one: the rule is for a band, and a middle press there
    // (an autoscroll, a new tab) isn't on the collection.
    let on_header = phase == PointerPhase::Down
        && tx.button() == PointerButton::Primary
        && !st.viewport.contains(window)
        && st.header.get().contains(window);
    match phase {
        // Any other press outside the viewport isn't the collection's, and neither are the
        // moves and the release that follow it.
        PointerPhase::Down if !st.viewport.contains(window) && !on_header => {
            (cfg.get_mut)(s).1.gesture = None;
            tx.live_only();
            return;
        }
        PointerPhase::Move | PointerPhase::Up if press.is_none() => {
            tx.live_only();
            return;
        }
        _ => {}
    }
    let point = if on_header {
        Point::new(window.x - st.viewport.x, st.scroll.get())
    } else {
        st.content_point(window)
    };
    let hover_was_live = st.hover_live;
    let hit = if on_header {
        Hit {
            index: None,
            region: Region::Gutter,
        }
    } else {
        hit(
            cfg,
            s,
            point,
            (phase == PointerPhase::Move).then_some(press).flatten(),
        )
    };
    {
        let st = (cfg.get_mut)(s).1;
        match phase {
            PointerPhase::Down => {
                st.pending = None;
                st.keyboard = false;
                st.gesture = Some(hit);
            }
            PointerPhase::Up => st.gesture = None,
            PointerPhase::Move => {}
        }
        st.hover_live = true;
    }
    let input = PointerInput {
        phase,
        button: tx.button(),
        point,
        window,
        travel: Point::new(d.dx, d.dy),
        hit,
        mods: tx.mods(),
        clicks: d.clicks,
        // A release the system drag took ends the press as a cancelled one.
        cancelled: d.cancelled() || taken,
        now: tx.clock(),
    };
    if phase == PointerPhase::Down
        && let Some(f) = &cfg.on_press
    {
        f(s, &input, tx);
    }
    // A press on the header held back (`Collection::header_defers_press`): it reaches the
    // controllers once it travels past the threshold, with the move that took it there; a
    // release before then is a click on the header, which the collection doesn't take.
    let mut released = false;
    if let Some(threshold) = cfg.header_defer {
        let st = (cfg.get_mut)(s).1;
        match phase {
            PointerPhase::Down if on_header => {
                st.deferred = Some(input);
                tx.live_only();
                return;
            }
            PointerPhase::Down => st.deferred = None,
            PointerPhase::Move if st.deferred.is_some() => {
                let past = input.travel.x.abs() > threshold || input.travel.y.abs() > threshold;
                if !past || input.cancelled {
                    tx.live_only();
                    return;
                }
                let down = st.deferred.take().expect("checked");
                released = route_pointer(cfg, s, tx, &down);
            }
            PointerPhase::Up if st.deferred.take().is_some() => {
                tx.live_only();
                return;
            }
            _ => {}
        }
    }
    let shows = route_pointer(cfg, s, tx, &input) || released;
    // A move that changed nothing the view reads commits nothing, an open band's included: a
    // marquee that redraws says so with `Ctx::repaint`.
    if phase == PointerPhase::Move && !shows && hover_was_live {
        tx.live_only();
    }
}

/// `on_secondary_press_local` on the container.
pub(super) fn on_secondary<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    e: PointerEvent,
    tx: &mut Tx,
) {
    let window = Point::new(e.window_x, e.window_y);
    let st = (cfg.get)(s).1;
    if !st.viewport.contains(window) {
        tx.live_only();
        return;
    }
    let point = st.content_point(window);
    (cfg.get_mut)(s).1.pending = None;
    let input = PointerInput {
        phase: PointerPhase::Down,
        button: PointerButton::Secondary,
        point,
        window,
        travel: Point::default(),
        hit: hit(cfg, s, point, None),
        mods: tx.mods(),
        clicks: 1,
        cancelled: false,
        now: tx.clock(),
    };
    if let Some(f) = &cfg.on_press {
        f(s, &input, tx);
    }
    route_pointer(cfg, s, tx, &input);
}

/// Routes a pointer input: the keyboard controller sees it, then an open marquee takes the
/// gesture's moves and release, else the pointer controller (or the default) has it. True when
/// anything the view reads changed.
fn route_pointer<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    tx: &mut Tx,
    input: &PointerInput,
) -> bool {
    let input = *input;
    let moved_to = (input.phase == PointerPhase::Move).then_some(input.point);
    let (_, shows) = dispatch(cfg, s, tx, Source::Pointer(moved_to), move |c, cx| {
        if let Some(k) = c.keyboard.as_mut() {
            k.pointer_seen(&input);
        }
        if let Some(m) = c.marquee.as_mut()
            && m.active()
        {
            match input.phase {
                PointerPhase::Move => m.moved(input.point, cx),
                PointerPhase::Up if input.cancelled => {
                    m.cancel(cx);
                }
                PointerPhase::Up => m.end(cx),
                PointerPhase::Down => {}
            }
            if input.phase != PointerPhase::Down {
                return Flow::Handled;
            }
        }
        match c.pointer.as_mut() {
            Some(p) => p.pointer(&input, cx),
            None => default_pointer(&input, cx),
        }
    });
    shows
}

/// `on_key` on the container. True when a controller took the key.
pub(super) fn on_key<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    event: &KeyEvent,
    tx: &mut Tx,
) -> bool {
    // A copy of the key's transaction, so controllers can match layout-aware chords
    // (`KeyInput::matches`) while the element keeps the real one for its effects.
    let keys = tx.clone();
    let input = KeyInput {
        event,
        physical: tx.physical_key(),
        base: tx.key_base(),
        repeat: tx.key_repeat(),
        platform: KeyPlatform::current(),
        now: tx.clock(),
        tx: Some(&keys),
    };
    let (flow, _) = dispatch(cfg, s, tx, Source::Key, |c, cx| {
        if let Some(p) = c.pointer.as_mut() {
            p.key_seen(&input);
        }
        if event.key == rune::Key::Escape
            && let Some(m) = c.marquee.as_mut()
            && m.cancel(cx)
        {
            return Flow::Handled;
        }
        match c.keyboard.as_mut() {
            Some(k) => k.key(&input, cx),
            None => Flow::Pass,
        }
    });
    if flow == Flow::Handled {
        (cfg.get_mut)(s).1.pending = None;
        true
    } else {
        false
    }
}

/// A rest row's `on_hover_change` (after its delays): the hook hears it, with the row's and the
/// viewport's window rects. A rest counts only on the row the pointer last arrived on by
/// moving (not one carried under it by a scroll, nor the row a press or a held drag ended on),
/// and not while row hover is off after keyboard navigation.
pub(super) fn on_rest<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    id: crate::contract::EntryId,
    inside: bool,
    tx: &mut Tx,
) {
    let Some(rest) = &cfg.rest else {
        tx.live_only();
        return;
    };
    let (src, st) = (cfg.get)(s);
    if inside && (!st.hover_live || st.rest_entered != Some(id)) {
        tx.live_only();
        return;
    }
    let index = src.index_of(id);
    if inside && index.is_none() {
        tx.live_only();
        return;
    }
    let row = index.map_or_else(Rect::default, |i| {
        st.content_to_window(geometry(cfg, s).rect_of(i))
    });
    let report = super::RestReport {
        collection: cfg.key.clone(),
        id,
        inside,
        row,
        viewport: st.viewport(),
    };
    (rest.report)(s, report, tx);
}

/// A drop on folder row `id`, taken or refused: with the rest hook, the pointer is on that row
/// and its rest is held. A system drag carries the pointer there with no move the collection
/// sees, so without this the pointer's next move there (X11's crossing after the drop, or the
/// first after a blur during the drag) would count as an arrival and peek the folder.
pub(super) fn dropped_on_row<S: 'static>(cfg: &Config<S>, s: &mut S, id: crate::contract::EntryId) {
    if cfg.rest.is_some() {
        let st = (cfg.get_mut)(s).1;
        st.rest_under = Some(id);
        st.rest_entered = None;
    }
}

/// `on_hover_change` on the container of a collection with the rest hook: leaving it forgets
/// the row the pointer was on.
pub(super) fn on_hover<S: 'static>(cfg: &Rc<Config<S>>, s: &mut S, inside: bool, tx: &mut Tx) {
    if !inside {
        (cfg.get_mut)(s).1.rest_under = None;
    }
    tx.live_only();
}

/// `on_pointer_move_local` on the container: hover comes back once the pointer moves, and with
/// the rest hook, the row the pointer arrives on is the one whose rest counts.
pub(super) fn on_pointer_move<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    p: rune::PointerEvent,
    tx: &mut Tx,
) {
    if cfg.rest.is_some() {
        let window = Point::new(p.window_x, p.window_y);
        let under = {
            let (src, st) = (cfg.get)(s);
            if st.viewport.contains(window) {
                geometry(cfg, s)
                    .index_at(st.content_point(window))
                    .filter(|&i| i < src.len())
                    .map(|i| src.id_at(i))
            } else {
                None
            }
        };
        let st = (cfg.get_mut)(s).1;
        if under != st.rest_under {
            st.rest_under = under;
            st.rest_entered = under;
        }
    }
    let st = (cfg.get_mut)(s).1;
    if st.hover_live {
        tx.live_only();
    } else {
        st.hover_live = true;
    }
}

/// `on_layout_tx` on the viewport: where it shows, as bookkeeping, except that a width that
/// changes how many entries a line holds (a grid's reflow) builds the view again.
pub(super) fn on_layout<S: 'static>(cfg: &Rc<Config<S>>, s: &mut S, r: [f32; 4], tx: &mut Tx) {
    let per = cfg.lines.per_line(r[2]).max(1);
    let row_height = cfg.row_height_of(s);
    let (src, st) = (cfg.get_mut)(s);
    st.viewport = Rect::from(r);
    st.row_height = row_height;
    // A collapsed viewport (a Columns column at no width) keeps its count: the width it comes
    // back at decides it, as a reflow. Reveals asked for meanwhile wait for it.
    if r[2] <= 0.0 {
        tx.live_only();
        return;
    }
    let changed = if st.laid_out {
        st.reflow_to(&cfg.key, src, per, tx)
    } else {
        // The very first width isn't a reflow: nothing was laid out at the count the state
        // started with, so a restore made before it (`enter`) stands.
        st.laid_out = true;
        let changed = per != st.per_line.get();
        st.per_line.set(per);
        changed
    };
    // A reveal asked for while the viewport had no width, keyed by the lines as they are now.
    if let Some(id) = st.reveal_after_layout.take()
        && let Some(index) = src.index_of(id)
    {
        st.reveal(&cfg.key, src, index, tx);
    }
    if !changed {
        tx.live_only();
    }
}

/// `on_visible_rows` on the rows stack, with lines turned into entries by `map`, the map of
/// the build that laid them out. A scroll away from where a waiting restore put the view is
/// the person's, and cancels it. A scroll into a group other than the one whose header the
/// build stuck at the top (`shown`) builds the view again, to stick that group's.
pub(super) fn on_visible<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    lines: std::ops::Range<usize>,
    map: &LineMap,
    shown: Option<usize>,
) -> bool {
    let restick = map.groups().is_some() && map.group_at_line(lines.start) != shown;
    (cfg.get)(s).1.top_line.set(Some(lines.start));
    let (src, st) = (cfg.get_mut)(s);
    let range = map.entries_in(lines, src.len());
    st.visible = range.clone();
    if st.pending.is_some() && (st.scroll.get() - st.pending_offset).abs() > 0.5 {
        st.pending = None;
    }
    let rest = match cfg.rest.as_ref().and_then(|r| r.view.as_ref()) {
        Some(f) => {
            let visible = range.clone().map(|i| src.id_at(i)).collect();
            let view = super::RestView {
                collection: cfg.key.clone(),
                visible,
            };
            f(s, view)
        }
        None => false,
    };
    let host = match &cfg.on_visible {
        Some(f) => f(s, range),
        None => false,
    };
    host || restick || rest
}

/// The controllers' timeouts and queued commands, from the host's poll.
pub(super) fn tick<S: 'static>(cfg: &Rc<Config<S>>, s: &mut S, tx: &mut Tx) -> bool {
    let (_, shows) = dispatch(cfg, s, tx, Source::Tick, |c, cx| {
        for p in c.pointer.iter_mut() {
            p.tick(cx);
        }
        for k in c.keyboard.iter_mut() {
            k.tick(cx);
        }
        for m in c.marquee.iter_mut() {
            m.tick(cx);
        }
        Flow::Pass
    });
    // An auto-scroll tick that only glides and asks for the next frame changes nothing the
    // view reads: the poll returns false, and Rune still applies its scroll and wake-up.
    shows
}

/// A drag over the collection near the viewport's top or bottom edge glides it that way, a
/// row's height a move, so a drop target out of view can be reached. `changed`: the move
/// changed what the view draws (the drop target), so it commits either way.
pub(super) fn drop_edge_scroll<S: 'static>(
    cfg: &Rc<Config<S>>,
    s: &mut S,
    point: PointerEvent,
    changed: bool,
    tx: &mut Tx,
) {
    let row_height = cfg.row_height_of(s);
    let st = (cfg.get)(s).1;
    let y = point.window_y - st.viewport.y;
    let offset = st.scroll_offset();
    let to = if y < row_height {
        (offset - row_height).max(0.0)
    } else if y > st.viewport.h - row_height {
        offset + row_height
    } else {
        if !changed {
            tx.live_only();
        }
        return;
    };
    tx.scroll_to(keys::viewport_path(&cfg.key), 0.0, to);
}
