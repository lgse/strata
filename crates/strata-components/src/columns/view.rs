//! [`ColumnsView`]: the builder, the collection each column builds, and how keys, presses,
//! intents and drags reach the strip.

use std::rc::Rc;

use rune::{El, Key, KeyEvent, Mods, PointerButton, Tx, col, row};
use strata_kit::look::Look;

use super::column;
use super::drag_scroll::{ColumnGeometry, Scroll, StripGeometry};
use super::fit;
use super::intent::ColumnsIntent;
use super::keys;
use super::pointer::BackgroundClick;
use super::sources::ColumnSources;
use super::state::ColumnsState;
use crate::collection::element::{Collection, Intent, PointerInput, RowMarks, viewport_path};
use crate::collection::pointer::PointerSettings;
use crate::contract::{
    Direction, Dnd, DropAction, DropHover, DropOffer, DropZone, Entry, EntryId, EntrySource, Point,
};
use crate::preview_drawer::{ColumnsFit, DrawerRules};
use crate::scroll::{AutoscrollSettings, BarLook, overlay};

/// What the view reads from the host's state, in one accessor.
///
/// Used by: hosts.
pub struct ColumnsParts<'a> {
    /// The listings, root first.
    pub sources: &'a dyn ColumnSources,
    /// The strip's state.
    pub state: &'a ColumnsState,
    /// The look.
    pub look: &'a Look,
}

/// What the view changes in the host's state, in one accessor: the strip's state, beside the
/// listings and the look it reads.
///
/// Used by: hosts.
pub struct ColumnsPartsMut<'a> {
    /// The listings, root first.
    pub sources: &'a dyn ColumnSources,
    /// The strip's state.
    pub state: &'a mut ColumnsState,
    /// The look.
    pub look: &'a Look,
}

/// What a column's header actions slot is told.
///
/// Used by: hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HeaderCx {
    /// The column.
    pub depth: usize,
    /// The column holds the keyboard cursor: the paste destination, which its header marks.
    pub active: bool,
    /// The latest pointer or keyboard input targeted it (or an open menu holds it): Strata
    /// shows its header actions (Refresh, Sort by, the filter, Empty Trash) only then.
    pub targeted: bool,
    /// Its folder is still loading.
    pub loading: bool,
}

/// The words the view shows and assistive technology reads. Strata passes translated text;
/// the defaults are English.
///
/// Used by: hosts.
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
}

impl Default for ColumnsLabels {
    fn default() -> Self {
        Self {
            strip: "Columns".into(),
            close: "Close this pane".into(),
            loading: "Loading".into(),
            width: "Column width".into(),
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

/// What the view was built with, shared by its parts' handlers.
pub(super) struct Shared<S> {
    pub(super) parts: PartsFn<S>,
    pub(super) parts_mut: PartsMutFn<S>,
    collection: Option<CollFn<S>>,
    pub(super) actions: Option<ActionsFn<S>>,
    marks: Option<MarksFn<S>>,
    pointer: PointerFn<S>,
    escape_closes: FlagFn<S>,
    pub(super) show_sizes: FlagFn<S>,
    pub(super) dnd: Dnd<S>,
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
            marks: self.marks.clone(),
            pointer: self.pointer.clone(),
            escape_closes: self.escape_closes.clone(),
            show_sizes: self.show_sizes.clone(),
            dnd: self.dnd.clone(),
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

/// The Columns view: see the [module docs](super).
///
/// The view owns two of each column collection's slots: the rows' content
/// (`Collection::content`: the icon, the label and the size badge) and their description
/// (`Collection::describe`: the badge's text). A host sets neither in
/// [`ColumnsView::collection`]; it gives the badge's words through [`ColumnSources::badge`] and
/// its setting through [`ColumnsView::show_sizes`].
///
/// Used by: hosts (Strata's browser and chooser, the gallery).
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
    /// A view over the listings, the strip's state and the look `parts` returns, changing the
    /// state through `parts_mut`.
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
                marks: None,
                pointer: Rc::new(|_| PointerSettings::default()),
                escape_closes: Rc::new(|_| true),
                show_sizes: Rc::new(|_| true),
                dnd: Dnd::rune(),
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

    /// The host's additions to column `depth`'s collection: its clock, its controllers'
    /// settings (except [`PointerSettings`], which go through
    /// [`ColumnsView::pointer_settings`]), a label or icon slot, a rename editor. The view sets
    /// the rest after (the name, row height, header, marks, intents, drops and keys), so
    /// don't set those here. The view owns the rows' content slot (`Collection::content`: the
    /// icon, the label and the size badge) and their description (`Collection::describe`: the
    /// badge's text), so don't set those either.
    pub fn collection(
        self,
        f: impl Fn(&S, usize, Collection<S>) -> Collection<S> + 'static,
    ) -> Self {
        self.with(|sh| sh.collection = Some(Rc::new(f)))
    }

    /// The pointer controllers' settings (P5's double-click interval, the click modes): the
    /// view adds which folder each column has open (`PointerSettings::already_open`), so a slow
    /// second click on it still renames.
    pub fn pointer_settings(self, f: impl Fn(&S) -> PointerSettings + 'static) -> Self {
        self.with(|sh| sh.pointer = Rc::new(f))
    }

    /// A column header's actions (Refresh, Sort by, the filter field, Empty Trash): the host's,
    /// placed after the title. The view adds the close button to the targeted column after the
    /// first. A secondary press on these is theirs, not the folder menu's. Build them
    /// `.focusable(false)`, as the close button is: the strip is the view's one focus stop, and a
    /// press on a focusable button would take the keyboard from it (Strata's header buttons
    /// don't take focus on a click).
    pub fn header_actions(self, f: impl Fn(&S, HeaderCx) -> Option<El> + 'static) -> Self {
        self.with(|sh| sh.actions = Some(Rc::new(f)))
    }

    /// The host's marks on each row (cut, dragging, the Enter target); the view adds the open
    /// path's.
    pub fn marks(self, f: impl Fn(&S, usize, EntryId) -> RowMarks + 'static) -> Self {
        self.with(|sh| sh.marks = Some(Rc::new(f)))
    }

    /// Whether Escape with nothing selected in the active column closes the deepest one: on by
    /// default; off in 10xer mode, where Escape with nothing to dismiss does nothing.
    pub fn escape_closes(self, f: impl Fn(&S) -> bool + 'static) -> Self {
        self.with(|sh| sh.escape_closes = Rc::new(f))
    }

    /// Whether rows show their size badge ([`ColumnSources::badge`]): Strata's "Show sizes in
    /// Columns view", on by default. Off, no row shows one, and assistive technology hears no
    /// size; a count that arrives while it is off stays hidden.
    pub fn show_sizes(self, f: impl Fn(&S) -> bool + 'static) -> Self {
        self.with(|sh| sh.show_sizes = Rc::new(f))
    }

    /// The drag-and-drop binder: Rune's own by default; X2 passes the system shim's.
    pub fn dnd(self, dnd: Dnd<S>) -> Self {
        self.with(|sh| sh.dnd = dnd)
    }

    /// Says which action a drag would take, or `None` to refuse it, for a binder that
    /// negotiates while the drag hovers (W1's `DropZone::action`): over column `depth`'s folder
    /// row (`Some((depth, Some(id)))`), its background (`Some((depth, None))`), or the blank
    /// space after the last column (`None`, which takes no drop: refuse it there unless the host
    /// routes it itself). Without it, W1's rule with an unknown volume, everywhere.
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

    /// Column `depth`'s collection, as the view builds it: for its geometry (a menu anchored at
    /// its cursor) or its `tick`. `None` past the last column.
    pub fn column_collection(&self, s: &S, depth: usize) -> Option<Collection<S>> {
        let p = (self.sh.parts)(s);
        let serial = p.state.columns.get(depth)?.serial;
        (depth < p.sources.len()).then(|| collection(&self.sh, s, serial))
    }

    /// Runs the columns' controllers' timeouts and queued commands, a held drag's autoscroll,
    /// and the widths' following of the text size and the area: call it from the host's poll
    /// on every frame, with the host's clock in seconds. True when state the view reads changed. The window's middle-click autoscroll is
    /// the host's to drive (`ColumnsState::autoscroll`).
    pub fn tick(&self, s: &mut S, now: f64, tx: &mut Tx) -> bool {
        let serials: Vec<u64> = {
            let p = (self.sh.parts)(s);
            let n = p.sources.len().min(p.state.columns.len());
            p.state.columns[..n].iter().map(|c| c.serial).collect()
        };
        let mut changed = false;
        for serial in serials {
            changed |= collection(&self.sh, s, serial).tick(s, tx);
        }
        {
            let p = (self.sh.parts_mut)(s);
            p.state.tick_reveal(now, p.look, tx);
            changed |= p.state.tick_widths(p.look, tx);
        }
        changed | drive(&self.sh, s, now, tx)
    }

    /// What the strip tells C9's drawer about itself, for `PreviewDrawer::arrangement` in
    /// Columns: the columns from the root through the active one, the width that keeps the
    /// active one whole, and the columns after it, which borrow the empty slot's width.
    pub fn fit(&self, s: &S, rules: &DrawerRules) -> ColumnsFit {
        let p = (self.sh.parts)(s);
        fit::columns_fit(p.state, p.look, rules)
    }

    /// Where a drop at window point `window` would go: the column, and its folder row or its
    /// background, by the strip's spans and each column's index geometry (mounted rows or
    /// not). For a binder that routes drops itself (X2).
    pub fn drop_target_at(&self, s: &S, window: Point) -> Option<(usize, Option<EntryId>)> {
        let p = (self.sh.parts)(s);
        let geo = geometry(p.state, p.look);
        let depth = super::drag_scroll::column_at(&geo, window)?;
        if depth >= p.sources.len() {
            return None;
        }
        let coll = &p.state.columns[depth].collection;
        if !coll.viewport().contains(window) {
            return Some((depth, None));
        }
        let c = self.column_collection(s, depth)?;
        let point = coll.content_point(window);
        let source = p.sources.source(depth);
        let row = c
            .geometry(s)
            .index_at(point)
            .filter(|&i| source.entry(i).kind.is_folder_like())
            .map(|i| source.id_at(i));
        Some((depth, row))
    }

    /// The strip: one Tab stop, whose keys go to the active column. Build it on every view;
    /// don't memoise it (`memo`), since the strip reads the state the handlers change between
    /// commits (the active and targeted column, a resize's live width), and each column's
    /// collection memoises its own rows.
    pub fn build(&self, s: &S) -> El {
        super::strip::build(&self.sh, s)
    }
}

/// Column `serial`'s collection as the view builds it.
pub(super) fn collection<S: 'static>(sh: &Rc<Shared<S>>, s: &S, serial: u64) -> Collection<S> {
    let p = (sh.parts)(s);
    let depth = p.state.depth_of(serial).unwrap_or(0);
    let key = keys::column(&p.state.key, serial);
    let (g, gm, lk, ver) = (sh.clone(), sh.clone(), sh.clone(), sh.clone());
    let mut c = Collection::new(
        key.clone(),
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
    );
    if let Some(f) = &sh.collection {
        c = f(s, depth, c);
    }
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
    let (marks, intents, press, cursor, selection, drops, over) = (
        sh.clone(),
        sh.clone(),
        sh.clone(),
        sh.clone(),
        sh.clone(),
        sh.clone(),
        sh.clone(),
    );
    let header = column::header(sh, s, depth, serial);
    let header = std::cell::RefCell::new(Some(header));
    let (feed, bar) = (column.feed.clone(), column.bar.clone());
    let edge_bar = column.bar.clone();
    let bar_key = format!("{key}-bar");
    let bar_target = viewport_path(&key);
    if let Some(f) = sh.drop_action.clone() {
        let act = sh.clone();
        c = c.drop_action(move |s: &S, target, offer, mods| {
            f(s, Some((depth_of(&act, s, serial)?, target)), offer, mods)
        });
    }
    c.name(title)
        .row_height(|_: &S, l: &Look| l.row.columns)
        .keys_by_host()
        .settings(pointer)
        .settings(autoscroll)
        .header(move |_: &S| header.borrow_mut().take())
        .header_defers_press(threshold)
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
        .dnd(sh.dnd.clone())
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
        .content(super::badge::content(sh, serial))
        .describe(super::badge::describe(sh, serial))
        .on_scroll(move |p| {
            feed.set(p);
            bar.follow(p);
        })
        .edge({
            let lk = sh.clone();
            move |s: &S| {
                let look = (lk.parts)(s).look;
                Some(edge_bar.build::<S>(&bar_key, &bar_target, BarLook::column(look)))
            }
        })
}

/// The depth of column `serial`, while it is open and listed.
pub(super) fn depth_of<S>(sh: &Shared<S>, s: &S, serial: u64) -> Option<usize> {
    let p = (sh.parts)(s);
    p.state.depth_of(serial).filter(|d| *d < p.sources.len())
}

/// The host's marks, and the open path: the folder whose column is active strongly, the
/// folders on the path before it faintly.
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

/// A column's intent: Left and Right move between columns, activating the folder whose column
/// is open goes to that column, a click on empty space closes deeper columns; everything else
/// goes to the host.
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
        Intent::Activate { id, .. } if reopens(sh, s, depth, id) => {
            let p = (sh.parts_mut)(s);
            p.state.set_active(depth + 1, p.look, tx);
        }
        Intent::Other(any) if any.downcast_ref::<BackgroundClick>().is_some() => {
            background_click(sh, s, depth, tx);
        }
        intent => {
            if let Intent::Activate { id, .. } = &intent {
                let p = (sh.parts_mut)(s);
                let folder = p
                    .sources
                    .source(depth)
                    .index_of(*id)
                    .is_some_and(|i| p.sources.source(depth).entry(i).kind.is_folder_like());
                if folder {
                    p.state.opened = Some((depth, *id));
                }
            }
            sh.emit(s, ColumnsIntent::Collection { depth, intent }, tx);
        }
    }
}

/// Whether activating `id` in column `depth` goes to its open column instead.
fn reopens<S>(sh: &Shared<S>, s: &S, depth: usize, id: EntryId) -> bool {
    let p = (sh.parts)(s);
    p.sources.open_child(depth) == Some(id) && depth + 1 < p.state.columns.len()
}

/// Right at the column's edge: into the open child column (its cursor on its first entry if it
/// has none), else open the folder under the cursor.
fn enter_right<S: 'static>(sh: &Rc<Shared<S>>, s: &mut S, depth: usize, tx: &mut Tx) {
    let p = (sh.parts_mut)(s);
    if depth + 1 < p.state.columns.len() && depth + 1 < p.sources.len() {
        let child = depth + 1;
        let listing = if p.sources.loading(child) {
            crate::collection::selection::Listing::Loading
        } else {
            crate::collection::selection::Listing::Complete
        };
        p.state.set_active_quiet(child);
        p.state
            .place_first(child, p.sources.source(child), listing, tx);
        p.state.reveal(p.look, tx);
        return;
    }
    let source = p.sources.source(depth);
    let folder = p.state.columns[depth]
        .collection
        .selection
        .cursor_index(source)
        .filter(|&i| source.entry(i).kind.is_folder_like())
        .map(|i| source.id_at(i));
    if let Some(id) = folder {
        sh.emit(s, ColumnsIntent::EnterFolder { depth, id }, tx);
    }
}

/// Left at the column's edge: into the parent column (its cursor on its open folder if it has
/// none), or past the root to the host.
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
}

/// A plain click on column `depth`'s empty space: in a column that wasn't active, the deeper
/// columns close, it becomes active (and is revealed) and its first visible entry is selected;
/// in the active one, every column's selection clears and the deeper columns close. Either
/// way the column keeps its scroll position.
fn background_click<S: 'static>(sh: &Rc<Shared<S>>, s: &mut S, depth: usize, tx: &mut Tx) {
    let p = (sh.parts_mut)(s);
    let was_active = p.state.press_was_active;
    let more = depth + 1 < p.state.columns.len();
    if was_active {
        p.state.clear_all(p.sources);
    } else {
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

/// Every press a column takes, before its controllers: it targets the column, makes it the
/// active one (except the second press of a click that opened the next column), and cancels a
/// slow-click rename pending in another column (Strata's `cancel_click_rename`).
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
    let p = (sh.parts_mut)(s);
    for (i, c) in p.state.columns.iter_mut().enumerate() {
        if i != depth
            && let Some(ctl) = c.collection.controllers.pointer.as_mut()
        {
            ctl.reset();
        }
    }
    p.state.press_was_active = p.state.active == depth;
    p.state.targeted = Some(depth);
    if input.button == PointerButton::Middle {
        return;
    }
    let hit = input
        .hit
        .index
        .filter(|&i| i < p.sources.source(depth).len())
        .map(|i| p.sources.source(depth).id_at(i));
    let second = input.clicks >= 2
        && hit.is_some()
        && p.state.opened == hit.map(|id| (depth, id))
        && p.state.active == depth + 1;
    if !second && p.state.active != depth {
        p.state.set_active_quiet(depth);
    }
}

/// The strip's `on_key`: the key goes to the active column; then Escape that nothing took
/// closes the deepest column.
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
    if collection(sh, s, serial).key(s, event, tx) {
        let p = (sh.parts_mut)(s);
        p.state.reveal(p.look, tx);
        return true;
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
    for c in &mut p.state.columns {
        if let Some(ctl) = c.collection.controllers.pointer.as_mut() {
            ctl.reset();
        }
    }
    p.state.clear_all(p.sources);
}

/// An accepted drag over a column's zones or the strip's blank space (`Some`), or gone from
/// them (`None`). True when the view's drop highlight changed.
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

/// The strip's blank space as a drop zone. A drop there does nothing (Strata's has no
/// target), but the zone follows every drag over it, accepted or refused, so the strip
/// autoscrolls and washes the column under the drag there too. A refusal keeps the last
/// point, and only leaving clears it.
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

/// Where the strip and its columns are now, in window points.
pub(super) fn geometry(st: &ColumnsState, look: &Look) -> StripGeometry {
    let feed = st.strip_feed.get();
    let offset = feed.offset[0];
    let x0 = st.strip.x - offset;
    let columns = st
        .spans(look)
        .into_iter()
        .zip(&st.columns)
        .map(|((l, r), c)| {
            let pos = c.feed.get();
            ColumnGeometry {
                left: x0 + l,
                right: x0 + r,
                listing: c.collection.viewport(),
                offset: pos.offset[1],
                max: pos.max[1],
            }
        })
        .collect();
    StripGeometry {
        strip: st.strip,
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
        None => {}
    }
    if let Some((depth, scroll)) = step.listing
        && let Some(path) = p.state.listing_path(depth)
    {
        match scroll {
            Scroll::Glide(y) => tx.scroll_to_with(path, 0.0, y, super::drag_scroll::GLIDE),
            Scroll::Instant(y) => tx.scroll_to_instant(path, 0.0, y),
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
    let mut content = row().key("columns").h_full();
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
        // Blank space, not a control: a press there clears the selection, as an empty area
        // does, so assistive technology hears no button for it.
        .a11y_hidden()
        .on_press(move |s: &mut S| blank_press(&blank, s));
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
