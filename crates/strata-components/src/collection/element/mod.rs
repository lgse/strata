// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod a11y;
mod keys;
mod lines;
mod matches;
mod origin;
mod route;
mod row;
/// Virtualized collection seam types and helpers.
pub mod seam;
mod state;

#[cfg(test)]
mod tests;

use std::any::Any;
use std::borrow::Cow;
use std::ops::Range;
use std::rc::Rc;

use rune::{
    El, GrowthSpec, KeyEvent, Live, Mods, PointerButton, ScrollPos, ScrollSpec, Tx, WheelEvent,
    col, text,
};
use strata_kit::look::Look;

pub use a11y::{Announcement, Roles, default_announcement};
pub use keys::{
    ICON, LABEL, ROWS, STATUS, STUCK, VIEWPORT, entry_path, header_key, part_path, row_key,
    row_path, viewport_path,
};
use lines::LineMap;
pub use lines::{Group, GroupStyle, GroupedRows, Groups, Lines, OnePerLine};
pub use matches::NameMatches;
pub(crate) use route::KeyTaken;
pub use row::{ContentSlot, Describe, Rest, RestReport, RestView, RowCx, RowMarks, RowState, Slot};
pub use seam::{
    Activation, Controllers, Ctx, Effect, Flow, Hit, Intent, KeyController, KeyInput,
    MarqueeController, PointerController, PointerInput, PointerPhase, Region, Settings,
    default_pointer,
};
pub use state::{CollectionState, DropOver};

#[cfg(doc)]
use crate::collection::selection::Selection;
use crate::contract::{
    CollectionGeometry, Dnd, DropAction, DropHover, DropOffer, DropZone, Dropped, EntryId,
    EntrySource, Point, Rect,
};

const REENTER: std::time::Duration = std::time::Duration::from_millis(100);

type GetFn<S> = Rc<dyn Fn(&S) -> (&dyn EntrySource, &CollectionState)>;
type GetMutFn<S> = Rc<dyn Fn(&mut S) -> (&dyn EntrySource, &mut CollectionState)>;
type LookFn<S> = Rc<dyn Fn(&S) -> &Look>;
type ReadFn<S, T> = Rc<dyn Fn(&S) -> T>;
type RowHeightFn<S> = Rc<dyn Fn(&S, &Look) -> f32>;
type RegionFn<S> = Rc<dyn Fn(&S, usize, Point) -> Region>;
type PartFn<S> = Rc<dyn Fn(&S) -> Option<El>>;
type RenameFn<S> = (ReadFn<S, Option<EntryId>>, Slot<S>);
type MarksFn<S> = Rc<dyn Fn(&S, EntryId) -> RowMarks>;
type IntentFn<S> = Rc<dyn Fn(&mut S, Intent, &mut Tx)>;
type CursorFn<S> = Rc<dyn Fn(&mut S, Option<EntryId>, &mut Tx)>;
type SelectionFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
type VisibleFn<S> = Rc<dyn Fn(&mut S, Range<usize>) -> bool>;
type AnnounceFn<S> = Rc<dyn Fn(&S, &Announcement) -> String>;
type GroupStyleFn = Rc<dyn Fn(&Look) -> GroupStyle>;
type PressFn<S> = Rc<dyn Fn(&mut S, &PointerInput, &mut Tx)>;
type ScrollFn = Rc<dyn Fn(ScrollPos)>;
pub(crate) type DragOverFn<S> = Rc<dyn Fn(&mut S, Option<Point>, &mut Tx) -> bool>;
pub(crate) type DragPhaseFn<S> = Rc<dyn Fn(&mut S, &rune::Drag, &str, &mut Tx) -> bool>;
pub(crate) type DropActionFn<S> =
    Rc<dyn Fn(&S, Option<EntryId>, &DropOffer, Mods) -> Option<DropAction>>;
type WheelFn<S> = Rc<dyn Fn(&mut S, WheelEvent, &mut Tx) -> bool>;
type BeforeKeysFn<S> = Rc<dyn Fn(&mut S, &KeyEvent, &mut Tx) -> bool>;

/// What [`Collection`] was built with, shared by its rows and handlers.
pub(crate) struct Config<S> {
    key: Rc<str>,
    get: GetFn<S>,
    get_mut: GetMutFn<S>,
    look: LookFn<S>,
    version: ReadFn<S, u64>,
    settings: Rc<Settings>,
    name: Option<String>,
    description: Option<String>,
    roles: Roles,
    row_height: RowHeightFn<S>,
    row_gap_y: f32,
    lines: Rc<dyn Lines>,
    group_style: GroupStyleFn,
    dnd: Dnd<S>,
    /// What the folder rows and the background take from other apps besides files.
    drop_content: &'static [&'static str],
    read_only: bool,
    bare_items: bool,
    on_drop: Option<row::DropFn<S>>,
    icon: Option<Slot<S>>,
    label: Option<Slot<S>>,
    content: Option<ContentSlot<S>>,
    describe: Option<Describe<S>>,
    rename: Option<RenameFn<S>>,
    marks: Option<MarksFn<S>>,
    regions: Option<RegionFn<S>>,
    header: Option<PartFn<S>>,
    viewport_fill: Option<rune::Color>,
    viewport_bottom_pad: f32,
    overlay: Option<PartFn<S>>,
    empty: Option<PartFn<S>>,
    /// Every part [`Collection::edge`] added, in order.
    edge: Vec<PartFn<S>>,
    on_intent: Option<IntentFn<S>>,
    on_press: Option<PressFn<S>>,
    on_header_click: Option<PressFn<S>>,
    on_cursor: Option<CursorFn<S>>,
    on_selection: Option<SelectionFn<S>>,
    on_visible: Option<VisibleFn<S>>,
    announce: Option<AnnounceFn<S>>,
    /// Every handler [`Collection::on_scroll`] added, in order.
    on_scroll: Vec<ScrollFn>,
    keys_by_host: bool,
    before_keys: Option<BeforeKeysFn<S>>,
    on_drag_over: Option<DragOverFn<S>>,
    on_drag_phase: Option<DragPhaseFn<S>>,
    drop_action: Option<DropActionFn<S>>,
    header_defer: Option<f32>,
    on_wheel: Option<WheelFn<S>>,
    rest: Option<Rest<S>>,
}

impl<S> Config<S> {
    /// The row height for `s`.
    fn row_height_of(&self, s: &S) -> f32 {
        (self.row_height)(s, (self.look)(s))
    }
}

/// The collection element's builder.
pub struct Collection<S> {
    cfg: Config<S>,
}

impl<S: 'static> Collection<S> {
    /// A collection keyed `key` (its rows' keys and scroll requests start there).
    pub fn new(
        key: impl Into<Rc<str>>,
        get: impl Fn(&S) -> (&dyn EntrySource, &CollectionState) + 'static,
        get_mut: impl Fn(&mut S) -> (&dyn EntrySource, &mut CollectionState) + 'static,
        look: impl Fn(&S) -> &Look + 'static,
        listing_version: impl Fn(&S) -> u64 + 'static,
    ) -> Self {
        Self {
            cfg: Config {
                key: key.into(),
                get: Rc::new(get),
                get_mut: Rc::new(get_mut),
                look: Rc::new(look),
                version: Rc::new(listing_version),
                settings: Rc::new(Settings::default()),
                name: None,
                description: None,
                roles: Roles::LISTBOX,
                row_height: Rc::new(|_, l| l.row.list),
                row_gap_y: 0.0,
                lines: Rc::new(OnePerLine),
                group_style: Rc::new(GroupStyle::from_look),
                dnd: Dnd::rune(),
                drop_content: &[],
                read_only: false,
                bare_items: false,
                on_drop: None,
                icon: None,
                label: None,
                content: None,
                describe: None,
                rename: None,
                marks: None,
                regions: None,
                header: None,
                viewport_fill: None,
                viewport_bottom_pad: 0.0,
                overlay: None,
                empty: None,
                edge: Vec::new(),
                on_intent: None,
                on_press: None,
                on_header_click: None,
                on_cursor: None,
                on_selection: None,
                on_visible: None,
                announce: None,
                on_scroll: Vec::new(),
                keys_by_host: false,
                before_keys: None,
                on_drag_over: None,
                on_drag_phase: None,
                drop_action: None,
                header_defer: None,
                on_wheel: None,
                rest: None,
            },
        }
    }

    /// The name assistive technology reads for the collection ("Documents").
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.cfg.name = Some(name.into());
        self
    }

    /// What assistive technology reads after the collection's name: what it holds ("Files"), in the host's words.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.cfg.description = Some(description.into());
        self
    }

    /// The roles: `Roles::LISTBOX` (the default), `Roles::LIST`, `Roles::GRID_ROWS` or `Roles::GRID_CELLS`.
    pub fn roles(mut self, roles: Roles) -> Self {
        self.cfg.roles = roles;
        self
    }

    /// The row height, read from the look: `row.list` by default, `row.columns` for a Columns column.
    pub fn row_height(mut self, row_height: impl Fn(&S, &Look) -> f32 + 'static) -> Self {
        self.cfg.row_height = Rc::new(row_height);
        self
    }

    /// How entries pack into lines; `OnePerLine` by default.
    pub fn lines(mut self, lines: impl Lines + 'static) -> Self {
        self.cfg.lines = Rc::new(lines);
        self
    }

    /// How header lines and the stuck header draw: [`GroupStyle::from_look`] by default.
    pub fn group_style(mut self, style: impl Fn(&Look) -> GroupStyle + 'static) -> Self {
        self.cfg.group_style = Rc::new(style);
        self
    }

    /// Adds a settings value for the controllers (the double-click interval, the click mode, the auto-scroll speed).
    pub fn settings<T: Any>(mut self, value: T) -> Self {
        Rc::make_mut(&mut self.cfg.settings).insert(value);
        self
    }

    /// The settings value of type `T` added so far.
    pub fn setting<T: Any>(&self) -> Option<&T> {
        self.cfg.settings.get::<T>()
    }

    /// The drag-and-drop binder: Rune's own by default.
    pub fn dnd(mut self, dnd: Dnd<S>) -> Self {
        self.cfg.dnd = dnd;
        self
    }

    /// The content types the folder rows and the background take from other apps besides files.
    pub fn content_types(mut self, mimes: &'static [&'static str]) -> Self {
        self.cfg.drop_content = mimes;
        self
    }

    /// Read-only: no drops and no rename, as the archive preview needs.
    pub fn read_only(mut self) -> Self {
        self.cfg.read_only = true;
        self
    }

    /// Items draw none of the row's own chrome: no selection fill, hover, outline, corner radius or side inset.
    pub fn bare_items(mut self) -> Self {
        self.cfg.bare_items = true;
        self
    }

    /// Blank margin at the top and bottom of each virtual row slot (`0.0` by default).
    pub fn row_gap_y(mut self, gap: f32) -> Self {
        self.cfg.row_gap_y = gap.max(0.0);
        self
    }

    /// Takes drops: on a folder-like row (`Some(id)`) or the background (`None`, the folder shown).
    pub fn on_drop(
        mut self,
        f: impl Fn(&mut S, Option<EntryId>, Dropped, &mut Tx) + 'static,
    ) -> Self {
        self.cfg.on_drop = Some(Rc::new(f));
        self
    }

    /// The icon or thumbnail part.
    pub fn icon(mut self, slot: impl Into<Slot<S>>) -> Self {
        self.cfg.icon = Some(slot.into());
        self
    }

    /// The label part (matched text, say). By default, the name in `ty.ui_compact`.
    pub fn label(mut self, slot: Slot<S>) -> Self {
        self.cfg.label = Some(slot);
        self
    }

    /// The row's content, from its icon and label.
    pub fn content(mut self, slot: ContentSlot<S>) -> Self {
        self.cfg.content = Some(slot);
        self
    }

    /// Each row's accessible description, read after its name. By default, none.
    pub fn describe(mut self, describe: Describe<S>) -> Self {
        self.cfg.describe = Some(describe);
        self
    }

    /// The rename editor: drawn in the label's place on the row `renaming` names.
    pub fn rename(
        mut self,
        renaming: impl Fn(&S) -> Option<EntryId> + 'static,
        editor: Slot<S>,
    ) -> Self {
        self.cfg.rename = Some((Rc::new(renaming), editor));
        self
    }

    /// The host's marks on each row: cut, dragging, the Enter target, the open path.
    pub fn marks(mut self, marks: impl Fn(&S, EntryId) -> RowMarks + 'static) -> Self {
        self.cfg.marks = Some(Rc::new(marks));
        self
    }

    /// Which part of entry `index`'s row a point (in the row's own points) is on.
    pub fn regions(mut self, f: impl Fn(&S, usize, Point) -> Region + 'static) -> Self {
        self.cfg.regions = Some(Rc::new(f));
        self
    }

    /// Fills the viewport area below `Collection::header` with `color` (so a translucent header above the viewport never double-composites over a background fill).
    pub fn viewport_fill(mut self, color: rune::Color) -> Self {
        self.cfg.viewport_fill = Some(color);
        self
    }

    /// Reserves `pad` points below the vertical viewport inside the filled area below `Collection::header` (e.g.
    pub fn viewport_bottom_pad(mut self, pad: f32) -> Self {
        self.cfg.viewport_bottom_pad = pad.max(0.0);
        self
    }

    /// A part above the viewport.
    pub fn header(mut self, f: impl Fn(&S) -> Option<El> + 'static) -> Self {
        self.cfg.header = Some(Rc::new(f));
        self
    }

    /// A primary press on the header part's blank space reaches the controllers only once it travels more than `threshold` points (along either axis; pass the pointer controller's drag threshold): a drag from there then bands as `Collection::header` says, and a click there changes nothing in the collection.
    pub fn header_defers_press(mut self, threshold: f32) -> Self {
        self.cfg.header_defer = Some(threshold);
        self
    }

    /// Hears a primary click on the header's blank space that released before travelling past `Collection::header_defers_press`, with the `Down` input of that press.
    pub fn on_header_click(mut self, f: impl Fn(&mut S, &PointerInput, &mut Tx) + 'static) -> Self {
        self.cfg.on_header_click = Some(Rc::new(f));
        self
    }

    /// A part drawn over the viewport, pinned to it below any header, in its coordinates.
    pub fn overlay(mut self, f: impl Fn(&S) -> Option<El> + 'static) -> Self {
        self.cfg.overlay = Some(Rc::new(f));
        self
    }

    /// What an empty collection shows, pinned over the viewport below any header.
    pub fn empty(mut self, f: impl Fn(&S) -> Option<El> + 'static) -> Self {
        self.cfg.empty = Some(Rc::new(f));
        self
    }

    /// A part over the viewport, below any header, in its coordinates, that takes input: scroll bar along its edge.
    pub fn edge(mut self, f: impl Fn(&S) -> Option<El> + 'static) -> Self {
        self.cfg.edge.push(Rc::new(f));
        self
    }

    /// Hears the viewport's scroll position on every frame it changes (Rune's `El::on_scroll_offset`), with no view commit.
    pub fn on_scroll(mut self, f: impl Fn(ScrollPos) + 'static) -> Self {
        self.cfg.on_scroll.push(Rc::new(f));
        self
    }

    /// The collection's key, the scope of its key paths (`viewport_path` of it is its viewport's).
    pub fn scope(&self) -> &str {
        &self.cfg.key
    }

    /// Keys reach this collection from the host, not from Rune's focus.
    pub fn keys_by_host(mut self) -> Self {
        self.cfg.keys_by_host = true;
        self
    }

    /// Hears each key the collection is given before its controllers do, with the key's own transaction.
    pub fn before_keys(mut self, f: impl Fn(&mut S, &KeyEvent, &mut Tx) -> bool + 'static) -> Self {
        self.cfg.before_keys = Some(Rc::new(f));
        self
    }

    /// Hears where a drag the collection's drop zones accept is, in window points, while it is over them (`Some`.
    pub fn on_drag_over(
        mut self,
        f: impl Fn(&mut S, Option<Point>, &mut Tx) -> bool + 'static,
    ) -> Self {
        self.cfg.on_drag_over = Some(Rc::new(f));
        self
    }

    /// Hears each phase of the container's drag handler before the controllers do, with the collection's key.
    pub fn on_drag_phase(
        mut self,
        f: impl Fn(&mut S, &rune::Drag, &str, &mut Tx) -> bool + 'static,
    ) -> Self {
        self.cfg.on_drag_phase = Some(Rc::new(f));
        self
    }

    /// Says which action a drag over a folder row (`Some(id)`) or the background (`None`) would take.
    pub fn drop_action(
        mut self,
        f: impl Fn(&S, Option<EntryId>, &DropOffer, Mods) -> Option<DropAction> + 'static,
    ) -> Self {
        self.cfg.drop_action = Some(Rc::new(f));
        self
    }

    /// Hears wheel and trackpad deltas over the viewport before it scrolls (Rune's `El::on_wheel` on the viewport itself).
    pub fn on_wheel(mut self, f: impl Fn(&mut S, WheelEvent, &mut Tx) -> bool + 'static) -> Self {
        self.cfg.on_wheel = Some(Rc::new(f));
        self
    }

    /// Rows that report the pointer resting on them, with Rune's hover delays, and a part pinned in them: folder peek.
    pub fn rest(mut self, rest: Rest<S>) -> Self {
        self.cfg.rest = Some(rest);
        self
    }

    /// Hears the controllers' intents (open, rename, drag, context menu).
    pub fn on_intent(mut self, f: impl Fn(&mut S, Intent, &mut Tx) + 'static) -> Self {
        self.cfg.on_intent = Some(Rc::new(f));
        self
    }

    /// Hears every press the collection takes, before its controllers, whether or not anything changes.
    pub fn on_press(mut self, f: impl Fn(&mut S, &PointerInput, &mut Tx) + 'static) -> Self {
        self.cfg.on_press = Some(Rc::new(f));
        self
    }

    /// Hears the cursor move, after an input moved it: the preview follows it.
    pub fn on_cursor(mut self, f: impl Fn(&mut S, Option<EntryId>, &mut Tx) + 'static) -> Self {
        self.cfg.on_cursor = Some(Rc::new(f));
        self
    }

    /// Hears the selection change, after an input changed it.
    pub fn on_selection(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.cfg.on_selection = Some(Rc::new(f));
        self
    }

    /// Hears which entries the viewport shows, when that changes.
    pub fn on_visible(mut self, f: impl Fn(&mut S, Range<usize>) -> bool + 'static) -> Self {
        self.cfg.on_visible = Some(Rc::new(f));
        self
    }

    /// The status line's wording for a selection change: `default_announcement` by default.
    pub fn announce(mut self, f: impl Fn(&S, &Announcement) -> String + 'static) -> Self {
        self.cfg.announce = Some(Rc::new(f));
        self
    }

    /// Runs the controllers' timeouts and queued commands.
    pub fn tick(self, s: &mut S, tx: &mut Tx) -> bool {
        let cfg = Rc::new(self.cfg);
        route::tick(&cfg, s, tx)
    }

    /// Routes a primary drag starting on an external marquee origin surface (such as the preview drawer's blank space).
    pub fn origin_drag(
        self,
        s: &mut S,
        (d, clears_on_press): (&mut rune::Drag, bool),
        tx: &mut Tx,
    ) {
        let cfg = Rc::new(self.cfg);
        origin::origin_drag(&cfg, s, (d, clears_on_press), tx);
    }

    /// Handles a key as the collection's own key handler would.
    pub fn key(self, s: &mut S, event: &KeyEvent, tx: &mut Tx) -> bool {
        self.key_taken(s, event, tx) != KeyTaken::No
    }

    pub(crate) fn key_taken(self, s: &mut S, event: &KeyEvent, tx: &mut Tx) -> KeyTaken {
        let cfg = Rc::new(self.cfg);
        route::on_key(&cfg, s, event, tx)
    }

    /// The key path of the cursor's row (or cell), under this collection's key.
    pub fn cursor_path(&self, s: &S) -> Option<String> {
        let (src, st) = (self.cfg.get)(s);
        let index = st.selection.cursor_index(src)?;
        Some(keys::entry_path(
            &self.cfg.key,
            src,
            index,
            st.per_line.get().max(1),
        ))
    }

    /// Works the entries per line out again from the lines' rule as it stands now.
    pub fn reflow(self, s: &mut S, tx: &mut Tx) -> bool {
        let (src, st) = (self.cfg.get_mut)(s);
        if st.viewport.w <= 0.0 {
            return false;
        }
        let per = self.cfg.lines.per_line(st.viewport.w);
        st.reflow_to(&self.cfg.key, src, per, tx)
    }

    /// The geometry of `s`'s entries as the element lays them out now, in content coordinates.
    pub fn geometry(&self, s: &S) -> Box<dyn CollectionGeometry> {
        route::geometry(&self.cfg, s)
    }

    /// Entry `index`'s rect in window points, as the element lays it out now.
    pub fn window_rect_of(&self, s: &S, index: usize) -> Rect {
        let rect = self.geometry(s).rect_of(index);
        (self.cfg.get)(s).1.content_to_window(rect)
    }

    /// The element, filling the height its parent gives it.
    pub fn build(self, s: &S) -> El {
        let cfg = Rc::new(self.cfg);
        let (src, st) = (cfg.get)(s);
        let look = (cfg.look)(s);
        let len = src.len();
        let per = if st.viewport.w > 0.0 {
            cfg.lines.per_line(st.viewport.w).max(1)
        } else {
            st.per_line.get().max(1)
        };
        st.per_line.set(per);
        let map = LineMap::new(per, Some(&st.groups), len);
        let lines = map.count(len);
        let row_height = cfg.row_height_of(s);
        let shown = st
            .top_line
            .get()
            .and_then(|line| map.group_at_line(line))
            .or_else(|| map.top_group(st.scroll.get(), row_height))
            .filter(|_| len > 0);
        st.stuck_shown.set(shown);
        let frame = row::Frame {
            look_key: row::look_key(look),
            config_key: row::config_key(&cfg),
            row_height,
            row_gap_y: cfg.row_gap_y,
            per_line: per,
            inset: if per > 1 { cfg.lines.inset() } else { 0.0 },
            len,
            grouped: map.groups().is_some(),
            hover_live: st.hover_live,
            cursor_shown: st.keyboard && st.active,
            drop: st.drop,
            renaming: cfg
                .rename
                .as_ref()
                .filter(|_| !cfg.read_only)
                .and_then(|(f, _)| f(s)),
        };

        let key_cfg = cfg.clone();
        let build_cfg = cfg.clone();
        let visible_cfg = cfg.clone();
        let (key_map, build_map, visible_map) = (map.clone(), map.clone(), map.clone());
        let rows = col()
            .key(ROWS)
            .w_full()
            .growth(GrowthSpec::default())
            .instant_changes()
            .virtual_rows(
                lines,
                row_height,
                move |s: &S, line| {
                    let src = (key_cfg.get)(s).0;
                    Cow::Owned(key_map.line_key(src, line))
                },
                move |s: &S, line| row::build_line(&build_cfg, &frame, &build_map, s, line),
            )
            .virtual_rows_version(((cfg.version)(s), per, len, map.fingerprint()))
            .on_visible_rows(move |s: &mut S, r| {
                route::on_visible(&visible_cfg, s, r, &visible_map, shown)
            });

        let live = st.scroll.clone();
        let layer = st.scroll_layer.clone();
        let on_scroll = cfg.on_scroll.clone();
        let stuck = shown.map(|shown| Stuck {
            map: map.clone(),
            h: row_height,
            push: st.stuck_offset.clone(),
            shown,
        });
        // Up to date for this build (a regroup moves no scroll): the scroll keeps it so after.
        if let Some(stuck) = &stuck {
            stuck.follow(st.scroll.get());
        }
        let stuck_part = stuck.as_ref().and_then(|stuck| {
            let group = map.groups()?.groups().get(stuck.shown)?;
            let style = (cfg.group_style)(look);
            let label = text(group.label.to_string()).truncate();
            Some(
                row::header_body(&style, row_height, label)
                    .key(STUCK)
                    .fill(style.fill)
                    .live_offset(&st.stuck_offset),
            )
        });
        let layout_cfg = cfg.clone();
        let viewport = col()
            .key(VIEWPORT)
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .on_scroll_offset(move |p: ScrollPos| {
                live.set(p.offset[1]);
                layer.set([0.0, -p.offset[1]]);
                if let Some(stuck) = &stuck {
                    stuck.follow(p.offset[1]);
                }
                for f in on_scroll.iter() {
                    f(p);
                }
            })
            .on_layout_tx(move |s: &mut S, r, tx| route::on_layout(&layout_cfg, s, r, tx));
        let viewport = match cfg.on_wheel.clone() {
            Some(f) => viewport.on_wheel(move |s: &mut S, e, tx| f(s, e, tx)),
            None => viewport,
        }
        .child(rows);

        let drag_cfg = cfg.clone();
        let secondary_cfg = cfg.clone();
        let key_cfg = cfg.clone();
        let move_cfg = cfg.clone();
        let a11y_cfg = cfg.clone();
        let mut root = col()
            .key(cfg.key.to_string())
            .role(cfg.roles.container)
            .w_full()
            .h_full()
            .min_h(0.0)
            .clip_soft(0.0)
            .pointer_buttons([PointerButton::Middle])
            .on_drag(move |s: &mut S, d, tx| route::on_drag(&drag_cfg, s, d, tx))
            .on_secondary_press_local(move |s: &mut S, e, tx| {
                route::on_secondary(&secondary_cfg, s, e, tx)
            })
            .on_pointer_move_local(move |s: &mut S, p, tx| {
                route::on_pointer_move(&move_cfg, s, p, tx)
            })
            .on_a11y_action(move |s: &mut S, key, action, tx| {
                route::on_a11y_action(&a11y_cfg, s, key, action, tx)
            });
        if cfg.rest.is_some() {
            let leave_cfg = cfg.clone();
            root = root
                .hover_delays(std::time::Duration::ZERO, REENTER)
                .on_hover_change(move |s: &mut S, inside, tx| {
                    route::on_hover(&leave_cfg, s, inside, tx)
                });
        }
        if !cfg.keys_by_host {
            root = root
                .focusable(true)
                .on_key(move |s: &mut S, e, tx| route::on_key(&key_cfg, s, e, tx) != KeyTaken::No);
        }
        if let Some(name) = &cfg.name {
            root = root.label(name.clone());
        }
        if let Some(description) = &cfg.description {
            root = root.description(description.clone());
        }
        let vanished = |i: usize| {
            cfg.marks
                .as_ref()
                .is_some_and(|f| f(s, src.id_at(i)).vanished)
        };
        if !cfg.keys_by_host
            && let Some(index) = st.selection.cursor_index(src)
            && !vanished(index)
        {
            root = root.active_descendant(keys::entry_path(&cfg.key, src, index, per));
        }
        if let Some(header) = cfg.header.as_ref().and_then(|f| f(s)) {
            let header_cfg = cfg.clone();
            root = root.child(
                col()
                    .w_full()
                    .on_layout_tx(move |s: &mut S, r, tx| {
                        (header_cfg.get_mut)(s).1.header.set(Rect::from(r));
                        tx.live_only();
                    })
                    .child(header),
            );
        } else {
            // No header now: a press where one was is outside the collection again.
            st.header.set(Rect::default());
        }
        let mut over = col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .clip_soft(0.0)
            .child(viewport);
        if let Some(fill) = cfg.viewport_fill {
            over = over.fill(fill);
        }
        if cfg.viewport_bottom_pad > 0.0 {
            over = over.pb(cfg.viewport_bottom_pad);
        }
        if len == 0
            && let Some(empty) = cfg.empty.as_ref().and_then(|f| f(s))
        {
            over = over.child(pinned(empty));
        }
        if let Some(part) = stuck_part {
            let part = col()
                .pin(Some(0.0), Some(0.0), Some(0.0), None)
                .h(row_height)
                .clip_soft(0.0)
                .a11y_hidden()
                .inert()
                .child(part);
            over = over.child(part);
        }
        if let Some(band) = st
            .controllers
            .marquee
            .as_ref()
            .and_then(|m| m.overlay(look))
        {
            let content = col()
                .pin(Some(0.0), Some(0.0), Some(0.0), None)
                .live_offset(&st.scroll_layer)
                .child(band);
            over = over.child(pinned(content).inert());
        }
        if let Some(overlay) = cfg.overlay.as_ref().and_then(|f| f(s)) {
            over = over.child(pinned(overlay).inert());
        }
        for edge in cfg.edge.iter().filter_map(|f| f(s)) {
            over = over.child(crate::scroll::anchor(0.0, 0.0, edge));
        }
        root = root.child(over);
        let announcement = st.announcement();
        let line = match &cfg.announce {
            Some(f) => f(s, &announcement),
            None => default_announcement(&announcement),
        };
        root = root.child(a11y::status(&line));
        if let Some(on_drop) = &cfg.on_drop
            && !cfg.read_only
        {
            root = cfg.dnd.drop_zone_keyed(
                root,
                cfg.key.to_string(),
                background_zone(&cfg, on_drop.clone()),
            );
        }
        root
    }
}

struct Stuck {
    map: LineMap,
    h: f32,
    push: Live<[f32; 2]>,
    shown: usize,
}

impl Stuck {
    fn follow(&self, offset: f32) {
        let dy = if self.map.top_group(offset, self.h) == Some(self.shown) {
            self.map.stuck_extent(offset, self.h) - self.h
        } else {
            -self.h
        };
        self.push.set([0.0, dy]);
    }
}

/// `part` over the whole of its parent.
fn pinned(part: El) -> El {
    col()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(part)
}

/// The background's drop zone: drops go to the folder shown.
fn background_zone<S: 'static>(cfg: &Rc<Config<S>>, on_drop: row::DropFn<S>) -> DropZone<S> {
    let get_mut = cfg.get_mut.clone();
    let hover_cfg = cfg.clone();
    let edge = cfg.clone();
    let zone = DropZone::new(move |s: &mut S, dropped, tx| {
        (get_mut)(s).1.drop = None;
        on_drop(s, None, dropped, tx);
    })
    .takes_content(cfg.drop_content);
    let zone = match cfg.drop_action.clone() {
        Some(f) => {
            let reject_cfg = cfg.clone();
            zone.action(move |s: &S, offer, mods| f(s, None, offer, mods))
                .on_reject(move |s: &mut S, _, tx| {
                    note_drop_hover(&reject_cfg, s, DropOver::Background, DropHover::Left, None);
                    drag_over_hover(&reject_cfg, s, DropHover::Left, tx);
                })
        }
        None => zone,
    };
    zone.on_hover(move |s: &mut S, hover, offer, tx| {
        note_drop_hover(&hover_cfg, s, DropOver::Background, hover, Some(offer));
        drag_over_hover(&hover_cfg, s, hover, tx);
    })
    .on_move(move |s: &mut S, point, tx| {
        let st = (edge.get_mut)(s).1;
        if st.drop_refused == Some(DropOver::Background) && !st.drop_refused_files {
            tx.live_only();
            return;
        }
        match &edge.on_drag_over {
            Some(f) => {
                if !f(s, Some(Point::new(point.window_x, point.window_y)), tx) {
                    tx.live_only();
                }
            }
            None => route::drop_edge_scroll(&edge, s, point, false, tx),
        }
    })
}

pub(super) fn note_drop_hover<S: 'static>(
    cfg: &Config<S>,
    s: &mut S,
    over: DropOver,
    hover: DropHover,
    offer: Option<&crate::contract::DropOffer>,
) {
    let st = (cfg.get_mut)(s).1;
    let mine = |d: Option<DropOver>| d == Some(over);
    match hover {
        DropHover::Accepting => {
            if mine(st.drop_refused) {
                st.drop_refused = None;
            }
            if over != DropOver::Background || st.drop.is_none() {
                st.drop = Some(over);
            }
        }
        // No highlight on a zone that refuses the drag: the platform shows "can't drop".
        DropHover::Rejecting | DropHover::Left => {
            if mine(st.drop) {
                st.drop = None;
            }
            st.drop_refused = match hover {
                DropHover::Rejecting => {
                    st.drop_refused_files = offer.is_some_and(|o| {
                        o.uris.is_some()
                            || o.mime_types.iter().any(|m| m == crate::contract::URI_LIST)
                    });
                    Some(over)
                }
                _ if mine(st.drop_refused) => None,
                _ => st.drop_refused,
            };
        }
    }
}

pub(super) fn drag_over_hover<S: 'static>(
    cfg: &Config<S>,
    s: &mut S,
    hover: DropHover,
    tx: &mut Tx,
) {
    let Some(f) = &cfg.on_drag_over else {
        return;
    };
    let at = match hover {
        DropHover::Left => None,
        DropHover::Accepting | DropHover::Rejecting => {
            let at = tx.drop_point().map(|p| Point::new(p.window_x, p.window_y));
            let refused = hover == DropHover::Rejecting;
            if refused && (at.is_none() || !(cfg.get)(s).1.drop_refused_files) {
                return;
            }
            at
        }
    };
    f(s, at, tx);
}
