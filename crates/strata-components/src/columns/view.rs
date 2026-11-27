// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Drag, El, Key, KeyEvent, Mods, Phase, PointerButton, Tx, col, row};
use strata_kit::look::Look;

use super::column;
use super::drag_scroll::{ColumnGeometry, Scroll, StripGeometry};
use super::fit;
use super::intent::ColumnsIntent;
use super::keys;
use super::pointer::{BackgroundClick, PeekClick, PeekGate};
use super::sources::ColumnSources;
use super::state::{ColumnState, ColumnsState, HeaderPeek};
use crate::collection::element::{
    Activation, Collection, Intent, KeyTaken, PointerInput, RowMarks, viewport_path,
};
use crate::collection::keyboard::{KeySettings, Rested};
use crate::collection::pointer::PointerSettings;
use crate::contract::{
    Direction, Dnd, DropAction, DropHover, DropOffer, DropZone, Entry, EntryId, EntrySource, Point,
    Rect,
};
use crate::preview_drawer::{ColumnsFit, DrawerRules};
use crate::scroll::{AutoscrollSettings, BarLook, overlay};

/// What the view reads from the host's state, in one accessor.
pub struct ColumnsParts<'a> {
    /// The listings, root first.
    pub sources: &'a dyn ColumnSources,
    /// The strip's state.
    pub state: &'a ColumnsState,
    /// The look.
    pub look: &'a Look,
}

/// What the view changes in the host's state, in one accessor.
pub struct ColumnsPartsMut<'a> {
    /// The listings, root first.
    pub sources: &'a dyn ColumnSources,
    /// The strip's state.
    pub state: &'a mut ColumnsState,
    /// The look.
    pub look: &'a Look,
}

/// What a column's header actions slot is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HeaderCx {
    /// The column.
    pub depth: usize,
    /// The column holds the keyboard cursor: the paste destination, which its header marks.
    pub active: bool,
    /// The latest pointer or keyboard input targeted it (or an open menu holds it).
    pub targeted: bool,
    /// Its folder is still loading.
    pub loading: bool,
}

/// The words the view shows and assistive technology reads.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ColumnsLabels {
    /// The strip's name.
    pub strip: String,
    /// Each column's close button ("Close this pane").
    pub close: String,
    /// A loading column's spinner.
    pub loading: String,
    /// The caption over a column's edge.
    pub width: String,
    /// What each column's group, named by its folder, is described as ("Columns view").
    pub pane: String,
    /// What each column's list, named by its folder, is described as ("Files").
    pub list: String,
    /// Accessible name template for a clipped column's reveal target (`"{name}"` replaced).
    pub reveal: String,
}

impl Default for ColumnsLabels {
    fn default() -> Self {
        Self {
            strip: "Columns".into(),
            close: "Close this pane".into(),
            loading: "Loading".into(),
            width: "Column width".into(),
            pane: "Columns view".into(),
            list: "Files".into(),
            reveal: "Reveal {name} column".into(),
        }
    }
}

type PartsFn<S> = Rc<dyn for<'a> Fn(&'a S) -> ColumnsParts<'a>>;
type PartsMutFn<S> = Rc<dyn for<'a> Fn(&'a mut S) -> ColumnsPartsMut<'a>>;
type CollFn<S> = Rc<dyn Fn(&S, usize, Collection<S>) -> Collection<S>>;
type ActionsFn<S> = Rc<dyn Fn(&S, HeaderCx) -> Option<El>>;
type MarksFn<S> = Rc<dyn Fn(&S, usize, EntryId) -> RowMarks>;
type PointerFn<S> = Rc<dyn Fn(&S) -> PointerSettings>;
type FlagFn<S> = Rc<dyn Fn(&S) -> bool>;
type IntentFn<S> = Rc<dyn Fn(&mut S, ColumnsIntent, &mut Tx)>;
type DropActionFn<S> =
    Rc<dyn Fn(&S, Option<(usize, Option<EntryId>)>, &DropOffer, Mods) -> Option<DropAction>>;
type ExtraDescriptionFn<S> = (
    Rc<dyn Fn(&S, EntryId) -> u64>,
    Rc<dyn Fn(&S, EntryId) -> Option<String>>,
);

/// What the view was built with, shared by its parts' handlers.
pub(super) struct Shared<S> {
    pub(super) parts: PartsFn<S>,
    pub(super) parts_mut: PartsMutFn<S>,
    collection: Option<CollFn<S>>,
    pub(super) actions: Option<ActionsFn<S>>,
    pub(super) band: Option<ActionsFn<S>>,
    marks: Option<MarksFn<S>>,
    pub(super) extra_description: Option<ExtraDescriptionFn<S>>,
    pointer: PointerFn<S>,
    escape_closes: FlagFn<S>,
    pub(super) close_buttons: FlagFn<S>,
    pub(super) show_sizes: FlagFn<S>,
    pub(super) trailing_edge: FlagFn<S>,
    pub(super) active_header: FlagFn<S>,
    pub(super) dnd: Dnd<S>,
    content: &'static [&'static str],
    drop_action: Option<DropActionFn<S>>,
    on_intent: Option<IntentFn<S>>,
    pub(super) labels: ColumnsLabels,
}

impl<S> Clone for Shared<S> {
    fn clone(&self) -> Self {
        Self {
            parts: self.parts.clone(),
            parts_mut: self.parts_mut.clone(),
            collection: self.collection.clone(),
            actions: self.actions.clone(),
            band: self.band.clone(),
            marks: self.marks.clone(),
            extra_description: self.extra_description.clone(),
            pointer: self.pointer.clone(),
            escape_closes: self.escape_closes.clone(),
            close_buttons: self.close_buttons.clone(),
            show_sizes: self.show_sizes.clone(),
            trailing_edge: self.trailing_edge.clone(),
            active_header: self.active_header.clone(),
            dnd: self.dnd.clone(),
            content: self.content,
            drop_action: self.drop_action.clone(),
            on_intent: self.on_intent.clone(),
            labels: self.labels.clone(),
        }
    }
}

impl<S> Shared<S> {
    pub(super) fn emit(&self, s: &mut S, intent: ColumnsIntent, tx: &mut Tx) {
        if let Some(f) = &self.on_intent {
            f(s, intent, tx);
        }
    }
}

/// An empty listing, for an accessor of a column that has gone.
struct NoEntries;

impl EntrySource for NoEntries {
    fn len(&self) -> usize {
        0
    }
    fn id_at(&self, _index: usize) -> EntryId {
        EntryId::new(0)
    }
    fn index_of(&self, _id: EntryId) -> Option<usize> {
        None
    }
    fn entry(&self, _index: usize) -> Entry<'_> {
        Entry::new(EntryId::new(0), "", crate::contract::EntryKind::File, "")
    }
}

static NO_ENTRIES: NoEntries = NoEntries;

/// The Columns view: see the module docs.
pub struct ColumnsView<S> {
    pub(super) sh: Rc<Shared<S>>,
}

impl<S> Clone for ColumnsView<S> {
    fn clone(&self) -> Self {
        Self {
            sh: self.sh.clone(),
        }
    }
}

impl<S: 'static> ColumnsView<S> {
    /// A view over the listings, the strip's state and the look `parts` returns, changing the state through `parts_mut`.
    pub fn new(
        parts: impl for<'a> Fn(&'a S) -> ColumnsParts<'a> + 'static,
        parts_mut: impl for<'a> Fn(&'a mut S) -> ColumnsPartsMut<'a> + 'static,
    ) -> Self {
        Self {
            sh: Rc::new(Shared {
                parts: Rc::new(parts),
                parts_mut: Rc::new(parts_mut),
                collection: None,
                actions: None,
                band: None,
                marks: None,
                extra_description: None,
                pointer: Rc::new(|_| PointerSettings::default()),
                escape_closes: Rc::new(|_| true),
                close_buttons: Rc::new(|_| true),
                show_sizes: Rc::new(|_| true),
                trailing_edge: Rc::new(|_| true),
                active_header: Rc::new(|_| true),
                dnd: Dnd::rune(),
                content: &[],
                drop_action: None,
                on_intent: None,
                labels: ColumnsLabels::default(),
            }),
        }
    }

    /// Changes the configuration: a view cloned before keeps what it had.
    fn with(mut self, f: impl FnOnce(&mut Shared<S>)) -> Self {
        f(Rc::make_mut(&mut self.sh));
        self
    }

    /// The host's additions to column `depth`'s collection.
    pub fn collection(
        self,
        f: impl Fn(&S, usize, Collection<S>) -> Collection<S> + 'static,
    ) -> Self {
        self.with(|sh| sh.collection = Some(Rc::new(f)))
    }

    /// The pointer controllers' settings: the view adds which folder each column has open (`PointerSettings::already_open`), so a slow second click on it still renames.
    pub fn pointer_settings(self, f: impl Fn(&S) -> PointerSettings + 'static) -> Self {
        self.with(|sh| sh.pointer = Rc::new(f))
    }

    /// A column header's actions (Refresh, Sort by, the filter field, Empty Trash): the host's, placed after the title.
    pub fn header_actions(self, f: impl Fn(&S, HeaderCx) -> Option<El> + 'static) -> Self {
        self.with(|sh| sh.actions = Some(Rc::new(f)))
    }

    /// A band under a column's header, across the column: the host's, for any column it returns one for.
    pub fn header_band(self, f: impl Fn(&S, HeaderCx) -> Option<El> + 'static) -> Self {
        self.with(|sh| sh.band = Some(Rc::new(f)))
    }

    /// The host's marks on each row (cut, dragging, the Enter target); the view adds the open path's.
    pub fn marks(self, f: impl Fn(&S, usize, EntryId) -> RowMarks + 'static) -> Self {
        self.with(|sh| sh.marks = Some(Rc::new(f)))
    }

    /// Extra accessible status text appended to a row's description, invalidated when `version` changes.
    pub fn extra_description(
        self,
        version: impl Fn(&S, EntryId) -> u64 + 'static,
        text: impl Fn(&S, EntryId) -> Option<String> + 'static,
    ) -> Self {
        self.with(|sh| sh.extra_description = Some((Rc::new(version), Rc::new(text))))
    }

    /// Whether Escape with nothing selected in the active column closes the deepest one: on by default.
    pub fn escape_closes(self, f: impl Fn(&S) -> bool + 'static) -> Self {
        self.with(|sh| sh.escape_closes = Rc::new(f))
    }

    /// Whether columns after the first show their close button in the header: on by default.
    pub fn close_buttons(self, f: impl Fn(&S) -> bool + 'static) -> Self {
        self.with(|sh| sh.close_buttons = Rc::new(f))
    }

    /// Whether rows show their size badge (`ColumnSources::badge`): Strata's "Show sizes in Columns view", on by default.
    pub fn show_sizes(self, f: impl Fn(&S) -> bool + 'static) -> Self {
        self.with(|sh| sh.show_sizes = Rc::new(f))
    }

    /// Whether the rightmost column draws its trailing vertical divider line: on by default.
    pub fn trailing_edge(self, f: impl Fn(&S) -> bool + 'static) -> Self {
        self.with(|sh| sh.trailing_edge = Rc::new(f))
    }

    /// Whether the active column's header draws its destination bar, raised fill, and active title colour: on by default.
    pub fn active_header(self, f: impl Fn(&S) -> bool + 'static) -> Self {
        self.with(|sh| sh.active_header = Rc::new(f))
    }

    /// The drag-and-drop binder: Rune's own by default; passes the system shim's.
    pub fn dnd(self, dnd: Dnd<S>) -> Self {
        self.with(|sh| sh.dnd = dnd)
    }

    /// The content types each column's folder rows and background take from other apps besides files, most wanted first.
    pub fn content_types(self, mimes: &'static [&'static str]) -> Self {
        self.with(|sh| sh.content = mimes)
    }

    /// Says which action a drag would take, or `None` to refuse it, for a binder that negotiates while the drag hovers.
    pub fn drop_action(
        self,
        f: impl Fn(&S, Option<(usize, Option<EntryId>)>, &DropOffer, Mods) -> Option<DropAction>
        + 'static,
    ) -> Self {
        self.with(|sh| sh.drop_action = Some(Rc::new(f)))
    }

    /// The words the view shows and assistive technology reads.
    pub fn labels(self, labels: ColumnsLabels) -> Self {
        self.with(|sh| sh.labels = labels)
    }

    /// Hears what the view asks of the host or tells it ([`ColumnsIntent`]).
    pub fn on_intent(self, f: impl Fn(&mut S, ColumnsIntent, &mut Tx) + 'static) -> Self {
        self.with(|sh| sh.on_intent = Some(Rc::new(f)))
    }

    /// Column `depth`'s collection, as the view builds it: for its geometry (a menu anchored at its cursor) or its `tick`.
    pub fn column_collection(&self, s: &S, depth: usize) -> Option<Collection<S>> {
        let p = (self.sh.parts)(s);
        let serial = p.state.columns.get(depth)?.serial;
        (depth < p.sources.len()).then(|| collection(&self.sh, s, serial))
    }

    /// Runs the columns' controllers' timeouts and queued commands, a held drag's autoscroll.
    pub fn tick(&self, s: &mut S, now: f64, tx: &mut Tx) -> bool {
        let serials: Vec<u64> = {
            let p = (self.sh.parts)(s);
            let n = p.sources.len().min(p.state.columns.len());
            p.state.columns[..n]
                .iter()
                .filter(|c| c.collection.needs_tick())
                .map(|c| c.serial)
                .collect()
        };
        let mut changed = false;
        for serial in serials {
            changed |= collection(&self.sh, s, serial).tick(s, tx);
        }
        {
            let p = (self.sh.parts_mut)(s);
            p.state.tick_reveal(now, p.look, tx);
            changed |= p.state.tick_widths(p.look, tx);
            changed |= p.state.sync_clipped(p.look);
            changed |= p.state.sync_strip_overflow(p.look);
        }
        changed | drive(&self.sh, s, now, tx)
    }

    /// Routes a primary drag starting on an external marquee origin surface (such as the preview drawer) into the rightmost open column's collection.
    pub fn origin_drag(&self, s: &mut S, (d, clears_on_press): (&mut Drag, bool), tx: &mut Tx) {
        let (serial, last, was_in_band, press_mods) = {
            let p = (self.sh.parts_mut)(s);
            let n = p.sources.len().min(p.state.columns.len());
            if n == 0 {
                tx.live_only();
                return;
            }
            if clears_selection_on_press(d.phase, clears_on_press, tx.mods()) {
                p.state.clear_selection(p.sources);
            }
            let last = n - 1;
            let col = &p.state.columns[last];
            (
                col.serial,
                last,
                col.collection.selection.in_band(),
                col.collection.origin_press.map(|(_, m)| m),
            )
        };
        collection(&self.sh, s, serial).origin_drag(s, (d, clears_on_press), tx);
        if !was_in_band {
            let plain = is_plain_mods(press_mods.unwrap_or_else(|| tx.mods()));
            self.finish_origin_marquee_start(s, (last, plain), tx);
        }
    }

    fn finish_origin_marquee_start(&self, s: &mut S, (last, plain): (usize, bool), tx: &mut Tx) {
        let started = {
            let p = (self.sh.parts_mut)(s);
            let in_band = p
                .state
                .columns
                .get(last)
                .is_some_and(|c| c.collection.selection.in_band());
            if in_band {
                p.state.targeted = Some(last);
                p.state.set_active_quiet(last);
                if plain {
                    p.state.clear_before(last, p.sources);
                }
                Some(p.state.key.clone())
            } else {
                None
            }
        };
        if let Some(key) = started {
            tx.focus(key, false);
            self.sh.emit(s, ColumnsIntent::Active { depth: last }, tx);
        }
    }

    /// What the strip tells drawer about itself, for `PreviewDrawer::arrangement` in Columns.
    pub fn fit(&self, s: &S, rules: &DrawerRules) -> ColumnsFit {
        let p = (self.sh.parts)(s);
        fit::columns_fit(p.state, p.look, rules)
    }

    /// Where a drop at window point `window` would go.
    pub fn drop_target_at(&self, s: &S, window: Point) -> Option<(usize, Option<EntryId>)> {
        let p = (self.sh.parts)(s);
        let geo = geometry(p.state, p.look);
        let depth = super::drag_scroll::column_at(&geo, window)?;
        if depth >= p.sources.len() {
            return None;
        }
        let col_geo = geo.columns[depth];
        if !col_geo.listing.contains(window) {
            return Some((depth, None));
        }
        let c = self.column_collection(s, depth)?;
        let point = Point::new(0.0, window.y - col_geo.listing.y + col_geo.offset);
        let source = p.sources.source(depth);
        let row = c
            .geometry(s)
            .index_at(point)
            .filter(|&i| source.entry(i).kind.is_folder_like())
            .map(|i| source.id_at(i));
        Some((depth, row))
    }

    /// The strip: one Tab stop, whose keys go to the active column.
    pub fn build(&self, s: &S) -> El {
        super::strip::build(&self.sh, s)
    }
}

fn is_plain_mods(m: Mods) -> bool {
    !(m.ctrl || m.shift || m.cmd)
}

fn clears_selection_on_press(phase: Phase, clears: bool, m: Mods) -> bool {
    phase == Phase::Down && clears && is_plain_mods(m)
}

/// Vertical gap on each side of a column row inside its `look.row.columns` slot.
pub(super) const ROW_GAP_Y: f32 = 1.0;

fn row_height<S: 'static>(sh: &Shared<S>, s: &S, serial: u64, look: &Look) -> f32 {
    let p = (sh.parts)(s);
    let details = p
        .state
        .depth_of(serial)
        .filter(|d| *d < p.sources.len())
        .is_some_and(|d| p.sources.has_details(d));
    match details {
        true => super::badge::detailed_height(look),
        false => look.row.columns,
    }
}

/// Column `serial`'s collection as the view builds it.
pub(super) fn collection<S: 'static>(sh: &Rc<Shared<S>>, s: &S, serial: u64) -> Collection<S> {
    let p = (sh.parts)(s);
    let depth = p.state.depth_of(serial).unwrap_or(0);
    let key = keys::column(&p.state.key, serial);
    let c = new_column_collection(sh, serial, key.clone()).description(sh.labels.list.clone());
    let c = hosted(sh, s, depth, c);
    let look = p.look;
    let column = &p.state.columns[depth.min(p.state.columns.len().saturating_sub(1))];
    let title = if depth < p.sources.len() {
        p.sources.title(depth).to_owned()
    } else {
        String::new()
    };
    let already_open = (depth < p.sources.len())
        .then(|| p.sources.open_child(depth))
        .flatten();
    let pointer = (sh.pointer)(s).already_open(already_open);
    let threshold = pointer.drag_threshold;
    let autoscroll = AutoscrollSettings {
        margin: strata_kit::slots::collection::row_inset(look),
        ..AutoscrollSettings::STRATA
    };
    let (marks, intents, press, hdr_click, cursor, selection) = (
        sh.clone(),
        sh.clone(),
        sh.clone(),
        sh.clone(),
        sh.clone(),
        sh.clone(),
    );
    let hdr = sh.clone();
    let tall = sh.clone();
    let c = c
        .name(title)
        .row_height(move |s: &S, l: &Look| row_height(&tall, s, serial, l))
        .row_gap_y(ROW_GAP_Y)
        .keys_by_host()
        .settings(pointer)
        .settings(autoscroll)
        .header(move |s: &S| {
            let depth = depth_of(&hdr, s, serial)?;
            Some(column::header(&hdr, s, depth, serial))
        })
        .header_defers_press(threshold)
        .on_header_click(move |s: &mut S, input, tx| {
            header_click(&hdr_click, s, (serial, input), tx)
        })
        .marks(move |s: &S, id| row_marks(&marks, s, serial, id))
        .on_intent(move |s: &mut S, intent, tx| column_intent(&intents, s, serial, intent, tx))
        .on_press(move |s: &mut S, input, tx| column_press(&press, s, serial, input, tx))
        .on_cursor(move |s: &mut S, id, tx| {
            if let Some(depth) = depth_of(&cursor, s, serial) {
                cursor.emit(s, ColumnsIntent::Cursor { depth, id }, tx);
            }
        })
        .on_selection(move |s: &mut S, tx| {
            if let Some(depth) = depth_of(&selection, s, serial) {
                selection.emit(s, ColumnsIntent::Selection { depth }, tx);
            }
        })
        .content(super::badge::content(sh, serial))
        .describe(super::badge::describe(sh, serial));
    let c = attach_column_dnd(sh, serial, c);
    attach_column_scroll(sh, column, &key, c)
}

fn attach_column_dnd<S: 'static>(
    sh: &Rc<Shared<S>>,
    serial: u64,
    mut c: Collection<S>,
) -> Collection<S> {
    if let Some(f) = sh.drop_action.clone() {
        let act = sh.clone();
        c = c.drop_action(move |s: &S, target, offer, mods| {
            f(s, Some((depth_of(&act, s, serial)?, target)), offer, mods)
        });
    }
    let (drops, over) = (sh.clone(), sh.clone());
    c.dnd(sh.dnd.clone())
        .content_types(sh.content)
        .on_drop(move |s: &mut S, target, dropped, tx| {
            (drops.parts_mut)(s).state.drag_over(None);
            if let Some(depth) = depth_of(&drops, s, serial) {
                drops.emit(
                    s,
                    ColumnsIntent::Dropped {
                        depth,
                        target,
                        dropped,
                    },
                    tx,
                );
            }
        })
        .on_drag_over(move |s: &mut S, at, tx| drag_over(&over, s, at, tx))
}

fn attach_column_scroll<S: 'static>(
    sh: &Rc<Shared<S>>,
    column: &ColumnState,
    key: &str,
    c: Collection<S>,
) -> Collection<S> {
    let (feed, bar, edge_bar) = (column.feed.clone(), column.bar.clone(), column.bar.clone());
    let bar_key = format!("{key}-bar");
    let bar_target = viewport_path(key);
    let lk = sh.clone();
    c.on_scroll(move |p| {
        feed.set(p);
        bar.follow(p);
    })
    .edge(move |s: &S| {
        let look = (lk.parts)(s).look;
        Some(edge_bar.build::<S>(&bar_key, &bar_target, BarLook::column(look)))
    })
}

fn new_column_collection<S: 'static>(
    sh: &Rc<Shared<S>>,
    serial: u64,
    key: String,
) -> Collection<S> {
    let (g, gm, lk, ver) = (sh.clone(), sh.clone(), sh.clone(), sh.clone());
    Collection::new(
        key,
        move |s: &S| {
            let p = (g.parts)(s);
            match p.state.depth_of(serial).filter(|d| *d < p.sources.len()) {
                Some(d) => (p.sources.source(d), &p.state.columns[d].collection),
                None => (&NO_ENTRIES as &dyn EntrySource, &p.state.spare),
            }
        },
        move |s: &mut S| {
            let p = (gm.parts_mut)(s);
            match p.state.depth_of(serial).filter(|d| *d < p.sources.len()) {
                Some(d) => (p.sources.source(d), &mut p.state.columns[d].collection),
                None => (&NO_ENTRIES as &dyn EntrySource, &mut p.state.spare),
            }
        },
        move |s: &S| (lk.parts)(s).look,
        move |s: &S| {
            let p = (ver.parts)(s);
            p.state
                .depth_of(serial)
                .filter(|d| *d < p.sources.len())
                .map_or(0, |d| p.sources.version(d))
        },
    )
}

fn hosted<S: 'static>(sh: &Shared<S>, s: &S, depth: usize, c: Collection<S>) -> Collection<S> {
    let c = match &sh.collection {
        Some(f) => f(s, depth, c),
        None => c,
    };
    let mut keys = c.setting::<KeySettings>().cloned().unwrap_or_default();
    keys.reselects_in_place = true;
    keys.select_all_to_last = true;
    keys.left_leaves_without_cursor = true;
    c.settings(keys)
}

/// The depth of column `serial`, while it is open and listed.
pub(super) fn depth_of<S>(sh: &Shared<S>, s: &S, serial: u64) -> Option<usize> {
    let p = (sh.parts)(s);
    p.state.depth_of(serial).filter(|d| *d < p.sources.len())
}

fn row_marks<S>(sh: &Shared<S>, s: &S, serial: u64, id: EntryId) -> RowMarks {
    let Some(depth) = depth_of(sh, s, serial) else {
        return RowMarks::default();
    };
    let mut marks = sh
        .marks
        .as_ref()
        .map_or_else(RowMarks::default, |f| f(s, depth, id));
    let p = (sh.parts)(s);
    if p.sources.open_child(depth) == Some(id) && depth + 1 < p.state.columns.len() {
        marks.active_path = true;
        marks.active_parent = depth + 1 == p.state.active;
    }
    marks
}

fn column_intent<S: 'static>(
    sh: &Rc<Shared<S>>,
    s: &mut S,
    serial: u64,
    intent: Intent,
    tx: &mut Tx,
) {
    let Some(depth) = depth_of(sh, s, serial) else {
        return;
    };
    match intent {
        Intent::Edge(Direction::Right) => enter_right(sh, s, depth, tx),
        Intent::Edge(Direction::Left) => go_left(sh, s, depth, tx),
        Intent::Activate {
            id,
            how: Activation::Key,
        } if reopens(sh, s, depth, id) => enter_child(sh, s, depth + 1, tx),
        Intent::Activate { id, .. } if reopens(sh, s, depth, id) => {
            let p = (sh.parts_mut)(s);
            p.state.set_active(depth + 1, p.look, tx);
            sh.emit(s, ColumnsIntent::Active { depth: depth + 1 }, tx);
        }
        Intent::Other(any) if any.downcast_ref::<Rested>().is_some() => {
            let p = (sh.parts)(s);
            let id = p.state.columns[depth].collection.selection.cursor();
            sh.emit(s, ColumnsIntent::Cursor { depth, id }, tx);
        }
        Intent::Other(any) if any.downcast_ref::<PeekClick>().is_some() => {
            let pt = any.downcast_ref::<PeekClick>().expect("checked").0;
            reveal_clipped(sh, s, depth, (pt, tx));
        }
        Intent::Other(any) if any.downcast_ref::<BackgroundClick>().is_some() => {
            background_click(sh, s, depth, tx);
        }
        intent => {
            note_opened(sh, s, depth, &intent);
            sh.emit(s, ColumnsIntent::Collection { depth, intent }, tx);
        }
    }
}

fn note_opened<S>(sh: &Shared<S>, s: &mut S, depth: usize, intent: &Intent) {
    let Intent::Activate { id, .. } = intent else {
        return;
    };
    let p = (sh.parts_mut)(s);
    let source = p.sources.source(depth);
    let folder = source
        .index_of(*id)
        .is_some_and(|i| source.entry(i).kind.is_folder_like());
    if folder {
        p.state.opened = Some((depth, *id));
    }
}

/// Whether activating `id` in column `depth` goes to its open column instead.
fn reopens<S>(sh: &Shared<S>, s: &S, depth: usize, id: EntryId) -> bool {
    let p = (sh.parts)(s);
    p.sources.open_child(depth) == Some(id) && depth + 1 < p.state.columns.len()
}

fn enter_child<S: 'static>(sh: &Rc<Shared<S>>, s: &mut S, child: usize, tx: &mut Tx) {
    let p = (sh.parts_mut)(s);
    let listing = if p.sources.loading(child) {
        crate::collection::selection::Listing::Loading
    } else {
        crate::collection::selection::Listing::Complete
    };
    p.state.set_active_quiet(child);
    p.state
        .place_first(child, p.sources.source(child), listing, tx);
    p.state.reveal(p.look, tx);
    sh.emit(s, ColumnsIntent::Active { depth: child }, tx);
}

fn enter_right<S: 'static>(sh: &Rc<Shared<S>>, s: &mut S, depth: usize, tx: &mut Tx) {
    let p = (sh.parts)(s);
    let source = p.sources.source(depth);
    let folder = p.state.columns[depth]
        .collection
        .selection
        .cursor_index(source)
        .filter(|&i| source.entry(i).kind.is_folder_like())
        .map(|i| source.id_at(i));
    let child_open = depth + 1 < p.state.columns.len() && depth + 1 < p.sources.len();
    let shown = p.sources.open_child(depth);
    if child_open && folder.is_none_or(|id| shown == Some(id)) {
        enter_child(sh, s, depth + 1, tx);
    } else if let Some(id) = folder {
        sh.emit(s, ColumnsIntent::EnterFolder { depth, id }, tx);
    }
}

fn go_left<S: 'static>(sh: &Rc<Shared<S>>, s: &mut S, depth: usize, tx: &mut Tx) {
    if depth == 0 {
        sh.emit(s, ColumnsIntent::LeftOfRoot, tx);
        return;
    }
    let p = (sh.parts_mut)(s);
    let parent = depth - 1;
    p.state.set_active_quiet(parent);
    p.state.place_open(parent, p.sources, tx);
    p.state.reveal(p.look, tx);
    sh.emit(s, ColumnsIntent::Active { depth: parent }, tx);
}

fn background_click<S: 'static>(sh: &Rc<Shared<S>>, s: &mut S, depth: usize, tx: &mut Tx) {
    let p = (sh.parts_mut)(s);
    let was_active = p.state.press_was_active;
    let more = depth + 1 < p.state.columns.len();
    p.state.clear_all(p.sources);
    if !was_active {
        let source = p.sources.source(depth);
        let coll = &mut p.state.columns[depth].collection;
        let first = coll.visible_range().start;
        if first < source.len() {
            let _ = coll.selection.select_only(source, first);
            coll.note_selection(source);
        }
        p.state.set_active_quiet(depth);
        p.state.reveal(p.look, tx);
    }
    if more {
        sh.emit(s, ColumnsIntent::CloseFrom(depth + 1), tx);
    }
}

fn column_press<S: 'static>(
    sh: &Rc<Shared<S>>,
    s: &mut S,
    serial: u64,
    input: &PointerInput,
    _tx: &mut Tx,
) {
    let Some(depth) = depth_of(sh, s, serial) else {
        return;
    };
    let mut p = (sh.parts_mut)(s);
    reset_other_columns(p.state, depth);
    p.state.press_was_active = p.state.active == depth;
    p.state.targeted = Some(depth);
    if input.button == PointerButton::Middle || classify_peek_press(&mut p, depth, input) {
        return;
    }
    if !is_opening_second_click(&p, depth, input) && p.state.active != depth {
        p.state.set_active_quiet(depth);
    }
}

fn reset_other_columns(state: &mut ColumnsState, active_depth: usize) {
    for (i, c) in state.columns.iter_mut().enumerate() {
        if i != active_depth
            && let Some(ctl) = c.collection.controllers.pointer.as_mut()
        {
            ctl.reset();
        }
    }
}

fn is_opening_second_click(p: &ColumnsPartsMut<'_>, depth: usize, input: &PointerInput) -> bool {
    if input.clicks < 2 || p.state.active != depth + 1 {
        return false;
    }
    let hit = input
        .hit
        .index
        .filter(|&i| i < p.sources.source(depth).len())
        .map(|i| p.sources.source(depth).id_at(i));
    hit.is_some_and(|id| p.state.opened == Some((depth, id)))
}

fn classify_peek_press(p: &mut ColumnsPartsMut<'_>, depth: usize, input: &PointerInput) -> bool {
    if input.button != PointerButton::Primary {
        return false;
    }
    let on_header = !p.state.columns[depth]
        .collection
        .viewport()
        .contains(input.window);
    if p.state.consume_peek_double_click(input.now, input.window) {
        p.state.header_peek = Some(HeaderPeek::Suppress);
        p.state.columns[depth].peek_gate.set(PeekGate::Suppress);
        return true;
    }
    if input.hit.index.is_none() && p.state.clipped_slice(depth, p.look).is_some() {
        if on_header {
            p.state.header_peek = Some(HeaderPeek::Peek(depth));
        } else {
            p.state.columns[depth].peek_gate.set(PeekGate::Peek);
        }
        return true;
    }
    if on_header {
        p.state.header_peek = Some(HeaderPeek::Normal(depth));
    }
    false
}

fn header_click<S: 'static>(
    sh: &Rc<Shared<S>>,
    s: &mut S,
    (serial, input): (u64, &PointerInput),
    tx: &mut Tx,
) {
    let Some(depth) = depth_of(sh, s, serial) else {
        tx.live_only();
        return;
    };
    let kind = (sh.parts_mut)(s).state.header_peek.take();
    match kind {
        Some(HeaderPeek::Suppress) => {
            tx.live_only();
        }
        Some(HeaderPeek::Peek(d)) if d == depth => {
            reveal_clipped(sh, s, depth, (input.window, tx));
        }
        Some(HeaderPeek::Normal(d)) if d == depth => {
            let p = (sh.parts_mut)(s);
            p.state.set_active_quiet(depth);
            sh.emit(s, ColumnsIntent::Active { depth }, tx);
        }
        _ => {
            tx.live_only();
        }
    }
}

fn reveal_clipped<S: 'static>(
    sh: &Rc<Shared<S>>,
    s: &mut S,
    depth: usize,
    (window, tx): (Point, &mut Tx),
) {
    {
        let p = (sh.parts_mut)(s);
        let now = tx.clock();
        p.state.peek_seq = Some((now, window));
        p.state.set_active_quiet(depth);
        p.state.reveal(p.look, tx);
    }
    sh.emit(s, ColumnsIntent::Active { depth }, tx);
}

pub(super) fn strip_key<S: 'static>(
    sh: &Rc<Shared<S>>,
    s: &mut S,
    event: &KeyEvent,
    tx: &mut Tx,
) -> bool {
    let (serial, len) = {
        let p = (sh.parts_mut)(s);
        let n = p.state.columns.len().min(p.sources.len());
        if n == 0 {
            return false;
        }
        let active = p.state.active.min(n - 1);
        p.state.targeted = Some(active);
        (p.state.columns[active].serial, n)
    };
    match collection(sh, s, serial).key_taken(s, event, tx) {
        KeyTaken::ByController => {
            let p = (sh.parts_mut)(s);
            p.state.reveal(p.look, tx);
            return true;
        }
        KeyTaken::ByHost => return true,
        KeyTaken::No => {}
    }
    let m = event.mods;
    if event.key == Key::Escape
        && !(m.ctrl || m.alt || m.cmd || m.logo)
        && len > 1
        && (sh.escape_closes)(s)
    {
        sh.emit(s, ColumnsIntent::CloseFrom(len - 1), tx);
        return true;
    }
    false
}

/// A press on the blank space after the last column clears the selection in every column.
pub(super) fn blank_press<S: 'static>(sh: &Rc<Shared<S>>, s: &mut S) {
    let p = (sh.parts_mut)(s);
    p.state.clear_selection(p.sources);
}

pub(super) fn drag_over<S: 'static>(
    sh: &Rc<Shared<S>>,
    s: &mut S,
    at: Option<Point>,
    tx: &mut Tx,
) -> bool {
    let p = (sh.parts_mut)(s);
    if p.state.drag_over(at) {
        tx.wake_after(std::time::Duration::ZERO);
    }
    let geo = geometry(p.state, p.look);
    p.state.drag.hover(&geo)
}

pub(super) fn blank_zone<S: 'static>(sh: &Rc<Shared<S>>) -> DropZone<S> {
    let (hover, moves) = (sh.clone(), sh.clone());
    let zone = DropZone::new(|_: &mut S, _, _| {});
    let zone = match sh.drop_action.clone() {
        Some(f) => zone.action(move |s: &S, offer, mods| f(s, None, offer, mods)),
        None => zone,
    };
    zone.on_hover(move |s: &mut S, h, _offer, tx| {
        let at = match h {
            DropHover::Left => None,
            DropHover::Accepting | DropHover::Rejecting => {
                match tx.drop_point().map(|p| Point::new(p.window_x, p.window_y)) {
                    Some(at) => Some(at),
                    None if h == DropHover::Rejecting => return,
                    None => None,
                }
            }
        };
        drag_over(&hover, s, at, tx);
    })
    .on_move(move |s: &mut S, p, tx| {
        if !drag_over(&moves, s, Some(Point::new(p.window_x, p.window_y)), tx) {
            tx.live_only();
        }
    })
}

pub(super) fn geometry(st: &ColumnsState, look: &Look) -> StripGeometry {
    let feed = st.strip_feed.get();
    let offset = feed.offset[0];
    let sd = st.strip_drawn.get();
    let strip = if sd[2] > 0.0 || sd[3] > 0.0 {
        Rect::from(sd)
    } else {
        st.strip
    };
    let x0 = strip.x - offset;
    let columns = st
        .spans(look)
        .into_iter()
        .zip(&st.columns)
        .map(|((l, r), c)| {
            let pos = c.feed.get();
            let cd = c.drawn.get();
            let (left, right) = if cd[2] > 0.0 {
                (cd[0], cd[0] + cd[2])
            } else {
                (x0 + l, x0 + r)
            };
            let v = c.collection.viewport();
            ColumnGeometry {
                left,
                right,
                listing: Rect::new(left, v.y, (right - left).max(0.0), v.h),
                offset: pos.offset[1],
                max: pos.max[1],
            }
        })
        .collect();
    StripGeometry {
        strip,
        offset,
        max: feed.max[0],
        columns,
    }
}

/// One frame of a held drag's autoscroll. True when the drop highlight moved.
fn drive<S: 'static>(sh: &Rc<Shared<S>>, s: &mut S, now: f64, tx: &mut Tx) -> bool {
    let p = (sh.parts_mut)(s);
    if p.state.drag.pointer().is_none() && !p.state.drag.scrolling() {
        return false;
    }
    let geo = geometry(p.state, p.look);
    let before = p.state.drag.hovered();
    let step = p.state.drag.step(now, &geo, p.look.animations);
    let strip = keys::strip(&p.state.key);
    match step.strip {
        Some(Scroll::Glide(x)) => {
            tx.scroll_to_with(strip, x, 0.0, super::drag_scroll::GLIDE);
        }
        Some(Scroll::Instant(x)) => tx.scroll_to_instant(strip, x, 0.0),
        Some(Scroll::Stop) => tx.stop_scroll(strip),
        None => {}
    }
    if let Some((depth, scroll)) = step.listing
        && let Some(path) = p.state.listing_path(depth)
    {
        match scroll {
            Scroll::Glide(y) => tx.scroll_to_with(path, 0.0, y, super::drag_scroll::GLIDE),
            Scroll::Instant(y) => tx.scroll_to_instant(path, 0.0, y),
            Scroll::Stop => tx.stop_scroll(path),
        }
    }
    if step.wake {
        tx.wake_after(std::time::Duration::ZERO);
    }
    p.state.drag.hovered() != before
}

/// The strip's row of columns and its blank space.
pub(super) fn columns_row<S: 'static>(sh: &Rc<Shared<S>>, s: &S) -> El {
    let p = (sh.parts)(s);
    let n = p.state.columns.len().min(p.sources.len());
    let mut content = row().key("columns").h_full().min_h(0.0);
    let overflows = p.state.strip_overflows_now(p.look);
    p.state.strip_overflows.set(overflows);
    if overflows {
        let bar = BarLook::strip(p.look);
        content = content.pb(bar.edge + bar.band());
    }
    for depth in 0..n {
        content = content.child(column::shell(sh, s, depth));
    }
    let blank = sh.clone();
    let blank_key = keys::blank(&p.state.key);
    let blank_el = col()
        .key(blank_key.clone())
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .a11y_hidden()
        .on_background_press(move |s: &mut S| blank_press(&blank, s));
    content.child(sh.dnd.drop_zone_keyed(blank_el, blank_key, blank_zone(sh)))
}

/// The strip's viewport with its bar over it.
pub(super) fn with_bar<S: 'static>(st: &ColumnsState, look: &Look, viewport: El) -> El {
    let bar = st.strip_bar.build::<S>(
        &format!("{}-bar", keys::strip(&st.key)),
        &keys::strip(&st.key),
        BarLook::strip(look),
    );
    overlay(st.strip_bar.area(viewport), [bar])
}
