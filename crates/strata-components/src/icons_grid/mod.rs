// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod caption;
mod geometry;
mod metrics;
mod regions;
mod size;
mod state;
mod tile;

#[cfg(test)]
mod tests;

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use rune::{El, Tx, WheelEvent, col};
use strata_kit::look::Look;

pub use caption::Cut;
pub use geometry::{GridLines, IconsGeometry};
pub use metrics::{COLUMN_SPACING, MIN_CARD, TileMetrics};
pub use size::{
    PRECISE_STEP, SizeCommand, SizeKeyed, SizeKeys, SizeRequest, WheelSteps, size_command,
};
pub use state::IconsGridState;
pub use tile::DETAILS;

use crate::collection::element::{
    Collection, CollectionState, Ctx, PointerInput, Region, Roles, RowCx, RowState, entry_path,
    viewport_path,
};
use crate::contract::{EntryId, EntrySource, Point};
use crate::scroll::{AutoscrollSettings, BarLook};
use crate::thumbnails::{Step, ThumbnailStore, ViewerId, Viewport};
use tile::Built;

/// What the grid reads from the host's state, in one accessor.
pub struct IconsParts<'a> {
    /// The listing, as the collection shows it.
    pub source: &'a dyn EntrySource,
    /// The collection's state.
    pub collection: &'a CollectionState,
    /// The grid's own state.
    pub grid: &'a IconsGridState,
    /// The tile size in points: the host's, per folder or the default.
    pub size: f32,
    /// The listing version the collection is built with, for the thumbnail store.
    pub listing: u64,
    /// The look.
    pub look: &'a Look,
    /// The thumbnail store and this grid's viewer in it, when thumbnails show.
    pub thumbs: Option<(&'a ThumbnailStore, ViewerId)>,
}

type PartsFn<S> = Rc<dyn for<'a> Fn(&'a S) -> IconsParts<'a>>;
type GridMutFn<S> = Rc<dyn Fn(&mut S) -> &mut IconsGridState>;
type ThumbsMutFn<S> = Rc<
    dyn for<'a> Fn(&'a mut S) -> Option<(&'a mut ThumbnailStore, ViewerId, &'a dyn EntrySource)>,
>;
type BaseFn<S> = Rc<dyn Fn(&S) -> Collection<S>>;
type VersionFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> u64>;
type DetailsFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> Option<Arc<str>>>;
type TooltipFn<S> = Rc<dyn Fn(&S, &RowCx<'_>, bool) -> Option<String>>;
type BadgeFn<S> = (
    Rc<dyn Fn(&S, EntryId) -> u64>,
    Rc<dyn Fn(&S, EntryId) -> Option<rune::Pixels>>,
);
type SizeFn<S> = Rc<dyn Fn(&mut S, SizeRequest, &mut Tx)>;
type VisibleFn<S> = Rc<dyn Fn(&mut S, Range<usize>) -> bool>;
type WheelFn<S> = Rc<dyn Fn(&mut S, WheelEvent, &mut Tx) -> bool>;

/// What the grid was built with, shared by its parts' handlers.
pub(crate) struct Shared<S> {
    key: Rc<str>,
    base: BaseFn<S>,
    parts: PartsFn<S>,
    grid_mut: GridMutFn<S>,
    thumbs_mut: Option<ThumbsMutFn<S>>,
    details: Option<(VersionFn<S>, DetailsFn<S>)>,
    tooltip: Option<(VersionFn<S>, TooltipFn<S>)>,
    badge: Option<BadgeFn<S>>,
    lines: usize,
    on_size: Option<SizeFn<S>>,
    on_visible: Option<VisibleFn<S>>,
    on_wheel: Option<WheelFn<S>>,
    scale: Option<f32>,
}

impl<S> Clone for Shared<S> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            base: self.base.clone(),
            parts: self.parts.clone(),
            grid_mut: self.grid_mut.clone(),
            thumbs_mut: self.thumbs_mut.clone(),
            details: self.details.clone(),
            tooltip: self.tooltip.clone(),
            badge: self.badge.clone(),
            lines: self.lines,
            on_size: self.on_size.clone(),
            on_visible: self.on_visible.clone(),
            on_wheel: self.on_wheel.clone(),
            scale: self.scale,
        }
    }
}

impl<S: 'static> Shared<S> {
    fn metrics(&self, s: &S) -> TileMetrics {
        let p = (self.parts)(s);
        TileMetrics::new(p.look, p.size, self.lines, self.details.is_some())
    }

    fn scale(&self, s: &S) -> f32 {
        self.scale.unwrap_or_else(|| (self.parts)(s).grid.scale())
    }

    fn step(&self, s: &S, m: &TileMetrics) -> Step {
        Step::for_tile((self.parts)(s).look, m.slot, self.scale(s))
    }

    fn built(&self, s: &S) -> Built {
        let m = self.metrics(s);
        let w = (self.parts)(s).collection.viewport().w;
        let per = m.per_line(w);
        let cell_w = m.cell_width(w, per);
        Built {
            m,
            cell_w,
            caption_w: m.caption_width(cell_w),
            step: self.step(s, &m),
        }
    }
}

/// The Icons view's grid: see the module docs.
pub struct IconsGrid<S> {
    shared: Shared<S>,
}

impl<S: 'static> IconsGrid<S> {
    /// A grid over the collection `base` builds (keyed `key`, as `base` keys it).
    pub fn new(
        key: &str,
        base: impl Fn(&S) -> Collection<S> + 'static,
        parts: impl for<'a> Fn(&'a S) -> IconsParts<'a> + 'static,
        grid_mut: impl Fn(&mut S) -> &mut IconsGridState + 'static,
    ) -> Self {
        Self {
            shared: Shared {
                key: key.into(),
                base: Rc::new(base),
                parts: Rc::new(parts),
                grid_mut: Rc::new(grid_mut),
                thumbs_mut: None,
                details: None,
                tooltip: None,
                badge: None,
                lines: 2,
                on_size: None,
                on_visible: None,
                on_wheel: None,
                scale: None,
            },
        }
    }

    /// Overlays an optional corner badge (`Pixels`) in the bottom-right corner of each tile's thumbnail slot.
    pub fn badge(
        mut self,
        version: impl Fn(&S, EntryId) -> u64 + 'static,
        pixels: impl Fn(&S, EntryId) -> Option<rune::Pixels> + 'static,
    ) -> Self {
        self.shared.badge = Some((Rc::new(version), Rc::new(pixels)));
        self
    }

    /// The thumbnail store and the grid's viewer in it, with the listing, for the grid's own calls.
    pub fn thumbs_mut(
        mut self,
        f: impl for<'a> Fn(&'a mut S) -> Option<(&'a mut ThumbnailStore, ViewerId, &'a dyn EntrySource)>
        + 'static,
    ) -> Self {
        self.shared.thumbs_mut = Some(Rc::new(f));
        self
    }

    /// The details line under each name ("12 items", "320×180", "4.2 MB"): the host's metadata, streamed in as it arrives.
    pub fn details(
        mut self,
        version: impl Fn(&S, &RowCx<'_>) -> u64 + 'static,
        text: impl Fn(&S, &RowCx<'_>) -> Option<Arc<str>> + 'static,
    ) -> Self {
        self.shared.details = Some((Rc::new(version), Rc::new(text)));
        self
    }

    /// Hover tooltip text for each tile, given whether its caption was truncated.
    pub fn tooltip(
        mut self,
        version: impl Fn(&S, &RowCx<'_>) -> u64 + 'static,
        text: impl Fn(&S, &RowCx<'_>, bool) -> Option<String> + 'static,
    ) -> Self {
        self.shared.tooltip = Some((Rc::new(version), Rc::new(text)));
        self
    }

    /// How many lines a name takes at most: Strata's 2 by default; 0 shows no name.
    pub fn caption_lines(mut self, lines: usize) -> Self {
        self.shared.lines = lines;
        self
    }

    /// Hears a size step or reset (keys, Ctrl+Shift+wheel, `IconsGrid::apply_size`).
    pub fn on_size(mut self, f: impl Fn(&mut S, SizeRequest, &mut Tx) + 'static) -> Self {
        self.shared.on_size = Some(Rc::new(f));
        self
    }

    /// Hears which entries the viewport shows, after the grid synced its thumbnails (a folder recount, say).
    pub fn on_visible(mut self, f: impl Fn(&mut S, Range<usize>) -> bool + 'static) -> Self {
        self.shared.on_visible = Some(Rc::new(f));
        self
    }

    /// Hears each wheel and trackpad delta over the grid before its size wheel does.
    pub fn on_wheel(mut self, f: impl Fn(&mut S, WheelEvent, &mut Tx) -> bool + 'static) -> Self {
        self.shared.on_wheel = Some(Rc::new(f));
        self
    }

    /// The display scale the view builds at (`cx.env().scale`), which picks the thumbnail step.
    pub fn scale(mut self, scale: f32) -> Self {
        self.shared.scale = (scale.is_finite() && scale > 0.0).then_some(scale);
        self
    }

    /// The tile sizes for `s` as it stands.
    pub fn metrics(&self, s: &S) -> TileMetrics {
        self.shared.metrics(s)
    }

    /// The layout key a scroll offset is remembered under in Icons.
    pub fn layout_key(width: f32, size: f32, look: &Look) -> u64 {
        let m = TileMetrics::new(look, size, 2, true);
        let mut h = std::hash::DefaultHasher::new();
        "icons".hash(&mut h);
        m.per_line(width).hash(&mut h);
        m.height.to_bits().hash(&mut h);
        h.finish()
    }

    /// The full key path of entry `id`'s tile as the grid lays it out now (it changes when the grid reflows to another count): where finds a folder tile's drop zone, and what a motion effect flies from (`{path}/icon`).
    pub fn tile_path(&self, s: &S, id: EntryId) -> Option<String> {
        let p = (self.shared.parts)(s);
        let index = p.source.index_of(id)?;
        Some(entry_path(
            &self.shared.key,
            p.source,
            index,
            p.collection.per_line(),
        ))
    }

    /// `base` with the grid's parts: its roles (`Roles::GRID_CELLS`), bare items, row height, the `GridLines` packing.
    pub fn collection(&self, s: &S) -> Collection<S> {
        collection_with(&Rc::new(self.shared.clone()), s)
    }

    /// The grid, filling the height its parent gives it.
    pub fn build(&self, s: &S) -> El {
        let shared = Rc::new(self.shared.clone());
        let parts = (shared.parts)(s);
        if let Some(scale) = shared.scale {
            parts.grid.scale.set(scale);
        }
        let list = collection_with(&shared, s).build(s);
        let grid_mut = shared.grid_mut.clone();
        let bar = (shared.parts)(s).grid.bar.clone();
        let grid = col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .on_layout_tx(move |s: &mut S, r, tx: &mut Tx| {
                let grid = grid_mut(s);
                if grid.width.get() == r[2] {
                    tx.live_only();
                } else {
                    grid.width.set(r[2]);
                }
            })
            .child(list);
        // The pointer over the grid shows its scroll bar.
        bar.area(grid)
    }

    /// Steps or resets the size: the host hears the request (`IconsGrid::on_size`), sets its size.
    pub fn apply_size(&self, s: &mut S, command: SizeCommand, tx: &mut Tx) {
        apply_size(&Rc::new(self.shared.clone()), s, command, tx);
    }

    /// Lays out at a size the host changed itself (a slider, another window, a folder with its own size).
    pub fn size_changed(&self, s: &mut S, tx: &mut Tx) -> bool {
        size_changed(&Rc::new(self.shared.clone()), s, tx)
    }

    /// From the host's poll: catches a size, display scale or thumbnail-listing version change the host made without `IconsGrid::size_changed`, so the thumbnail store and cursor reveal follow within a frame, and flushes thumbnail syncs deferred while dragging the scrollbar thumb once the thumb releases or pauses.
    pub fn poll(&self, s: &mut S, tx: &mut Tx) -> bool {
        let shared = Rc::new(self.shared.clone());
        let (size, last) = {
            let p = (shared.parts)(s);
            (p.size, p.grid.size.get())
        };
        let mut rebuild = false;
        if last != Some(size) {
            size_changed(&shared, s, tx);
        } else {
            rebuild |= flush_thumbs(&shared, s, tx);
        }
        if !(shared.parts)(s).grid.thumbs_deferred() {
            rebuild |= sync_stale(&shared, s);
            if let Some((store, _)) = (shared.parts)(s).thumbs {
                rebuild |= store.take_skipped();
            }
        }
        rebuild
    }
}

/// Where a middle press starts autoscroll in a grid.
pub fn autoscroll_background(
    input: &PointerInput,
    _cx: &Ctx<'_>,
    _settings: &AutoscrollSettings,
) -> bool {
    input.hit.index.is_none() || input.hit.region == Region::Gutter
}

fn collection_with<S: 'static>(shared: &Rc<Shared<S>>, s: &S) -> Collection<S> {
    let b = shared.built(s);
    let m = b.m;
    let region_shared = shared.clone();
    let visible_shared = shared.clone();
    let wheel_shared = shared.clone();
    let base = (shared.base)(s);
    let grid = (shared.parts)(s).grid;
    let (bar, feed) = (grid.bar.clone(), grid.feed.clone());
    let edge_bar = bar.clone();
    let (bar_key, target) = (format!("{}-bar", shared.key), viewport_path(base.scope()));
    let edge_shared = shared.clone();
    base.on_scroll(move |p| {
        feed.set(p);
        bar.follow(p);
    })
    .edge(move |s: &S| {
        let look = BarLook::listing((edge_shared.parts)(s).look);
        Some(edge_bar.build::<S>(&bar_key, &target, look))
    })
    .roles(Roles::GRID_CELLS)
    .bare_items()
    .row_height(move |_: &S, _: &Look| m.height)
    .lines(GridLines::new(m))
    .icon(tile::icon_slot(shared, b))
    .label(tile::label_slot(shared, b))
    .content(tile::content_slot(shared, b))
    .regions(move |s: &S, index, local: Point| {
        let p = (region_shared.parts)(s);
        if index >= p.source.len() {
            return Region::Gutter;
        }
        let entry = p.source.entry(index);
        let cut = b.cut(&region_shared, s, entry.name);
        let details_w = region_shared
            .details
            .as_ref()
            .and_then(|(_, f)| {
                let rcx = row_cx(p.look, entry, index, &region_shared.key);
                f(s, &rcx)
            })
            .filter(|d| !d.is_empty())
            .map_or(0.0, |d| {
                rune::text::measure(&d, &[], m.details_font, None).0 + 2.0 * m.caption_pad
            });
        regions::region(&m, b.cell_w, &cut, details_w, local)
    })
    .on_visible(move |s: &mut S, range| on_visible(&visible_shared, s, range))
    .on_wheel(move |s: &mut S, e: WheelEvent, tx: &mut Tx| on_wheel(&wheel_shared, s, e, tx))
}

/// A [`RowCx`] for reading the host's details outside a build (the regions).
fn row_cx<'a>(
    look: &'a Look,
    entry: crate::contract::Entry<'a>,
    index: usize,
    scope: &'a str,
) -> RowCx<'a> {
    RowCx {
        index,
        entry,
        state: RowState::default(),
        look,
        scope,
        hover_live: false,
    }
}

/// How long a held scrollbar thumb stays still before deferred thumbnail syncs flush.
const THUMB_DRAG_SETTLE: f64 = 0.150;

fn on_visible<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, range: Range<usize>) -> bool {
    let synced = if (shared.parts)(s).grid.bar.is_dragging() {
        let grid = (shared.grid_mut)(s);
        grid.pending_thumbs = Some(range.clone());
        grid.thumb_moved = true;
        grid.thumb_settled.set(false);
        false
    } else {
        let grid = (shared.grid_mut)(s);
        grid.pending_thumbs = None;
        grid.thumb_moved = false;
        grid.thumb_moved_at = None;
        grid.thumb_settled.set(false);
        sync_thumbs(shared, s, Some(range.clone()))
    };
    let host = shared.on_visible.as_ref().is_some_and(|f| f(s, range));
    synced || host
}

fn flush_thumbs<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, tx: &mut Tx) -> bool {
    let dragging = (shared.parts)(s).grid.bar.is_dragging();
    let clock = tx.clock();
    let ready = {
        let grid = (shared.grid_mut)(s);
        if !dragging {
            grid.thumb_moved = false;
            grid.thumb_moved_at = None;
            grid.thumb_settled.set(false);
            grid.pending_thumbs.take()
        } else if grid.pending_thumbs.is_none() {
            if grid.thumb_moved_at.is_none() {
                grid.thumb_settled.set(true);
            }
            return false;
        } else {
            if std::mem::take(&mut grid.thumb_moved) {
                grid.thumb_moved_at = Some(clock);
            }
            match grid.thumb_moved_at {
                Some(at) if clock - at < THUMB_DRAG_SETTLE => {
                    tx.wake_after(std::time::Duration::from_secs_f64(
                        THUMB_DRAG_SETTLE - (clock - at),
                    ));
                    None
                }
                _ => {
                    grid.thumb_moved_at = None;
                    grid.thumb_settled.set(true);
                    grid.pending_thumbs.take()
                }
            }
        }
    };
    let Some(range) = ready else {
        return false;
    };
    sync_thumbs(shared, s, Some(range))
}

fn sync_thumbs<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, range: Option<Range<usize>>) -> bool {
    let Some(thumbs_mut) = shared.thumbs_mut.clone() else {
        return false;
    };
    let m = shared.metrics(s);
    let step = shared.step(s, &m);
    let (per, listing) = {
        let p = (shared.parts)(s);
        (p.collection.per_line().max(1), p.listing)
    };
    let Some((store, viewer, src)) = thumbs_mut(s) else {
        return false;
    };
    let has_range = range.is_some();
    let pruned = match range {
        Some(r) => store.sync(
            viewer,
            src,
            Viewport::new(r, step).per_line(per).listing(listing),
        ),
        None => store.set_step(viewer, src, step),
    };
    let grid = (shared.parts)(s).grid;
    grid.synced.set(Some(step));
    if has_range {
        grid.synced_listing.set(Some(listing));
    }
    pruned
}

fn resync_thumbs<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S) -> bool {
    let Some(thumbs_mut) = shared.thumbs_mut.clone() else {
        return false;
    };
    let listing = (shared.parts)(s).listing;
    let Some((store, viewer, src)) = thumbs_mut(s) else {
        return false;
    };
    let pruned = store.resync(viewer, src, listing);
    (shared.parts)(s).grid.synced_listing.set(Some(listing));
    pruned
}

/// Syncs the store when the thumbnail step or listing version changed since the last sync.
fn sync_stale<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S) -> bool {
    if shared.thumbs_mut.is_none() {
        return false;
    }
    let m = shared.metrics(s);
    let step = shared.step(s, &m);
    let (synced_step, synced_listing, listing, visible, empty) = {
        let p = (shared.parts)(s);
        (
            p.grid.synced.get(),
            p.grid.synced_listing.get(),
            p.listing,
            p.collection.visible_range(),
            p.source.is_empty(),
        )
    };
    if synced_listing.is_none() && (!visible.is_empty() || empty) {
        return sync_thumbs(shared, s, Some(visible));
    }
    let mut pruned = false;
    if synced_step != Some(step) {
        pruned |= sync_thumbs(shared, s, None);
    }
    if synced_listing.is_some_and(|prev| prev != listing) {
        pruned |= resync_thumbs(shared, s);
    }
    pruned
}

/// Asks the store for the step the grid draws at now, if it isn't the one it last asked for.
fn sync_step<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S) -> bool {
    if shared.thumbs_mut.is_none() {
        return false;
    }
    let m = shared.metrics(s);
    let step = shared.step(s, &m);
    if (shared.parts)(s).grid.synced.get() == Some(step) {
        return false;
    }
    sync_thumbs(shared, s, None)
}

fn size_changed<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, tx: &mut Tx) -> bool {
    let (size, shown) = {
        let p = (shared.parts)(s);
        let cursor = p.collection.selection.cursor_index(p.source);
        let shown = cursor.filter(|i| p.collection.visible_range().contains(i));
        (p.size, shown)
    };
    let old = (shared.grid_mut)(s).size.replace(Some(size));
    let reflowed = collection_with(shared, s).reflow(s, tx);
    if !reflowed
        && old.is_some_and(|old| old != size)
        && let Some(index) = shown
    {
        let p = (shared.parts)(s);
        p.collection.reveal(&shared.key, p.source, index, tx);
    }
    sync_step(shared, s);
    reflowed
}

fn apply_size<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, command: SizeCommand, tx: &mut Tx) {
    let request = {
        let p = (shared.parts)(s);
        match command {
            SizeCommand::Step(n) => SizeRequest::Step {
                from: p.size,
                to: p.look.tiles.step(p.size, n),
            },
            SizeCommand::Reset => SizeRequest::Reset,
        }
    };
    if let Some(f) = &shared.on_size {
        f(s, request, tx);
    }
    size_changed(shared, s, tx);
}

fn on_wheel<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, e: WheelEvent, tx: &mut Tx) -> bool {
    if shared.on_wheel.as_ref().is_some_and(|f| f(s, e, tx)) {
        return true;
    }
    let grid = (shared.grid_mut)(s);
    let mut wheel = grid.wheel.get();
    if !WheelSteps::claims(&e) {
        wheel.reset();
        grid.wheel.set(wheel);
        return false;
    }
    let steps = wheel.steps(&e);
    grid.wheel.set(wheel);
    if steps != 0 {
        apply_size(shared, s, SizeCommand::Step(steps), tx);
    } else {
        tx.live_only();
    }
    true
}
