//! Rows: what each one shows (its slots), the state it draws from, and the memo version that
//! keeps it built while none of that changes.

use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::Duration;

use rune::{El, Tx, el, icon, memo, row, text};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::collection as tokens;

use super::Config;
use super::keys::{ICON, LABEL, header_key, line_key, row_key};
use super::lines::{Group, GroupStyle, Line, LineMap};
use super::state::DropOver;
use crate::contract::{DropHover, DropZone, ElementKey, Entry, EntryId, Point, Rect};

/// Marks the host puts on rows, each with its own look.
///
/// Fields may be added: start from [`RowMarks::default`] and set the ones that apply.
///
/// Used by: hosts, C2 (the open path), C18 (the Enter target), C22 (the peek anchor), X3 (cut).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct RowMarks {
    /// Cut to the clipboard: drawn faded (`collection.cut.opacity`).
    pub cut: bool,
    /// Part of a drag in progress: drawn faded (`collection.drag.opacity`).
    pub dragging: bool,
    /// What Enter in the filter field opens: outlined in the accent.
    pub enter_target: bool,
    /// On the open path, in Columns: the folder whose child column is open.
    pub active_path: bool,
    /// The open path's folder in the column before the active one.
    pub active_parent: bool,
    /// The row a folder peek is open from.
    pub peek_anchor: bool,
    /// Gone from view while its entry stays listed: the row draws nothing and leaves the
    /// accessibility tree, a pointer press there is on empty space, and the cursor on it is no
    /// one's active descendant; but it keeps its slot. C23 hides the rows it dissolves this
    /// way, and the rows a restore flies onto, for the length of one effect.
    ///
    /// It stays an entry of the listing for the keyboard and the marquee (C1c, C1d): arrows,
    /// type-ahead and Select All can move the cursor onto it or select it, and a marquee over
    /// its slot selects it. A host that hides rows for longer keeps the cursor and selection
    /// off them itself, and one that takes the keys
    /// ([`keys_by_host`](super::Collection::keys_by_host)) its active descendant too.
    pub vanished: bool,
}

/// What a row draws from: its entry's place in the selection, and the host's marks.
///
/// Fields may be added.
///
/// Used by: slot builders (C3, C4, C5, C25).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct RowState {
    /// Selected (a provisional pick included).
    pub selected: bool,
    /// Selected as a load's automatic pick, not a fill.
    pub provisional: bool,
    /// The cursor's entry.
    pub cursor: bool,
    /// The cursor's entry, with the cursor outline showing (keyboard mode, active column).
    pub cursor_shown: bool,
    /// A drag hovering the collection would drop on this folder.
    pub drop_target: bool,
    /// Its rename editor is open.
    pub renaming: bool,
    /// The host's marks.
    pub marks: RowMarks,
}

/// What a slot builds a row part from.
///
/// Used by: slot builders (C3, C4, C5, C25).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RowCx<'a> {
    /// The entry's index in the source.
    pub index: usize,
    /// The entry.
    pub entry: Entry<'a>,
    /// What the row draws from.
    pub state: RowState,
    /// The look.
    pub look: &'a Look,
    /// The collection's key.
    pub scope: &'a str,
    /// Whether the item draws hover now: off after keyboard navigation until the pointer
    /// moves. The row's own hover follows it; a bare item's content slot follows it itself
    /// (`Collection::bare_items`).
    pub hover_live: bool,
}

type BuildFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> El>;
type ContentFn<S> = Rc<dyn Fn(&S, &RowCx<'_>, El, El) -> El>;
type VersionFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> u64>;
type DescribeFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> Option<String>>;

/// A row part the host or the view builds: an icon or thumbnail (C25), a label (matched
/// text), or a rename editor (C5).
///
/// Rows are memoized: a row is built again only when its entry, its [`RowState`], the look or
/// a slot's version changes. So a slot that reads anything from the state beyond its
/// [`RowCx`] (a thumbnail's readiness, a filter's matched ranges) is made with
/// [`Slot::new`] and a version that changes with what it reads; one that reads only its
/// `RowCx` is [`Slot::pure`]. Rows with a rename open are never memoized.
///
/// [`RowCx::index`] isn't part of the version: a row kept by its memo while entries are
/// inserted above it keeps the part it built at its old index. Draw nothing from the index
/// (zebra stripes, a position) without versioning it.
///
/// Parts a motion effect flies (C23) key themselves [`ICON`] and [`LABEL`], as the default
/// parts do.
///
/// Used by: C3, C4, C5, C25, hosts.
pub struct Slot<S> {
    pub(super) build: BuildFn<S>,
    pub(super) version: Option<VersionFn<S>>,
}

impl<S> Clone for Slot<S> {
    fn clone(&self) -> Self {
        Self {
            build: self.build.clone(),
            version: self.version.clone(),
        }
    }
}

impl<S: 'static> Slot<S> {
    /// A slot built by `build`, built again whenever `version` changes: the slot reads the
    /// state beyond its [`RowCx`], and `version` hashes what it reads.
    pub fn new(
        version: impl Fn(&S, &RowCx<'_>) -> u64 + 'static,
        build: impl Fn(&S, &RowCx<'_>) -> El + 'static,
    ) -> Self {
        Self {
            build: Rc::new(build),
            version: Some(Rc::new(version)),
        }
    }

    /// A slot built by `build` from its [`RowCx`] alone, so the row's own version covers it.
    pub fn pure(build: impl Fn(&S, &RowCx<'_>) -> El + 'static) -> Self {
        Self {
            build: Rc::new(build),
            version: None,
        }
    }
}

/// The row's content, laid out from its icon and label parts (C4 adds trailing cells).
///
/// Used by: C4.
pub struct ContentSlot<S> {
    pub(super) build: ContentFn<S>,
    pub(super) version: Option<VersionFn<S>>,
}

impl<S> Clone for ContentSlot<S> {
    fn clone(&self) -> Self {
        Self {
            build: self.build.clone(),
            version: self.version.clone(),
        }
    }
}

impl<S: 'static> ContentSlot<S> {
    /// Content laid out by `build` from the row's icon and label, built again whenever
    /// `version` changes (see [`Slot`] on versions).
    pub fn new(
        version: impl Fn(&S, &RowCx<'_>) -> u64 + 'static,
        build: impl Fn(&S, &RowCx<'_>, El, El) -> El + 'static,
    ) -> Self {
        Self {
            build: Rc::new(build),
            version: Some(Rc::new(version)),
        }
    }

    /// Content laid out from its [`RowCx`], icon and label alone.
    pub fn pure(build: impl Fn(&S, &RowCx<'_>, El, El) -> El + 'static) -> Self {
        Self {
            build: Rc::new(build),
            version: None,
        }
    }
}

/// A row's accessible description, read after its name (the entry's): what a slot shows that
/// assistive technology doesn't hear, such as the archive preview's file sizes (C14b). `None`
/// gives the row none. Versioned as a [`Slot`] is.
///
/// Used by: C14b.
pub struct Describe<S> {
    pub(super) build: DescribeFn<S>,
    pub(super) version: Option<VersionFn<S>>,
}

impl<S> Clone for Describe<S> {
    fn clone(&self) -> Self {
        Self {
            build: self.build.clone(),
            version: self.version.clone(),
        }
    }
}

impl<S: 'static> Describe<S> {
    /// A description from `build`, built again whenever `version` changes (see [`Slot`] on
    /// versions).
    pub fn new(
        version: impl Fn(&S, &RowCx<'_>) -> u64 + 'static,
        build: impl Fn(&S, &RowCx<'_>) -> Option<String> + 'static,
    ) -> Self {
        Self {
            build: Rc::new(build),
            version: Some(Rc::new(version)),
        }
    }

    /// A description from the row's [`RowCx`] alone.
    pub fn pure(build: impl Fn(&S, &RowCx<'_>) -> Option<String> + 'static) -> Self {
        Self {
            build: Rc::new(build),
            version: None,
        }
    }
}

type DelaysFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> Option<(Duration, Duration)>>;
type ReportFn<S> = Rc<dyn Fn(&mut S, RestReport, &mut Tx)>;
type PartFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> Option<El>>;
type ViewFn<S> = Rc<dyn Fn(&mut S, RestView) -> bool>;

/// Rows that report the pointer resting on them, through Rune's `El::hover_delays`: C22's
/// folder peek. Set with [`Collection::rest`](super::Collection::rest).
///
/// Each row `delays` gives `Some((enter, leave))` for reports `true` once the pointer has
/// rested on it for `enter`, and `false` once it has stayed off it for `leave`; a pass shorter
/// than the delay reports nothing, and a press cancels a wait (Rune). A row that leaves the
/// tree (scrolled out, a listing change, another view) reports its leave at once.
///
/// A `true` report counts only on the row the pointer last arrived on by moving over the
/// collection, as Strata's peek starts only from a motion enter: a row a scroll carries under
/// a parked pointer, the row a press of the collection's own ended on, a folder row a drag was
/// dropped on (taken or refused, a system drag included, which moves the pointer there
/// unseen), and the row a drag from elsewhere ended on (once the host calls
/// [`CollectionState::hold_rest`]) report nothing until the pointer leaves it (for another
/// row, or out of the collection) and comes back; moving inside the row doesn't count. None counts while row hover is off after keyboard
/// navigation ([`CollectionState::row_hover`]).
///
/// Rune counts a row under an overlay (a card, a menu, a scrim) as hovered too (Rune draft
/// U-C22-1): the hook's owner decides what an overlay hides.
///
/// [`CollectionState::hold_rest`]: super::CollectionState::hold_rest
///
/// The delays and the part are part of the row's memo version, so they may read anything; a
/// part that reads the state beyond its [`RowCx`] gives a `version` that changes with it.
///
/// [`CollectionState::row_hover`]: super::CollectionState::row_hover
///
/// Used by: C22.
pub struct Rest<S> {
    pub(super) delays: DelaysFn<S>,
    pub(super) report: ReportFn<S>,
    pub(super) part: Option<(VersionFn<S>, PartFn<S>)>,
    pub(super) view: Option<ViewFn<S>>,
}

impl<S> Clone for Rest<S> {
    fn clone(&self) -> Self {
        Self {
            delays: self.delays.clone(),
            report: self.report.clone(),
            part: self.part.clone(),
            view: self.view.clone(),
        }
    }
}

impl<S: 'static> Rest<S> {
    /// Rows `delays` gives delays for report to `report`.
    pub fn new(
        delays: impl Fn(&S, &RowCx<'_>) -> Option<(Duration, Duration)> + 'static,
        report: impl Fn(&mut S, RestReport, &mut Tx) + 'static,
    ) -> Self {
        Self {
            delays: Rc::new(delays),
            report: Rc::new(report),
            part: None,
            view: None,
        }
    }

    /// A part added to the rows `build` returns one for, last, made inert and hidden from
    /// assistive technology: it takes no input and says nothing. The builder pins it within the
    /// row (C22's peek anchor). Built again whenever `version` changes.
    pub fn part(
        mut self,
        version: impl Fn(&S, &RowCx<'_>) -> u64 + 'static,
        build: impl Fn(&S, &RowCx<'_>) -> Option<El> + 'static,
    ) -> Self {
        self.part = Some((Rc::new(version), Rc::new(build)));
        self
    }

    /// Hears which entries the viewport shows whenever that changes (a scroll, a listing
    /// change): C22 closes a peek whose folder left the view. Return true when it changed what
    /// the view reads. Rune's `on_visible_rows` gives no transaction.
    pub fn on_view(mut self, f: impl Fn(&mut S, RestView) -> bool + 'static) -> Self {
        self.view = Some(Rc::new(f));
        self
    }

    /// The memo version of what this hook adds to a row.
    fn version(&self, s: &S, rcx: &RowCx<'_>) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        (self.delays)(s, rcx).hash(&mut h);
        if let Some((version, _)) = &self.part {
            version(s, rcx).hash(&mut h);
        }
        h.finish()
    }
}

/// What the viewport shows, for [`Rest::on_view`].
///
/// Fields may be added.
///
/// Used by: C22.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RestView {
    /// The collection's key.
    pub collection: Rc<str>,
    /// The entries the viewport shows, in order.
    pub visible: Vec<EntryId>,
}

/// A row's rest report: the pointer came to rest on it, or left it.
///
/// Fields may be added.
///
/// Used by: C22.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RestReport {
    /// The collection's key.
    pub collection: Rc<str>,
    /// The row's entry.
    pub id: EntryId,
    /// The pointer rests on the row (`true`) or left it (`false`).
    pub inside: bool,
    /// The row's rect in window points, as the element lays it out now. Empty on a leave from
    /// an entry the source no longer holds.
    pub row: Rect,
    /// The viewport's rect in window points.
    pub viewport: Rect,
}

/// The width of a row's outline (the cursor, a drop target, the Enter target): a hairline,
/// as Strata draws them (`outline: 1px`, `src/style.css`). No look token sizes outlines.
const OUTLINE: f32 = 1.0;

/// What the view decided once for every row it builds.
#[derive(Clone, Copy, Debug)]
pub(super) struct Frame {
    pub look_key: u64,
    /// What the builder's settings make every row: see [`config_key`].
    pub config_key: u64,
    pub row_height: f32,
    pub per_line: usize,
    /// The space left and right of a line of several entries (`Lines::inset`).
    pub inset: f32,
    pub len: usize,
    /// Drawn in groups under header lines: rows state their own place in the set.
    pub grouped: bool,
    pub hover_live: bool,
    pub cursor_shown: bool,
    pub drop: Option<DropOver>,
    pub renaming: Option<EntryId>,
}

/// A key for everything in the look that rows read, as a memo version.
pub(super) fn look_key(look: &Look) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    look.kit.c.hash(&mut h);
    look.kit.ty.hash(&mut h);
    look.kit.radius.hash(&mut h);
    look.kit.dark.hash(&mut h);
    look.space.hash(&mut h);
    look.icon_size.hash(&mut h);
    look.row.hash(&mut h);
    look.content.hash(&mut h);
    look.icons.name.hash(&mut h);
    (look.icons.icon as usize).hash(&mut h);
    (look.icons.named as usize).hash(&mut h);
    for v in [
        tokens::row_inset(look),
        tokens::row_radius(look),
        tokens::cut_opacity(look),
        tokens::drag_opacity(look),
    ] {
        v.to_bits().hash(&mut h);
    }
    h.finish()
}

/// A key for the builder settings that change what every row is, as a memo version: its
/// roles, read-only, whether it takes drops, and which parts a slot fills. A collection whose
/// builder changes one of them (a view that switches roles) builds its rows again.
pub(super) fn config_key<S>(cfg: &Config<S>) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    cfg.roles.hash(&mut h);
    cfg.read_only.hash(&mut h);
    cfg.on_drop.is_some().hash(&mut h);
    cfg.icon.is_some().hash(&mut h);
    cfg.label.is_some().hash(&mut h);
    cfg.content.is_some().hash(&mut h);
    cfg.describe.is_some().hash(&mut h);
    // Hashed only when set, so every collection without bare items keeps its rows' versions.
    if cfg.bare_items {
        true.hash(&mut h);
    }
    if cfg.rest.is_some() {
        "rest".hash(&mut h);
    }
    h.finish()
}

/// Virtual line `line`: its one entry's row, a row of its entries, or a group's header.
pub(super) fn build_line<S: 'static>(
    cfg: &Rc<Config<S>>,
    frame: &Frame,
    map: &LineMap,
    s: &S,
    line: usize,
) -> El {
    let per = frame.per_line.max(1);
    let (start, end) = match map.line(line, frame.len) {
        Line::Header(g) => {
            let group = &map.groups().expect("a header line is grouped").groups()[g];
            let look = (cfg.look)(s);
            return header_line(cfg, &(cfg.group_style)(look), frame.row_height, group);
        }
        Line::Entries(r) => (r.start, r.end),
    };
    if per == 1 {
        return build_item(cfg, frame, s, start);
    }
    let (src, _) = (cfg.get)(s);
    let first = src.id_at(start);
    let mut el = row().key(line_key(first, per)).w_full().h(frame.row_height);
    if frame.inset > 0.0 {
        el = el.px(frame.inset);
    }
    if let Some(role) = cfg.roles.line {
        el = el.role(role);
    }
    for i in start..end {
        el = el.child(build_item(cfg, frame, s, i));
    }
    for _ in end..start + per {
        el = el.child(row().w(0.0).grow(1.0));
    }
    el
}

/// The row (or grid cell) of entry `index`, memoized.
fn build_item<S: 'static>(cfg: &Rc<Config<S>>, frame: &Frame, s: &S, index: usize) -> El {
    let (src, st) = (cfg.get)(s);
    let look = (cfg.look)(s);
    let entry = src.entry(index);
    let id = entry.id;
    let selected = st.selection.is_selected_at(src, index);
    let cursor = st.selection.cursor() == Some(id);
    let state = RowState {
        selected,
        provisional: selected && st.selection.is_provisional(),
        cursor,
        cursor_shown: cursor && frame.cursor_shown,
        drop_target: frame.drop == Some(DropOver::Row(id)),
        renaming: cfg.rename.is_some() && !cfg.read_only && frame.renaming == Some(id),
        marks: cfg
            .marks
            .as_ref()
            .map_or_else(RowMarks::default, |f| f(s, id)),
    };
    let rcx = RowCx {
        index,
        entry,
        state,
        look,
        scope: &cfg.key,
        hover_live: frame.hover_live,
    };
    if state.renaming {
        return item(cfg, frame, s, &rcx);
    }
    let versions = [
        cfg.icon
            .as_ref()
            .and_then(|v| v.version.as_ref())
            .map(|f| f(s, &rcx)),
        cfg.label
            .as_ref()
            .and_then(|v| v.version.as_ref())
            .map(|f| f(s, &rcx)),
        cfg.content
            .as_ref()
            .and_then(|v| v.version.as_ref())
            .map(|f| f(s, &rcx)),
        cfg.describe
            .as_ref()
            .and_then(|v| v.version.as_ref())
            .map(|f| f(s, &rcx)),
    ];
    // A grid cell, and a row among header lines, states its place in the set itself, so it
    // changes with its index.
    let place = (frame.per_line > 1 || frame.grouped).then_some((index, frame.len));
    // Only rows that draw them read the rest hook and the peek fill.
    let rest = cfg.rest.as_ref().map(|r| r.version(s, &rcx));
    let peek_fill = state
        .marks
        .peek_anchor
        .then(|| color_key(tokens::peek_fill(look)));
    let version = (
        (entry.name, entry.kind, entry.icon, entry.thumbnail),
        state,
        frame.look_key,
        frame.config_key,
        frame.row_height.to_bits(),
        frame.hover_live,
        versions,
        place,
        (rest, peek_fill),
    );
    memo(row_key(id), version, || item(cfg, frame, s, &rcx))
}

fn item<S: 'static>(cfg: &Rc<Config<S>>, frame: &Frame, s: &S, rcx: &RowCx<'_>) -> El {
    let look = rcx.look;
    let kit = &look.kit;
    let state = rcx.state;
    let entry = rcx.entry;
    let id = entry.id;
    if state.marks.vanished {
        // Keyed, so the slot keeps its place and a motion effect can still find it.
        let slot = el()
            .key(row_key(id))
            .h(frame.row_height)
            .a11y_hidden()
            .inert();
        return if frame.per_line > 1 {
            slot.w(0.0).grow(1.0)
        } else {
            slot.w_full()
        };
    }

    let icon_el = match &cfg.icon {
        Some(slot) => (slot.build)(s, rcx),
        None => default_icon(rcx),
    };
    let label_el = match (&cfg.rename, &cfg.label) {
        (Some((_, slot)), _) if state.renaming => (slot.build)(s, rcx),
        (_, Some(slot)) => (slot.build)(s, rcx),
        _ => default_label(rcx),
    };
    let content = match &cfg.content {
        Some(slot) => (slot.build)(s, rcx, icon_el, label_el),
        None => row()
            .w_full()
            .h_full()
            .gap(look.space.sm)
            .center_y()
            .child(icon_el)
            .child(label_el),
    };

    let mut opacity: f32 = 1.0;
    if state.marks.cut {
        opacity = opacity.min(tokens::cut_opacity(look));
    }
    if state.marks.dragging {
        opacity = opacity.min(tokens::drag_opacity(look));
    }
    if cfg.bare_items {
        // The content slot's element is the item: no box of the row's own around it (a grid
        // mounts a line of these in one frame, and each element costs).
        let mut item = content
            .key(row_key(id))
            .role(cfg.roles.item)
            .label(entry.name)
            .focus_selected(state.selected)
            .h(frame.row_height);
        if opacity < 1.0 {
            item = item.opacity(opacity);
        }
        return finish_item(cfg, frame, s, rcx, item);
    }
    let mut inner = row()
        .w_full()
        .h_full()
        .radius(tokens::row_radius(look))
        .px(look.space.xs)
        .center_y()
        .clip_soft(0.0);
    if state.selected || state.drop_target {
        inner = inner.fill(rune_kit::TableStyle::from_kit(kit).selected);
    } else if state.marks.peek_anchor {
        inner = inner.fill(tokens::peek_fill(look));
    }
    if frame.hover_live {
        inner = inner.hover(kit.c.hover);
    }
    if state.cursor_shown {
        inner = inner.outline(OUTLINE, -OUTLINE, kit.c.border_focus);
    } else if state.drop_target || state.marks.enter_target {
        inner = inner.outline(OUTLINE, -OUTLINE, kit.c.text_accent);
    }
    if opacity < 1.0 {
        inner = inner.opacity(opacity);
    }
    inner = inner.child(content);

    let outer = row()
        .key(row_key(id))
        .role(cfg.roles.item)
        .label(entry.name)
        .focus_selected(state.selected)
        .h(frame.row_height)
        .px(tokens::row_inset(look))
        .child(inner);
    finish_item(cfg, frame, s, rcx, outer)
}

/// An item's width in its line, its place in the set, its description and its drop zone.
fn finish_item<S: 'static>(
    cfg: &Rc<Config<S>>,
    frame: &Frame,
    s: &S,
    rcx: &RowCx<'_>,
    mut outer: El,
) -> El {
    let entry = rcx.entry;
    let id = entry.id;
    outer = if frame.per_line > 1 {
        outer.w(0.0).grow(1.0)
    } else {
        outer.w_full()
    };
    if frame.per_line > 1 || frame.grouped {
        outer = outer.position_in_set(rcx.index + 1, frame.len);
    }
    if let Some(description) = cfg.describe.as_ref().and_then(|d| (d.build)(s, rcx)) {
        outer = outer.description(description);
    }
    if let Some(rest) = &cfg.rest {
        outer = rest_row(cfg, rest, s, rcx, outer);
    }
    if cfg.read_only {
        return outer;
    }
    if let Some(on_drop) = &cfg.on_drop
        && entry.kind.is_folder_like()
    {
        // Row keys are unique only inside the collection: its key is their scope.
        let key = ElementKey::new(row_key(id)).within(cfg.key.to_string());
        outer = cfg
            .dnd
            .drop_zone_keyed(outer, key, row_zone(cfg, on_drop.clone(), id));
    }
    outer
}

/// Row `outer` with the rest hook's delays and report, and its part.
fn rest_row<S: 'static>(
    cfg: &Rc<Config<S>>,
    rest: &Rest<S>,
    s: &S,
    rcx: &RowCx<'_>,
    mut outer: El,
) -> El {
    if let Some((enter, leave)) = (rest.delays)(s, rcx) {
        let report_cfg = cfg.clone();
        let id = rcx.entry.id;
        // Not a press handler, so presses still reach the container (`Handlers::has_primary`
        // at the pinned rev).
        outer = outer
            .hover_delays(enter, leave)
            .on_hover_change(move |s: &mut S, inside, tx| {
                super::route::on_rest(&report_cfg, s, id, inside, tx)
            });
    }
    if let Some(part) = rest.part.as_ref().and_then(|(_, build)| build(s, rcx)) {
        outer = outer.child(part.inert().a11y_hidden());
    }
    outer
}

/// A colour as a memo key.
fn color_key(c: rune::Color) -> [u32; 4] {
    [c.r, c.g, c.b, c.a].map(f32::to_bits)
}

/// A group's header line: its label over the style's divider, a heading named by the label.
fn header_line<S: 'static>(cfg: &Config<S>, style: &GroupStyle, h: f32, group: &Group) -> El {
    header_body(style, h, text(group.label.to_string()).truncate())
        .key(header_key(group.key))
        .role(cfg.roles.header)
        .label(group.label.to_string())
}

/// What a header draws, around its label part: the stuck header's too, so the two match.
pub(super) fn header_body(style: &GroupStyle, h: f32, label: El) -> El {
    let mut el = row()
        .w_full()
        .h(h)
        .px(style.inset)
        .pb(style.bottom)
        .items_end()
        .child(
            label
                .key(LABEL)
                .a11y_hidden()
                .font(style.font)
                .color(style.color)
                .shrink()
                .min_w(0.0),
        );
    if let Some(divider) = style.divider {
        el = el.border_sides([0.0, 0.0, OUTLINE, 0.0], divider);
    }
    el
}

/// The entry's catalog icon, else the Folder or File icon, at the row icon size.
fn default_icon(rcx: &RowCx<'_>) -> El {
    let look = rcx.look;
    let fallback = if rcx.entry.kind.is_folder_like() {
        IconName::Folder
    } else {
        IconName::File
    };
    let glyph = look
        .icon_named(rcx.entry.icon)
        .unwrap_or_else(|| look.icon(fallback));
    icon(glyph, look.icon_size.md)
        .key(ICON)
        .color(look.kit.c.icon_base)
        .a11y_hidden()
}

/// The entry's name, cut short with an ellipsis.
fn default_label(rcx: &RowCx<'_>) -> El {
    let kit = &rcx.look.kit;
    text(rcx.entry.name.to_owned())
        .key(LABEL)
        .a11y_hidden()
        .font(kit.ty.ui_compact)
        .color(kit.c.text_base)
        .truncate()
        .shrink()
        .min_w(0.0)
}

pub(super) type DropFn<S> = Rc<dyn Fn(&mut S, Option<EntryId>, crate::contract::Dropped, &mut Tx)>;

/// A folder row's drop zone: drops go to `on_drop` with the folder's id, the hover draws the
/// row's drop highlight, and moves near the viewport's edges scroll it.
///
/// The stuck group header takes no input, so a drag over it is over the row it covers: there
/// the drop goes to the folder shown, as on a header line, judged by the drop's point (or the
/// last move's, when the platform gives the drop none). The row's drop highlight follows the
/// moves, and is hidden under the stuck header anyway.
fn row_zone<S: 'static>(cfg: &Rc<Config<S>>, on_drop: DropFn<S>, id: EntryId) -> DropZone<S> {
    let get_mut = cfg.get_mut.clone();
    let hover_cfg = cfg.clone();
    let edge = cfg.clone();
    let drop_cfg = cfg.clone();
    let zone = DropZone::new(move |s: &mut S, dropped: crate::contract::Dropped, tx| {
        // Where it landed when the platform says, else where the last move was (a drop with
        // no position and no move over the stuck header goes to the row it covers).
        let under_stuck = dropped
            .point
            .map(|p| super::route::under_stuck_header(&drop_cfg, s, p));
        let st = (get_mut)(s).1;
        let last = st.drop.take();
        let target = match (under_stuck, last) {
            (Some(true), _) | (None, Some(DropOver::Background)) => None,
            _ => Some(id),
        };
        super::route::dropped_on_row(&drop_cfg, s, id);
        on_drop(s, target, dropped, tx);
    });
    // With the host's action, a folder row it refuses (the drag's own folder, one inside the
    // drag) stops the drop: passed on, the background around it would take it into the folder
    // shown.
    let zone = match cfg.drop_action.clone() {
        Some(f) => {
            let reject_cfg = cfg.clone();
            zone.action(move |s: &S, offer, mods| f(s, Some(id), offer, mods))
                .on_reject(move |s: &mut S, _, tx| {
                    super::route::dropped_on_row(&reject_cfg, s, id);
                    super::note_drop_hover(
                        &reject_cfg,
                        s,
                        DropOver::Row(id),
                        DropHover::Left,
                        None,
                    );
                    super::drag_over_hover(&reject_cfg, s, DropHover::Left, tx);
                })
        }
        None => zone,
    };
    zone.on_hover(move |s: &mut S, hover, offer, tx| {
        super::note_drop_hover(&hover_cfg, s, DropOver::Row(id), hover, Some(offer));
        super::drag_over_hover(&hover_cfg, s, hover, tx);
    })
    .on_move(move |s: &mut S, point, tx| {
        let under_stuck = super::route::under_stuck_header(&edge, s, point);
        let st = (edge.get_mut)(s).1;
        // A refused drag's moves (P1 sends them to a zone with on_reject) only drive the
        // hook, and only a file drag's: no highlight.
        let refused = st.drop_refused == Some(DropOver::Row(id));
        if refused && !st.drop_refused_files {
            tx.live_only();
            return;
        }
        let over = if refused {
            None
        } else if under_stuck {
            Some(DropOver::Background)
        } else {
            Some(DropOver::Row(id))
        };
        let changed = st.drop != over;
        st.drop = over;
        match &edge.on_drag_over {
            Some(f) => {
                let host = f(s, Some(Point::new(point.window_x, point.window_y)), tx);
                if !changed && !host {
                    tx.live_only();
                }
            }
            None => super::route::drop_edge_scroll(&edge, s, point, changed, tx),
        }
    })
}
