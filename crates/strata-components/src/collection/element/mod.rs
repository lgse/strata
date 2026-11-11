//! The virtual-row collection element with row slots (C1e): the Rune element under the list,
//! the icons grid and each Columns column.
//!
//! [`Collection`] draws an [`EntrySource`] as virtual rows inside a scroll viewport, from the
//! [`CollectionState`] the host keeps (its [`Selection`] among it). Rows host slots the host or
//! the view fills ([`Slot`]): an icon or thumbnail (C25), a label or rename editor (C5),
//! trailing cells (C4, [`ContentSlot`]), a drop highlight on folder rows and a drag source
//! (W1's [`Dnd`] binder). Drags out start from the pointer controller's
//! [`Intent::BeginDrag`], not from rows (see Decisions). Pointer, keyboard and marquee input
//! goes through one seam
//! ([`seam`]) to the controllers C1b, C1c and C1d install; with none, a press selects. A
//! grouped collection (C4's file-type groups, [`CollectionState::set_groups`]) puts a header
//! line above each group, and keeps the top group's header stuck at the top of the viewport.
//!
//! ```ignore
//! struct App { listing: ListSource, coll: CollectionState, look: Look, version: u64 }
//!
//! Collection::new(
//!     "files",
//!     |s: &App| (&s.listing, &s.coll),
//!     |s: &mut App| (&s.listing, &mut s.coll),
//!     |s: &App| &s.look,
//!     |s: &App| s.version,
//! )
//! .name("Files")
//! .on_cursor(|s: &mut App, id, _| s.preview(id))
//! .build(s)
//! ```
//!
//! # The listing version (required)
//!
//! The host passes `listing_version`, a value it changes whenever the source's order or
//! entries change while its length stays the same: a re-sort, a refresh that replaces entries,
//! a rename that changes an id. Rune then skips re-reading every row's key on a commit, which
//! is what keeps a 100,000-entry list's commits independent of its length. A change of length
//! is always noticed (at a cost that grows with the list: Rune issue #203). A stale version
//! leaves rows keyed by their old ids; the rows still draw the right entries, since handlers
//! hit-test by index and resolve ids at the event.
//!
//! # Coordinates
//!
//! Every handler sits on the container. A pointer's content point is its window point minus
//! the viewport's window origin plus the scroll offset ([`CollectionState::content_point`]),
//! as `contract::geometry` defines it; [`CollectionState::content_to_window`] goes back, for
//! popup anchors (C21).
//!
//! # Performance
//!
//! Rows are built by index (`El::virtual_rows`), only for the mounted window, and each row is
//! a `memo` keyed by its entry id, versioned by its entry, its [`RowState`], the look and its
//! slots' versions. Scrolling commits no view; hover is drawn by Rune; the viewport's layout
//! and visible rows are recorded as bookkeeping. A selection change rebuilds only the rows
//! whose state changed. The perf test (`tests/perf.rs`) holds this to rune-kit's Table.
//!
//! # Decisions
//!
//! - **Focus.** Rune 2a44156 has no focus-change notification and no focus-within styles
//!   (Rune draft U-C1e-4; anomalyco/rune#106). The element never moves focus itself: the
//!   position-restore bullets on focus after Back, Forward and Up, and the first-visit
//!   refocus, are the host's, which can focus the collection by its key with `Tx::focus`
//!   (Rune 22aa8cde). A selection looks
//!   the same whether or not the collection has focus (Strata dims it when unfocused), and the
//!   cursor outline follows keyboard mode (the last input that moved the cursor was a key)
//!   instead of keyboard focus.
//! - **Collections keyed by their host are no focus stops** ([`Collection::keys_by_host`],
//!   C2). Their container isn't focusable, has no key handler and names no active descendant;
//!   a press in one focuses the nearest ancestor with a key handler, which routes keys to the
//!   collection it picks ([`Collection::key`]) and names that collection's cursor row
//!   ([`Collection::cursor_path`]). Columns is one Tab stop this way, as in Strata.
//! - **Selection fill.** Selected rows use rune-kit's table selection colour
//!   (`TableStyle::from_kit(kit).selected`), hover Kit's `c.hover`, the cursor outline
//!   `c.border_focus` and a drop target or Enter target `c.text_accent`.
//! - **Rows move at once.** Inserted and removed rows land without growth or exit motion
//!   (`El::instant_changes`), so what is drawn always matches the index geometry presses and
//!   the marquee hit-test against. Removal effects (C23's dissolve, a slide) are the host's.
//! - **No drag source on rows.** Rune counts `El::drag_out` as a primary press handler
//!   (`Handlers::has_primary`, `el/handler_set.rs` at the pinned rev), so a row with one becomes
//!   the press target (`runtime/input/hit.rs`, `primary_target`) and the container's `on_drag`
//!   never hears presses on it: no selection, no click intent, no drag threshold. So rows get
//!   no `drag_out`, and the element takes no `DragSource`. A drag starts when the pointer
//!   controller (C1b) emits [`Intent::BeginDrag`] past its threshold, with the window point;
//!   the host binds that to the system drag (X2, through the platform shim), with the
//!   container's phases from [`Collection::on_drag_phase`] and the zones' action from
//!   [`Collection::drop_action`].
//! - **Selecting from assistive technology.** Options have no press handler of their own
//!   (all input goes to the container, above), so an assistive client's default action on an
//!   option (AT-SPI's "click") does nothing at the pinned rev. Rune has no way to route an
//!   option's action to its container with the target's key (Rune draft U-C1e-8). Until it
//!   does, a screen-reader user selects with the keyboard, through the container's active
//!   descendant (C1c).
//! - **No cursor reveal on a pointer press.** A key or a tick that moves the cursor scrolls it
//!   into view; a press moves it only onto what the viewport already shows.
//! - **Reveals can be kept to the collection's own viewport**
//!   ([`CollectionState::set_own_viewport_reveals`], C2). A reveal request scrolls a sideways
//!   viewport around the collection too, at once (Rune draft U-C2-4), so a collection inside a
//!   strip that glides to it on its own (C2's columns) reveals by an instant offset on its own
//!   viewport instead, worked out from the geometry. Off by default: other hosts' reveals are
//!   Rune's, as before.
//! - **Sticky group headers are an overlay** ([`CollectionState::set_groups`], C1f). Rune
//!   sticks an element only inside its own virtual row, which would put a whole group (up to
//!   every entry) in one row, so each group's header is a virtual line of its own and the
//!   header of the group at the top is a copy drawn over the viewport's top line (Rune draft
//!   [U-C1f-4]). The copy is pushed out by the next header through a live value, with no view
//!   commit; its label is built, so a scroll into another group builds the view once (none
//!   per frame). A live label would save that build, but `live_text` anywhere in the tree
//!   makes Rune build the view on every shift of the virtual window (Rune draft [U-C1f-6]).
//!   The copy takes no input and is hidden from assistive technology.
//! - **A press on a header is on empty space** (owner ruling). A press on a header line or
//!   the stuck header is on no entry: a plain one clears the selection, a drag starts a band,
//!   a secondary press opens the background menu. Strata's GTK headers decide nothing of their
//!   own. A drop there goes to the folder shown.
//! - **Deviation: header lines are the row height** (owner ruling). Rune's virtual list has one
//!   height estimate, so a header of another height would put rows where the index geometry
//!   doesn't. Strata's `.type-group-heading` is about 2 pt taller than a row; this one is a
//!   row tall until Rune draft [U-C1f-1].
//! - **Deviation: a page leaves a row for the stuck header** (owner ruling). In a grouped
//!   collection [`GroupedRows`]'s page step is the rows that fit below the stuck header, minus
//!   one, so W1d's row of overlap stays in view. Strata's pages, with no stuck header, count
//!   the whole viewport.
//! - **A primary press on the header part's blank space is on empty space** (owner ruling,
//!   C4). It reaches the controllers as a press on no entry ([`Region::Gutter`]) at the viewport's top
//!   edge, so a drag from it draws a band into the rows, as Strata's List heading strip does
//!   (`browser/selection/marquee`). Parts of the header with their own press or drag
//!   handlers (C4's headings) keep their presses, as Rune gives a press to its innermost
//!   handler. C2 and C3 get the same rule for any header they add.
//! - **Rest rows carry a hover handler** ([`Collection::rest`], C22, owner ruling). Rows whose
//!   rest hook gives them delays get Rune's `hover_delays` and `on_hover_change`, the only
//!   handler a row has besides a drop zone. It isn't a press handler, so presses still reach the
//!   container. A collection without the hook builds the same rows as before.
//! - **A heading per group, nothing per row** (owner ruling). A header line is a heading named
//!   by its group; rows don't repeat their group's name, as in GTK.
//!
//! # Rune drafts (not filed)
//!
//! Issues C1f found in Rune at the pinned rev, written up for the owner to file. Each repro
//! runs headless (`rune::Runtime`).
//!
//! - **U-C1f-1: exact heights for `virtual_rows`.** A virtual list has one height estimate, and
//!   rows it hasn't laid out take it (`el/virtual_list.rs`, `height_of`), so rows of two
//!   heights (a section header and its items) put rows where an app's index geometry doesn't.
//!   Repro: `virtual_rows` of 1,000 rows alternating 26 and 30 pt with an estimate of 26,
//!   `scroll_to_instant` to 20,000 pt, then compare a mounted row's drawn y with the sum of
//!   the heights above it: they differ by the unvisited rows' 4 pt each. Ask: a height per
//!   index (or "rows are exactly this tall") that layout takes instead of the estimate.
//! - **U-C1f-2: a virtual row outside the set.** Every mounted row of a virtual list gets its
//!   list index as its position in the set (`a11y/tree.rs`, `a11y_index_in_list`), so a
//!   section header among the items reads as an item. Repro: `virtual_rows` whose rows 0 and
//!   11 are headings and the rest options with `position_in_set`; `--tree` gives the heading
//!   at row 11 `"position": 12`. Ask: a way for a row to take no position (and not count).
//! - **U-C1f-3: group membership across flat virtual rows.** A section's header can't own the
//!   items under it when both are rows of one virtual list, as `aria-owns` or AccessKit's
//!   `member_of` would let it, so a screen reader moving through options never hears the
//!   group it enters. Repro: as U-C1f-2; no builder relates row 12 to the heading at row 11.
//!   Ask: an `El` builder naming the element (by key) a row belongs to.
//! - **U-C1f-4: section headers that stick across virtual rows.** A sticky element sticks only
//!   within its parent, and a virtual list's rows are the parents, so a header sticks only if
//!   its whole section is one row (README, "Sticky headers"), which defeats virtualization for
//!   a section of thousands. Repro: `virtual_rows` of 10,000 one-line rows in sections of
//!   1,000, each section's first row holding a `.sticky_top(0.0)` header; scroll into a
//!   section and the header scrolls away with its row. Ask: sections of a virtual list whose
//!   header sticks while any of the section's rows show.
//! - **U-C1f-5: `on_visible_rows` a frame late after a rekeying commit.** After a commit that
//!   changes the rows (a grid reflowed to a new width), the handler hears the new range only
//!   on the next frame, so for one frame the app's visible range is the old one. Repro: a
//!   grid of 4,000 cells in lines keyed by their first cell, scrolled to line 76 at 5 a line;
//!   narrow it to 2 a line: on the first frame the cells drawn are 380..413 and the last
//!   report still 380..465; it updates on the second.
//! - **U-C1f-6: `live_text` turns off the virtual-window fast path.** A commit that only
//!   shifts a virtual window skips `view` unless a node has live text
//!   (`runtime/commit/remount.rs`, `virtual_only`), so one `live_text` label anywhere makes
//!   every window shift of a scroll run the view. Repro: `virtual_rows` of 1,000 rows beside a
//!   `text("x").live_text(&label)`, scroll 120 frames of 7 pt: the view runs 7 times, and none
//!   without the `live_text`. Ask: let live text stay with the remounted tree.
//!
//! [U-C1f-1]: self#rune-drafts-not-filed
//! [U-C1f-4]: self#rune-drafts-not-filed
//! [U-C1f-6]: self#rune-drafts-not-filed
//!
//! Work packages: C1e, `docs/work/phase-2-composites/C1e-collection-element.md`; C1f (group
//! header lines, the press hook and C1e's round-3 fixes) has no document of its own.

#![warn(missing_docs)]

mod a11y;
mod keys;
mod lines;
mod route;
mod row;
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

/// How long the pointer must stay out of a collection with the rest hook before its leave
/// counts: longer than a leave and an enter X11 sends back to back (one batch of events,
/// well under a 60 Hz frame even on a loaded machine), shorter than a person leaving and
/// coming back (the rest delay itself is 500 ms).
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

/// What [`Collection`] was built with, shared by its rows and handlers.
pub(crate) struct Config<S> {
    key: Rc<str>,
    get: GetFn<S>,
    get_mut: GetMutFn<S>,
    look: LookFn<S>,
    version: ReadFn<S, u64>,
    settings: Rc<Settings>,
    name: Option<String>,
    roles: Roles,
    row_height: RowHeightFn<S>,
    lines: Rc<dyn Lines>,
    group_style: GroupStyleFn,
    dnd: Dnd<S>,
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
    overlay: Option<PartFn<S>>,
    empty: Option<PartFn<S>>,
    /// Every part [`Collection::edge`] added, in order.
    edge: Vec<PartFn<S>>,
    on_intent: Option<IntentFn<S>>,
    on_press: Option<PressFn<S>>,
    on_cursor: Option<CursorFn<S>>,
    on_selection: Option<SelectionFn<S>>,
    on_visible: Option<VisibleFn<S>>,
    announce: Option<AnnounceFn<S>>,
    /// Every handler [`Collection::on_scroll`] added, in order.
    on_scroll: Vec<ScrollFn>,
    keys_by_host: bool,
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
///
/// The host keeps the source and a [`CollectionState`] in its state and gives the element
/// accessors to both, as one pair, so a handler can read the source while it changes the
/// state. See the [module docs](self) for the required `listing_version`.
///
/// Used by: C2 (one per column), C3, C4, C14b (read-only), hosts.
pub struct Collection<S> {
    cfg: Config<S>,
}

impl<S: 'static> Collection<S> {
    /// A collection keyed `key` (its rows' keys and scroll requests start there), over the
    /// source and state `get` and `get_mut` return, drawn with the look `look` returns.
    /// `listing_version` changes whenever the source's order or entries change at the same
    /// length (see the [module docs](self#the-listing-version-required)).
    ///
    /// The accessors are closures, so one host can build several collections over its own
    /// collections, each capturing which one (C2's columns):
    /// `move |s: &App| (&s.columns[i].listing, &s.columns[i].state)`.
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
                roles: Roles::LISTBOX,
                row_height: Rc::new(|_, l| l.row.list),
                lines: Rc::new(OnePerLine),
                group_style: Rc::new(GroupStyle::from_look),
                dnd: Dnd::rune(),
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
                overlay: None,
                empty: None,
                edge: Vec::new(),
                on_intent: None,
                on_press: None,
                on_cursor: None,
                on_selection: None,
                on_visible: None,
                announce: None,
                on_scroll: Vec::new(),
                keys_by_host: false,
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

    /// The roles: [`Roles::LISTBOX`] (the default), [`Roles::GRID_ROWS`] or
    /// [`Roles::GRID_CELLS`].
    pub fn roles(mut self, roles: Roles) -> Self {
        self.cfg.roles = roles;
        self
    }

    /// The row height, read from the look: `row.list` by default, `row.columns` for a Columns
    /// column, `slots::list::row_height` for ListTable. Every row draws at exactly this
    /// height, so the geometry matches what is drawn.
    pub fn row_height(mut self, row_height: impl Fn(&S, &Look) -> f32 + 'static) -> Self {
        self.cfg.row_height = Rc::new(row_height);
        self
    }

    /// How entries pack into lines (C3's grid); [`OnePerLine`] by default.
    pub fn lines(mut self, lines: impl Lines + 'static) -> Self {
        self.cfg.lines = Rc::new(lines);
        self
    }

    /// How header lines and the stuck header draw: [`GroupStyle::from_look`] by default.
    pub fn group_style(mut self, style: impl Fn(&Look) -> GroupStyle + 'static) -> Self {
        self.cfg.group_style = Rc::new(style);
        self
    }

    /// Adds a settings value for the controllers (the double-click interval, the click mode,
    /// the auto-scroll speed), which each reads by its type with [`Ctx::settings`]. Call it
    /// once per type; a second value of a type replaces the first. Taken as the view builds,
    /// so pass the current values each time.
    pub fn settings<T: Any>(mut self, value: T) -> Self {
        Rc::make_mut(&mut self.cfg.settings).insert(value);
        self
    }

    /// The drag-and-drop binder: Rune's own by default.
    pub fn dnd(mut self, dnd: Dnd<S>) -> Self {
        self.cfg.dnd = dnd;
        self
    }

    /// Read-only: no drops and no rename, as the archive preview needs (C14b). Controllers
    /// see it as [`Ctx::read_only`].
    pub fn read_only(mut self) -> Self {
        self.cfg.read_only = true;
        self
    }

    /// Items draw none of the row's own chrome: no selection fill, hover, outline, corner
    /// radius or side inset, only the host's opacity marks (cut, dragging). The content slot's
    /// element is the item itself (keyed, named and sized by the element), and draws the
    /// item's state from [`RowCx::state`] and [`RowCx::hover_live`], as C3's tiles light their
    /// frame and caption, not the whole cell.
    pub fn bare_items(mut self) -> Self {
        self.cfg.bare_items = true;
        self
    }

    /// Takes drops: on a folder-like row (`Some(id)`) or the background (`None`, the folder
    /// shown). Folder rows draw the drop highlight while a drag hovers them.
    ///
    /// The zones reach the binder ([`Collection::dnd`]) by key (W1's `Dnd::drop_zone_keyed`):
    /// the background by the collection's key, and a folder row by [`row_key`] with the
    /// collection's key as its scope, since row keys are unique only inside one collection.
    pub fn on_drop(
        mut self,
        f: impl Fn(&mut S, Option<EntryId>, Dropped, &mut Tx) + 'static,
    ) -> Self {
        self.cfg.on_drop = Some(Rc::new(f));
        self
    }

    /// The icon or thumbnail part (C25). By default, the entry's catalog icon.
    pub fn icon(mut self, slot: Slot<S>) -> Self {
        self.cfg.icon = Some(slot);
        self
    }

    /// The label part (matched text, say). By default, the name in `ty.ui_compact`.
    pub fn label(mut self, slot: Slot<S>) -> Self {
        self.cfg.label = Some(slot);
        self
    }

    /// The row's content, from its icon and label (C4's cells). By default, the icon then
    /// the label.
    pub fn content(mut self, slot: ContentSlot<S>) -> Self {
        self.cfg.content = Some(slot);
        self
    }

    /// Each row's accessible description, read after its name. By default, none.
    pub fn describe(mut self, describe: Describe<S>) -> Self {
        self.cfg.describe = Some(describe);
        self
    }

    /// The rename editor (C5): drawn in the label's place on the row `renaming` names. A
    /// read-only collection draws none.
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

    /// Which part of entry `index`'s row a point (in the row's own points) is on, for a row
    /// whose content isn't the default icon and label (C3's caption, C4's cells).
    pub fn regions(mut self, f: impl Fn(&S, usize, Point) -> Region + 'static) -> Self {
        self.cfg.regions = Some(Rc::new(f));
        self
    }

    /// A part above the viewport (C4's column headings). A primary press on its blank space
    /// (where no part of it takes presses itself) is on empty space at the viewport's top edge:
    /// a plain one clears the selection and a drag draws a band from there, as Strata's List
    /// heading strip does. Other buttons there aren't the collection's. Its parts with press or
    /// drag handlers keep their presses.
    pub fn header(mut self, f: impl Fn(&S) -> Option<El> + 'static) -> Self {
        self.cfg.header = Some(Rc::new(f));
        self
    }

    /// A primary press on the header part's blank space reaches the controllers only once it
    /// travels more than `threshold` points (along either axis; pass the pointer controller's
    /// drag threshold): a drag from there then bands as [`Collection::header`] says, and a
    /// click there changes nothing in the collection. [`Collection::on_press`] still hears the
    /// press. Columns (C2): a click on a column's title focuses it and keeps its selection,
    /// while a drag from its header's blank space starts a band, as in Strata.
    pub fn header_defers_press(mut self, threshold: f32) -> Self {
        self.cfg.header_defer = Some(threshold);
        self
    }

    /// A part drawn over the viewport, pinned to it below any header, in its coordinates
    /// (C1d's band). It takes no input: presses go through it to the rows.
    pub fn overlay(mut self, f: impl Fn(&S) -> Option<El> + 'static) -> Self {
        self.cfg.overlay = Some(Rc::new(f));
        self
    }

    /// What an empty collection shows, pinned over the viewport below any header.
    pub fn empty(mut self, f: impl Fn(&S) -> Option<El> + 'static) -> Self {
        self.cfg.empty = Some(Rc::new(f));
        self
    }

    /// A part over the viewport, below any header, in its coordinates, that takes input: C24's
    /// scroll bar along its edge. It hangs from a zero-size anchor at the viewport's top-left
    /// (`scroll::anchor`), so only its own parts cover the rows. Drawn above the overlay. A press on an element of
    /// it with a press or drag handler is that element's, not the collection's (Rune gives a
    /// press to its innermost handler); elsewhere presses reach the rows.
    ///
    /// Each call adds a part (a view's bar, then a host's), drawn in the order added; none
    /// replaces another.
    pub fn edge(mut self, f: impl Fn(&S) -> Option<El> + 'static) -> Self {
        self.cfg.edge.push(Rc::new(f));
        self
    }

    /// Hears the viewport's scroll position on every frame it changes (Rune's
    /// `El::on_scroll_offset`), with no view commit: C24's scroll bar and autoscroll follow it
    /// through live values. It reads no state.
    ///
    /// Each call adds a handler, heard in the order added: a view that follows its own scroll
    /// (C3's and C4's bars) never drops a host's, nor the host's the view's.
    pub fn on_scroll(mut self, f: impl Fn(ScrollPos) + 'static) -> Self {
        self.cfg.on_scroll.push(Rc::new(f));
        self
    }

    /// The collection's key, the scope of its key paths ([`viewport_path`] of it is its
    /// viewport's).
    pub fn scope(&self) -> &str {
        &self.cfg.key
    }

    /// Keys reach this collection from the host, not from Rune's focus: the collection is no
    /// focus stop (its container isn't focusable and has no key handler), and names no active
    /// descendant. An ancestor that is the focus stop routes each key to it with
    /// [`Collection::key`] and names the cursor's row as its own active descendant
    /// ([`Collection::cursor_path`]). A press in it focuses that ancestor, the nearest one with
    /// a key handler. Columns (C2) builds every column this way, so the whole strip is one Tab
    /// stop, as in Strata, and the strip sends each key to the column that holds the cursor.
    pub fn keys_by_host(mut self) -> Self {
        self.cfg.keys_by_host = true;
        self
    }

    /// Hears where a drag the collection's drop zones accept is, in window points, while it is
    /// over them (`Some`, on reaching a zone and on every move), and when it leaves them or a
    /// zone refuses it (`None`). Set, it replaces the element's own edge scroll during a drag
    /// (a row's height a move near the viewport's top or bottom): the host scrolls instead.
    /// Columns (C2) follows the drag across its whole strip with it, since Rune tells only the
    /// innermost zone under a drag that it moved (Rune draft U-C2-1). Return true when it
    /// changed what the view reads. Needs [`Collection::on_drop`], which makes the zones.
    pub fn on_drag_over(
        mut self,
        f: impl Fn(&mut S, Option<Point>, &mut Tx) -> bool + 'static,
    ) -> Self {
        self.cfg.on_drag_over = Some(Rc::new(f));
        self
    }

    /// Hears each phase of the container's drag handler before the controllers do, with the
    /// collection's key: for a host that starts a system drag from [`Intent::BeginDrag`] (X2,
    /// through P1's `Dnd::start_drag`, which needs the phase and the key of the element whose
    /// `on_drag` it is, and `Dnd::owns` on every phase). True takes the phase: a taken move is
    /// ignored, and a taken release ends the press as a cancelled one (no click, no rename),
    /// as the system drag owns it. A taken press is ignored too.
    ///
    /// The container keeps its key and its place for as long as the collection is built, so
    /// a drag started from it can rely on its source staying mounted (P1's rule), however the
    /// rows scroll or re-key.
    pub fn on_drag_phase(
        mut self,
        f: impl Fn(&mut S, &rune::Drag, &str, &mut Tx) -> bool + 'static,
    ) -> Self {
        self.cfg.on_drag_phase = Some(Rc::new(f));
        self
    }

    /// Says which action a drag over a folder row (`Some(id)`) or the background (`None`)
    /// would take, or `None` to refuse it, for a binder that negotiates while the drag hovers
    /// (W1's `DropZone::action`): the host's copy-or-move rule with its volume lookup, and its
    /// guard against dropping items into their own folder. Without it, W1's rule with an
    /// unknown volume. A drag over the stuck group header is asked about the row it covers
    /// (its drop then goes to the background, as before). Needs [`Collection::on_drop`].
    pub fn drop_action(
        mut self,
        f: impl Fn(&S, Option<EntryId>, &DropOffer, Mods) -> Option<DropAction> + 'static,
    ) -> Self {
        self.cfg.drop_action = Some(Rc::new(f));
        self
    }

    /// Hears wheel and trackpad deltas over the viewport before it scrolls (Rune's
    /// `El::on_wheel` on the viewport itself): true takes the delta. An `on_wheel` on an
    /// ancestor of the collection never hears a wheel over its rows, since Rune offers a wheel
    /// only up to the nearest scroll view (Rune draft U-C3-3), so a view that claims a modified
    /// wheel (C3's Ctrl+Shift+wheel) does it here.
    pub fn on_wheel(mut self, f: impl Fn(&mut S, WheelEvent, &mut Tx) -> bool + 'static) -> Self {
        self.cfg.on_wheel = Some(Rc::new(f));
        self
    }

    /// Rows that report the pointer resting on them, with Rune's hover delays, and a part
    /// pinned in them: C22's folder peek. See [`Rest`].
    pub fn rest(mut self, rest: Rest<S>) -> Self {
        self.cfg.rest = Some(rest);
        self
    }

    /// Hears the controllers' intents (open, rename, drag, context menu).
    pub fn on_intent(mut self, f: impl Fn(&mut S, Intent, &mut Tx) + 'static) -> Self {
        self.cfg.on_intent = Some(Rc::new(f));
        self
    }

    /// Hears every press the collection takes, before its controllers, whether or not anything
    /// changes: primary and middle presses through its drag handler, secondary ones through
    /// its menu press, inside the viewport (a header line and the stuck header included, not
    /// a part above it). Columns (C2) cancels a slow-click rename pending in another column
    /// here, as Strata's `ViewState::cancel_click_rename` does.
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

    /// Hears which entries the viewport shows, when that changes (C25's priority). Return
    /// true only if the view reads what it changed: scrolling then commits.
    pub fn on_visible(mut self, f: impl Fn(&mut S, Range<usize>) -> bool + 'static) -> Self {
        self.cfg.on_visible = Some(Rc::new(f));
        self
    }

    /// The status line's wording for a selection change: [`default_announcement`] by
    /// default. The line changes only when an operation changed the selection
    /// ([`CollectionState::note_selection`]), so a listing change alone announces nothing.
    pub fn announce(mut self, f: impl Fn(&S, &Announcement) -> String + 'static) -> Self {
        self.cfg.announce = Some(Rc::new(f));
        self
    }

    /// Runs the controllers' timeouts and queued commands. Call it from the host's poll on
    /// every frame while a controller asks for wake-ups ([`Effect::WakeAfter`], an
    /// auto-scroll), with the same builder the view uses; true when state changed. A tick
    /// with nothing to do costs a [`Ctx`] and no commit.
    ///
    /// A command that starts outside the collection (a menu item, a verb typed in a footer
    /// prompt) reaches its controller this way: the controller queues it in a handle the
    /// host shares, and the next tick runs it with a [`Ctx`] (see [`KeyController::tick`]).
    pub fn tick(self, s: &mut S, tx: &mut Tx) -> bool {
        let cfg = Rc::new(self.cfg);
        route::tick(&cfg, s, tx)
    }

    /// Handles a key as the collection's own key handler would: the keyboard controller hears
    /// it, through the same routing a focused collection's keys take. For a collection built
    /// with [`Collection::keys_by_host`], from the focus stop's `on_key`, with the key's own
    /// transaction (so layout-aware chords still match). True when a controller took the key;
    /// the caller then says so to Rune.
    pub fn key(self, s: &mut S, event: &KeyEvent, tx: &mut Tx) -> bool {
        let cfg = Rc::new(self.cfg);
        route::on_key(&cfg, s, event, tx)
    }

    /// The key path of the cursor's row (or cell), under this collection's key, as its
    /// container names it its active descendant: for the focus stop of a collection built with
    /// [`Collection::keys_by_host`]. `None` with no cursor. The path starts at the collection's
    /// key, so the focus stop names it from above (a key path's leading keys are the nearest
    /// keyed ancestors of the rest).
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

    /// Works the entries per line out again from the lines' rule as it stands now: call it
    /// from the handler that changed what the rule reads at the same width (C3's tile size),
    /// with this builder built from the new state. When the count changed, the cursor scrolls
    /// into view on `tx`, as a width reflow does. True when it changed. (The build works the
    /// count out too, so it is right without this; only the reveal needs the handler's `tx`.)
    pub fn reflow(self, s: &mut S, tx: &mut Tx) -> bool {
        let (src, st) = (self.cfg.get_mut)(s);
        if st.viewport.w <= 0.0 {
            return false;
        }
        let per = self.cfg.lines.per_line(st.viewport.w);
        st.reflow_to(&self.cfg.key, src, per, tx)
    }

    /// The geometry of `s`'s entries as the element lays them out now, in content
    /// coordinates: for an anchor out of the element (C21's keyboard menu at the cursor,
    /// through [`CollectionState::content_to_window`]).
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
        // Once the viewport has a width, the count follows the lines' rule as it stands now (a
        // tile size changed at the same width included), and the state records it so sync,
        // reveal and the visible range agree with what this builds.
        let per = if st.viewport.w > 0.0 {
            cfg.lines.per_line(st.viewport.w).max(1)
        } else {
            st.per_line.get().max(1)
        };
        st.per_line.set(per);
        let map = LineMap::new(per, Some(&st.groups), len);
        let lines = map.count(len);
        let row_height = cfg.row_height_of(s);
        // The group whose header this build sticks at the top (see `Stuck`): the one Rune last
        // reported at the top, else the one at the scroll offset.
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
            // Rows land at once: an animated gap would draw rows where the geometry doesn't
            // put them, and a press during it would hit the wrong entry.
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
            });
        if cfg.rest.is_some() {
            // The pointer leaving the collection forgets the row it was on, so coming back onto
            // that row is an arrival; but only once it has stayed out for REENTER. X11 sends a
            // leave and an enter back to back after a drop, with the pointer still: no
            // arrival, so a held row stays held.
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
                .on_key(move |s: &mut S, e, tx| route::on_key(&key_cfg, s, e, tx));
        }
        if let Some(name) = &cfg.name {
            root = root.label(name.clone());
        }
        // A vanished row (`RowMarks::vanished`) has left the tree: point at nothing rather than
        // at it.
        let vanished = |i: usize| {
            cfg.marks
                .as_ref()
                .is_some_and(|f| f(s, src.id_at(i)).vanished)
        };
        if !cfg.keys_by_host
            && let Some(index) = st.selection.cursor_index(src)
            && !vanished(index)
        {
            // A path under this collection, so a second collection showing the same folder
            // (another pane) can't answer for it.
            root = root.active_descendant(keys::entry_path(&cfg.key, src, index, per));
        }
        if let Some(header) = cfg.header.as_ref().and_then(|f| f(s)) {
            // Where it shows, as bookkeeping: a press on its blank space starts on empty space
            // (`route::on_drag`).
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
        // The viewport and what sits over it: the empty state and the overlay, pinned to the
        // viewport alone, below any header.
        let mut over = col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .clip_soft(0.0)
            .child(viewport);
        if len == 0
            && let Some(empty) = cfg.empty.as_ref().and_then(|f| f(s))
        {
            over = over.child(pinned(empty));
        }
        if let Some(part) = stuck_part {
            // Over the viewport's top line, clipped to it, so the next header pushes it up and
            // out; it takes no input (presses on it are on no entry: `route::hit`), and the
            // header line it copies is the one assistive technology reads.
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
            // In content coordinates: a layer at the content's origin that rides the scroll
            // offset in the same frame.
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
            // Hung from the viewport's top-left, not pinned over all of it: a layer the
            // viewport's size would be what a system drag's hit test (P1's) finds over the rows.
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

/// The stuck group header's push, which the viewport's scroll keeps up to date with no view
/// commit: up by as much as the next group's header line has come into the top line.
///
/// Its label is the group the build saw at the top (`shown`). A scroll into another group
/// builds the view again (`route::on_visible` says so), once per group crossed; until that
/// build lands the stuck header hides, and the new group's own header line shows at the top.
/// A live label would save that build, but a `live_text` anywhere in the tree makes Rune build
/// the view on every shift of the virtual window instead (Rune draft [U-C1f-6]).
///
/// [U-C1f-6]: crate::collection::element#rune-drafts-not-filed
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
    });
    // With the host's action, the background stops what it refuses (an item onto its own
    // folder) as the rows do, so it hears the refused drag's moves: autoscroll and C2's column
    // wash go on over it.
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

/// Records a hover on the zone `over` (a folder row or the background): an accepted drag
/// highlights it (the background only if no row is), a refused one doesn't and is noted as
/// refused (with whether `offer` offers files), and leaving clears both.
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

/// Tells the host's drag-over hook about a hover: where a drag reached a zone, accepted or
/// refused (the hook drives autoscroll and C2's column wash, which follow any drag, as
/// Strata's motion controller does), or that it left one. A refusal with no point known yet
/// keeps the hook's last point; the zone's next move gives the new one.
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
