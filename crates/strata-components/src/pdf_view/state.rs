//! [`PdfState`]: one PDF preview's pages, zoom, selection and find, and the requests it asks
//! the host for.

use std::collections::BTreeMap;
use std::ops::Range;
use std::time::Duration;

use rune::{Drag, Phase, Tx};

use super::find::{Find, Hit};
use super::model::{
    Content, PageReply, PdfDocument, RenderCaps, RenderKind, RenderRequest, Shared, Ticket,
};
use super::queue::{Held, Landed, Queue, Want};
use super::selection::{Selection, Unit};
use super::zoom::{self, GAP, LIMITS, SETTLE};
use crate::preview_drawer::{FindStatus, HostedPreview};
use crate::scroll::{Bar, ScrollFeed};

/// What a primary drag is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Gesture {
    #[default]
    Idle,
    /// Panning from the scroll offset the press found.
    Pan([f32; 2]),
    Select,
}

/// The point under the viewport's centre, kept across a zoom: a page, how far down it the
/// centre is, and how far across the content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Anchor {
    pub page: usize,
    pub down: f32,
    pub across: f32,
    /// The host's clock when it was taken.
    pub at: f64,
    /// Its page has laid out and been asked to scroll into place; the scroll lands on a later
    /// frame, so a zoom before then still uses this anchor.
    pub placed: bool,
}

/// How long a zoom's anchor waits for its page to lay out before it's let go.
const ANCHOR_WAIT: f64 = 0.25;

/// A find match to bring into view: its page, and its centre as fractions of the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Reveal {
    pub page: usize,
    pub at: [f32; 2],
    /// Whether the page has been asked to mount.
    pub asked: bool,
}

/// One PDF preview. The host keeps it in its state and hands the view accessors to it.
///
/// Pages, sizes, images and text layers arrive as data; the view asks for renders through
/// [`PdfState::pop`] and never renders, decodes or reads files.
pub struct PdfState {
    key: String,
    pub(crate) doc: PdfDocument,
    pub(crate) content: Content,
    pub(crate) queue: Queue,
    pub(crate) level: f32,
    /// Bumped at each change of every page's size (a zoom, a new pane width): the rows' keys
    /// carry it, so Rune drops their old heights and places them all from one estimate.
    pub(crate) zoom_gen: u32,
    pub(crate) scale: f32,
    /// The scroll viewport's rect in window points, from its last layout.
    pub(crate) viewport: Option<[f32; 4]>,
    /// Each mounted page's rect in window points, from its last layout.
    pub(crate) rects: BTreeMap<usize, [f32; 4]>,
    pub(crate) visible: Range<usize>,
    forward: bool,
    /// The page under the viewport's centre.
    pub(crate) current: usize,
    pub(crate) selection: Selection,
    pub(crate) gesture: Gesture,
    /// The pointer is over text: the I-beam shows.
    pub(crate) over_text: bool,
    pub(crate) find: Find,
    searchable: bool,
    pub(crate) anchor: Option<Anchor>,
    pub(crate) reveal: Option<Reveal>,
    /// The host's clock at its last [`PdfState::tick`].
    clock: f64,
    settle_at: Option<f64>,
    /// Bumped when requests are waiting, to mount the element that wakes the host.
    pub(crate) wake: u64,
    /// The host's last [`PdfState::pop`] found nothing: it may be at rest, so new work wakes
    /// it. While it pops, its own poll takes the work and no wake is needed.
    idle: bool,
    /// A zoom the host asked for ([`PdfState::set_level`]), applied from a layout report.
    level_request: Option<f32>,
    /// The pointer's last point over the view, in window points: the I-beam follows the text
    /// that scrolls under a still pointer.
    pointer: Option<[f32; 2]>,
    pub(crate) feed: ScrollFeed,
    pub(crate) bar: Bar,
    pub(crate) sideways: Bar,
}

impl std::fmt::Debug for PdfState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfState")
            .field("key", &self.key)
            .field("pages", &self.doc.pages())
            .field("content", &self.content)
            .field("level", &self.level)
            .field("visible", &self.visible)
            .field("selection", &self.selection.ranges)
            .finish_non_exhaustive()
    }
}

impl Default for PdfState {
    fn default() -> Self {
        PdfState::new("pdf")
    }
}

impl PdfState {
    /// An empty preview keyed `key`: its parts are keyed `{key}-…`.
    pub fn new(key: impl Into<String>) -> Self {
        PdfState {
            key: key.into(),
            doc: PdfDocument::new(0, [612.0, 792.0]),
            content: Content::Pages,
            queue: Queue::default(),
            level: 1.0,
            zoom_gen: 0,
            scale: 1.0,
            viewport: None,
            rects: BTreeMap::new(),
            visible: 0..0,
            forward: true,
            current: 0,
            selection: Selection::default(),
            gesture: Gesture::Idle,
            over_text: false,
            find: Find::default(),
            searchable: false,
            anchor: None,
            reveal: None,
            clock: 0.0,
            settle_at: None,
            wake: 0,
            idle: true,
            level_request: None,
            pointer: None,
            feed: ScrollFeed::new(),
            bar: Bar::new(),
            sideways: Bar::new(),
        }
    }

    /// Shows `doc` from its first page at fitted width, dropping what the last document held.
    /// Replies to requests from before are dropped, and requests still in flight are
    /// cancelled ([`PdfState::take_cancelled`]).
    pub fn open(&mut self, doc: PdfDocument) {
        self.queue.reset(doc.pages());
        self.doc = doc;
        self.content = Content::Pages;
        self.level = 1.0;
        self.zoom_gen = self.zoom_gen.wrapping_add(1);
        self.rects.clear();
        self.visible = 0..0;
        self.current = 0;
        self.selection.clear();
        self.gesture = Gesture::Idle;
        self.find.clear();
        self.anchor = None;
        self.reveal = None;
        self.settle_at = None;
        self.idle = true;
        // A new document opens at fitted width, as Strata's viewer does.
        self.level_request = None;
    }

    /// [`PdfState::open`], with the page the preview opened on already rendered: Strata loads
    /// a PDF by rendering its first page. A [`PageReply::Failed`] here is dropped and the page
    /// asked for once more through [`PdfState::pop`] (Strata fails the whole preview when its
    /// first page doesn't render: a host doing the same calls [`PdfState::failed`]).
    pub fn open_with(&mut self, doc: PdfDocument, page: usize, reply: PageReply) {
        self.open(doc);
        let ticket = Ticket {
            generation: self.queue.generation,
            serial: 0,
            page: page as u32,
            kind: RenderKind::Page,
        };
        self.complete(ticket, reply);
    }

    /// Shows "Preview unavailable" and "This PDF is password protected." with an Open button,
    /// as Strata does for a PDF that needs an opening password: the renderer can't take one.
    pub fn locked(&mut self) {
        self.open(PdfDocument::new(0, [1.0, 1.0]));
        self.content = Content::Locked;
    }

    /// Shows "Preview unavailable" and `message`, with no Open button (a damaged PDF).
    pub fn failed(&mut self, message: impl Into<String>) {
        self.open(PdfDocument::new(0, [1.0, 1.0]));
        self.content = Content::Failed(message.into());
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    pub fn document(&self) -> &PdfDocument {
        &self.doc
    }

    /// Whether the drawer offers find over the text layer, and find asks for the text of pages
    /// it hasn't seen. Off unless set: Strata's find doesn't search PDF pages
    /// (`preview/preview-panel/find`).
    pub fn set_searchable(&mut self, on: bool) {
        self.searchable = on;
        if !on {
            self.find.clear();
        }
    }

    pub fn set_caps(&mut self, caps: RenderCaps) {
        self.queue.caps = caps;
    }

    /// The byte budgets for images (RGBA) and text layers. Defaults:
    /// [`IMAGE_BUDGET`](super::IMAGE_BUDGET) and [`TEXT_BUDGET`](super::TEXT_BUDGET).
    pub fn set_budgets(&mut self, images: usize, text: usize) {
        self.queue.image_budget = images;
        self.queue.text_budget = text;
    }

    /// Bytes held: images, then text layers.
    pub fn held_bytes(&self) -> (usize, usize) {
        (self.queue.image_bytes(), self.queue.text_bytes())
    }

    /// The zoom level: 1 is fitted width, up to 4.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// The page under the viewport's centre, from 0.
    pub fn current_page(&self) -> usize {
        self.current
    }

    /// The pages Rune reports visible.
    pub fn visible(&self) -> Range<usize> {
        self.visible.clone()
    }

    /// What Ctrl+C would copy now.
    pub fn selected_text(&self) -> String {
        self.selection.text(|p| self.layer(p))
    }

    pub fn has_selection(&self) -> bool {
        !self.selection.is_empty()
    }

    /// The image page `page` holds: its pixel size.
    pub fn held(&self, page: usize) -> Option<[u32; 2]> {
        self.slot_held(page).map(|h| [h.width, h.height])
    }

    fn slot_held(&self, page: usize) -> Option<Held> {
        self.queue.slots.get(page).and_then(|s| s.held)
    }

    pub(crate) fn layer(&self, page: usize) -> Option<&Shared> {
        self.queue.slots.get(page).and_then(|s| s.layer.as_ref())
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    pub(crate) fn scroll_key(&self) -> String {
        format!("{}-scroll", self.key)
    }

    pub(crate) fn list_key(&self) -> String {
        format!("{}-pages", self.key)
    }

    /// Page `page`'s row key: it carries the zoom generation ([`PdfState::zoom_gen`]).
    pub(crate) fn row_key(&self, page: usize) -> String {
        format!("{}-z{}-p{page}", self.key, self.zoom_gen)
    }

    /// The drawn page width in points.
    pub(crate) fn page_width(&self) -> f32 {
        zoom::drawn_width(self.pane_width(), self.level)
    }

    pub(crate) fn pane_width(&self) -> f32 {
        self.viewport.map_or(800.0, |v| v[2])
    }

    pub(crate) fn page_height(&self, page: usize) -> f32 {
        zoom::drawn_height(self.page_width(), self.doc.size(page))
    }

    /// The height Rune places a row it hasn't laid out at: the default page's.
    pub(crate) fn row_estimate(&self) -> f32 {
        let n = self.doc.pages().max(1);
        let w = self.page_width();
        let sum: f32 = (0..n)
            .map(|p| zoom::drawn_height(w, self.doc.size(p)))
            .sum();
        (sum / n as f32).round() + GAP
    }

    /// The content's width: the pages and their gaps, or the pane.
    pub(crate) fn content_width(&self) -> f32 {
        (self.page_width() + 2.0 * GAP).max(self.pane_width())
    }

    fn want(&self) -> Want {
        let n = self.doc.pages();
        Want {
            visible: self.visible.start.min(n)..self.visible.end.min(n),
            centre: self.current,
            forward: self.forward,
            width_px: self.page_width() * self.scale,
            settled: self.settle_at.is_none(),
            text_lane: self.searchable && self.find.active(),
            search: self.find.generation,
        }
    }

    /// The next render to start, or none. Pop whenever the renderer is free: what's wanted
    /// most when popped (the visible pages from the centre out, then the next page, then, with
    /// find open, pages' text), so a page that left the view while queued is never asked for.
    pub fn pop(&mut self) -> Option<RenderRequest> {
        if self.content != Content::Pages || self.viewport.is_none() {
            return None;
        }
        let want = self.want();
        let next = self.queue.pop(&self.doc, &want);
        self.idle = next.is_none();
        next
    }

    /// Requests handed out that are no longer wanted: the host may stop them.
    pub fn take_cancelled(&mut self) -> Vec<Ticket> {
        self.queue.take_cancelled()
    }

    /// Whether a request handed out is still wanted.
    pub fn is_wanted(&self, ticket: Ticket) -> bool {
        self.queue.is_wanted(ticket)
    }

    /// Takes a render's reply. Return [`Landed::commits`] from the host's poll.
    pub fn complete(&mut self, ticket: Ticket, reply: PageReply) -> Landed {
        // Any answer to the request in flight counts the page for this query, even one that
        // changes nothing (a text-only reply with no text): else the page is asked again.
        let answered = self.queue.is_wanted(ticket);
        let landed = self.queue.complete(&mut self.doc, ticket, reply);
        let page = ticket.page();
        if (answered || landed != Landed::Dropped)
            && self.find.active()
            && let Some(slot) = self.queue.slots.get_mut(page)
        {
            slot.searched = self.find.generation;
        }
        if landed == Landed::Shown {
            let layer = self.layer(page).cloned();
            if self.find.add(page, layer.as_ref()) {
                self.select_hit();
            }
            // The selection keeps its offsets on a page whose text came back the same; a page
            // whose text changed drops its part.
            if let Some(&(_, end)) = self.selection.ranges.get(&page)
                && layer.as_ref().is_none_or(|l| end > l.len())
            {
                self.selection.ranges.remove(&page);
            }
        }
        let want = self.want();
        let ranges = &self.selection.ranges;
        let evicted = self.queue.enforce(&want, |p| ranges.contains_key(&p));
        if evicted && landed != Landed::Shown {
            return Landed::Shown;
        }
        landed
    }

    /// The host's clock, each frame (seconds). Ends the zoom's rest, after which visible pages
    /// render again at the new level. True when the view changed.
    pub fn tick(&mut self, now: f64) -> bool {
        self.clock = now;
        if self.anchor.is_some_and(|a| now - a.at > ANCHOR_WAIT) {
            // Its page never laid out (the jump was clamped, or something else scrolled).
            self.anchor = None;
        }
        if let Some(at) = self.settle_at
            && now + 1e-6 >= at
        {
            self.settle_at = None;
            let want = self.want();
            self.queue.reconcile(&self.doc, &want);
        }
        false
    }

    /// Applies what find asked for, from the host's poll: the find calls carry no `Tx`. True
    /// when it asked for anything.
    pub fn flush(&mut self, tx: &mut Tx) -> bool {
        let Some(r) = self.reveal else {
            return false;
        };
        if self.rects.contains_key(&r.page) && self.visible.contains(&r.page) {
            self.place_reveal(tx);
            return true;
        }
        if r.asked {
            // Mounted and not visible yet, or it never mounted: ask once more, then let go.
            self.reveal = None;
        } else {
            self.reveal = Some(Reveal { asked: true, ..r });
        }
        tx.scroll_into_view(self.row_key(r.page));
        true
    }

    /// Scrolls the revealed match to the viewport's centre: its page is laid out.
    pub(crate) fn place_reveal(&mut self, tx: &mut Tx) {
        let (Some(r), Some(vp)) = (self.reveal.take(), self.viewport) else {
            return;
        };
        let Some(&rect) = self.rects.get(&r.page) else {
            return;
        };
        tx.scroll_key_to(
            self.row_key(r.page),
            vp[3] / 2.0 - (GAP + r.at[1] * rect[3]),
        );
        if self.content_width() > vp[2] + 0.5 {
            let x = GAP
                + (self.content_width() - 2.0 * GAP - self.page_width()) / 2.0
                + r.at[0] * self.page_width();
            tx.scroll_to_x(self.scroll_key(), (x - vp[2] / 2.0).max(0.0));
        }
    }

    /// Selects from `from` to `to` (each a page and a character), over the pages whose layers
    /// the view holds: a host putting a selection back.
    pub fn select(&mut self, from: (usize, usize), to: (usize, usize)) {
        self.selection.clear();
        self.selection.anchor = Some(from);
        let q = &self.queue;
        self.selection.extend(
            |p| q.slots.get(p).and_then(|s| s.layer.as_ref()),
            to.0,
            to.1,
        );
    }

    /// Brings page `page` into view, the point `down` (0 to 1) of the way down it at the
    /// viewport's centre, from the host's next [`PdfState::flush`]: a host restoring a page.
    pub fn show_page(&mut self, page: usize, down: f32) {
        if page < self.doc.pages() {
            self.reveal = Some(Reveal {
                page,
                at: [0.5, down.clamp(0.0, 1.0)],
                asked: false,
            });
        }
    }

    /// Zooms to `level` (1 to 4), about the view's centre, as Ctrl+wheel does: a host restoring
    /// a zoom. It applies once the view has laid out again, where the pages are known; return
    /// true from the poll that calls it.
    pub fn set_level(&mut self, level: f32) {
        self.level_request = Some(level);
        self.wake = self.wake.wrapping_add(1);
    }

    /// From the wake element's layout: a zoom the host asked for.
    pub(crate) fn apply_level_request(&mut self, tx: &mut Tx) -> bool {
        match self.level_request.take() {
            Some(level) => self.zoom_to(level, tx),
            None => false,
        }
    }

    // ---- From the view's handlers ----

    /// Rune's visible rows. True when the view should run again (requests are waiting, or a
    /// page dropped its image).
    pub(crate) fn set_visible(&mut self, r: Range<usize>) -> bool {
        if r != self.visible {
            if r.start != self.visible.start {
                self.forward = r.start > self.visible.start;
            }
            self.visible = r;
        }
        let want = self.want();
        let retried = self.queue.reconcile(&self.doc, &want);
        let ranges = &self.selection.ranges;
        let evicted = self.queue.enforce(&want, |p| ranges.contains_key(&p));
        let woke = self.wake_if_idle();
        woke || evicted || retried
    }

    /// Mounts the element that wakes the host when work is waiting and the host may be at
    /// rest. True when it did (the view runs again).
    fn wake_if_idle(&mut self) -> bool {
        if !self.idle || !self.queue.has_work(&self.doc, &self.want()) {
            return false;
        }
        self.idle = false;
        self.wake = self.wake.wrapping_add(1);
        true
    }

    /// The viewport's layout. A new width refits the pages, keeping the centre. The `Tx` asks
    /// the scroll; true when the view reads something new.
    pub(crate) fn set_viewport(&mut self, rect: [f32; 4], scale: f32, tx: &mut Tx) -> bool {
        let old = self.viewport;
        self.scale = scale;
        let widened = old.is_some_and(|o| (o[2] - rect[2]).abs() > 0.25);
        if widened {
            let anchor = self.pending_anchor().or_else(|| self.anchor_now());
            self.viewport = Some(rect);
            self.rezoom(self.level, anchor, tx);
            return true;
        }
        self.viewport = Some(rect);
        if old.is_none() {
            // The rows mount now, at the pane's width.
            self.zoom_gen = self.zoom_gen.wrapping_add(1);
            self.wake = self.wake.wrapping_add(1);
            return true;
        }
        old.is_some_and(|o| (o[3] - rect[3]).abs() > 0.25)
    }

    /// A page's layout, from the row built at zoom generation `zoom_gen`. Places a zoom's
    /// anchor or a find's match once its page is laid out, and turns the I-beam on or off for
    /// text moving under a still pointer. True when the page under the centre changed
    /// (assistive technology reads it) or the cursor did.
    ///
    /// A row of an earlier generation can still report its layout after a zoom (its handler
    /// runs on the next turn): that rect is the old size, and is ignored.
    pub(crate) fn page_laid_out(
        &mut self,
        page: usize,
        zoom_gen: u32,
        rect: [f32; 4],
        tx: &mut Tx,
    ) -> bool {
        if zoom_gen != self.zoom_gen {
            return false;
        }
        self.rects.insert(page, rect);
        if let (Some(a), Some(vp)) = (self.anchor, self.viewport)
            && a.page == page
        {
            if a.placed {
                // Laid out again after the placing scroll: the anchor has landed.
                self.anchor = None;
            } else {
                self.anchor = Some(Anchor { placed: true, ..a });
                tx.scroll_key_to(self.row_key(page), vp[3] / 2.0 - (GAP + a.down * rect[3]));
            }
        }
        if self.reveal.is_some_and(|r| r.page == page) {
            self.place_reveal(tx);
        }
        // Text scrolled under a still pointer, or away from it: the I-beam follows.
        let cursor = self.pointer.is_some_and(|p| self.hover(p));
        let Some(vp) = self.viewport else {
            return cursor;
        };
        let cy = vp[1] + vp[3] / 2.0;
        if rect[1] - GAP <= cy && cy < rect[1] + rect[3] && self.current != page {
            self.current = page;
            return true;
        }
        cursor
    }

    /// The visible pages laid out, with their rects.
    ///
    /// Rune reports layouts as queued actions, handled after the host's poll and before the
    /// frame's input: a handler for input sees where the last frame put the pages, while the
    /// poll can run before the last scroll's reports. So the poll's zoom waits for a layout
    /// report of its own ([`PdfState::set_level`]).
    fn shown(&self) -> impl Iterator<Item = (usize, [f32; 4])> + '_ {
        self.rects
            .range(self.visible.clone())
            .map(|(&p, &r)| (p, r))
    }

    /// The last zoom's anchor, while it waits for its page.
    fn pending_anchor(&self) -> Option<Anchor> {
        self.anchor.filter(|a| self.clock - a.at <= ANCHOR_WAIT)
    }

    /// The point under the viewport's centre, from the pages laid out last.
    fn anchor_now(&self) -> Option<Anchor> {
        let vp = self.viewport?;
        let cy = vp[1] + vp[3] / 2.0;
        let shown: Vec<(usize, [f32; 4])> = self.shown().collect();
        let (page, rect) = shown
            .iter()
            .find(|(_, r)| r[1] - GAP <= cy && cy < r[1] + r[3])
            .or_else(|| shown.iter().find(|(_, r)| cy < r[1]))
            .copied()?;
        let down = ((cy - rect[1]) / rect[3].max(1.0)).clamp(0.0, 1.0);
        let off = self.feed.get().offset[0];
        let across = ((off + vp[2] / 2.0) / self.content_width()).clamp(0.0, 1.0);
        Some(Anchor {
            page,
            down,
            across,
            at: self.clock,
            placed: false,
        })
    }

    /// Zooms by Ctrl+wheel `dy` (a wheel turned away zooms in), about the view's centre.
    pub(crate) fn zoom_wheel(&mut self, dy: f32, tx: &mut Tx) -> bool {
        let next = zoom::after_wheel(self.level, dy);
        self.zoom_to(next, tx)
    }

    /// Ctrl+0: back to fitted width on the same page. Already there, nothing moves.
    pub(crate) fn reset_zoom(&mut self, tx: &mut Tx) -> bool {
        self.zoom_to(LIMITS.min, tx)
    }

    fn zoom_to(&mut self, level: f32, tx: &mut Tx) -> bool {
        let level = LIMITS.clamp(level);
        if (level - self.level).abs() < 1e-4 || self.content != Content::Pages {
            return false;
        }
        // A notch on the frame after the last one: that one's anchor hasn't landed yet, and the
        // pages on screen are still where its estimate put them, not where it will.
        let anchor = self.pending_anchor().or_else(|| self.anchor_now());
        self.rezoom(level, anchor, tx);
        self.settle_at = Some(self.clock + SETTLE);
        tx.wake_after(Duration::from_secs_f64(SETTLE));
        true
    }

    /// Every page changes size: the rows take new keys, the view jumps to where the anchor
    /// lands if every row above were the estimate, and the anchor's page, once laid out,
    /// places itself exactly ([`PdfState::page_laid_out`]). Rune can only place a row it has
    /// laid out, so the jump comes first (probe P1).
    fn rezoom(&mut self, level: f32, anchor: Option<Anchor>, tx: &mut Tx) {
        self.level = level;
        self.zoom_gen = self.zoom_gen.wrapping_add(1);
        self.rects.clear();
        let (Some(a), Some(vp)) = (anchor, self.viewport) else {
            return;
        };
        let y = a.page as f32 * self.row_estimate() + GAP + a.down * self.page_height(a.page)
            - vp[3] / 2.0;
        let x = a.across * self.content_width() - vp[2] / 2.0;
        tx.scroll_to_instant(self.scroll_key(), x.max(0.0), y.max(0.0));
        self.anchor = Some(Anchor {
            at: self.clock,
            placed: false,
            ..a
        });
    }

    /// The text under window point `p`: a visible page, and the point in its layer's pixels.
    pub(crate) fn text_at(&self, p: [f32; 2]) -> Option<(usize, f32, f32)> {
        self.shown().find_map(|(page, r)| {
            let layer = self.layer(page)?;
            let (x, y) = to_layer(layer, &r, p);
            let inside = (0.0..=layer.width).contains(&x) && (0.0..=layer.height).contains(&y);
            (inside && layer.hit_text(x, y)).then_some((page, x, y))
        })
    }

    /// The nearest point on any visible page with text to window point `p`.
    fn text_near(&self, p: [f32; 2]) -> Option<(usize, f32, f32)> {
        let mut best: Option<((usize, f32, f32), f32)> = None;
        for (page, r) in self.shown() {
            let Some(layer) = self.layer(page) else {
                continue;
            };
            let (x, y) = to_layer(layer, &r, p);
            let (cx, cy) = (x.clamp(0.0, layer.width), y.clamp(0.0, layer.height));
            let k = r[2] / layer.width;
            let d = ((cx - x) * k).powi(2) + ((cy - y) * k).powi(2);
            if best.is_none_or(|(_, b)| d < b) {
                best = Some(((page, cx, cy), d));
            }
        }
        best.map(|(at, _)| at)
    }

    /// A primary drag on the pages: over text it selects, elsewhere it pans. True when the
    /// view reads something new.
    pub(crate) fn drag(&mut self, d: &Drag, shift: bool, tx: &mut Tx) -> bool {
        let p = d.window_point();
        match d.phase {
            Phase::Down => {
                if let Some((page, x, y)) = self.text_at(p) {
                    self.gesture = Gesture::Select;
                    let Some(layer) = self.layer(page).cloned() else {
                        return false;
                    };
                    let caret = layer.caret_at(x, y);
                    if shift && self.selection.anchor.is_some() {
                        self.selection.unit = Unit::Char;
                        let q = &self.queue;
                        self.selection.extend(
                            |p| q.slots.get(p).and_then(|s| s.layer.as_ref()),
                            page,
                            caret,
                        );
                    } else {
                        self.selection
                            .press(&layer, page, caret, Unit::of_clicks(d.clicks));
                    }
                    return true;
                }
                self.gesture = Gesture::Pan(self.feed.get().offset);
                self.selection.clear();
                true
            }
            Phase::Move => match self.gesture {
                Gesture::Select => {
                    let Some((page, x, y)) = self.text_near(p) else {
                        return false;
                    };
                    let Some(layer) = self.layer(page) else {
                        return false;
                    };
                    let caret = layer.caret_at(x, y);
                    let q = &self.queue;
                    self.selection.extend(
                        |p| q.slots.get(p).and_then(|s| s.layer.as_ref()),
                        page,
                        caret,
                    )
                }
                Gesture::Pan(o) => {
                    tx.scroll_to_instant(self.scroll_key(), o[0] - d.dx, o[1] - d.dy);
                    false
                }
                Gesture::Idle => false,
            },
            Phase::Up => {
                let was = self.gesture;
                self.gesture = Gesture::Idle;
                let cursor = self.hover(p);
                matches!(was, Gesture::Pan(_)) || cursor
            }
        }
    }

    /// The pointer left the view: text scrolling under where it was no longer moves the I-beam.
    pub(crate) fn leave(&mut self) {
        self.pointer = None;
    }

    /// The pointer moved over the view: true when the I-beam turns on or off.
    pub(crate) fn hover(&mut self, p: [f32; 2]) -> bool {
        self.pointer = Some(p);
        if self.gesture != Gesture::Idle {
            // A drag shows its own cursor; the I-beam is worked out again on release.
            return false;
        }
        let over = self.text_at(p).is_some();
        if over == self.over_text {
            return false;
        }
        self.over_text = over;
        true
    }

    /// Ctrl+A: the text of every page whose layer the view holds. False when none has text,
    /// so the key goes on (Strata's `select_all`).
    pub(crate) fn select_all(&mut self) -> bool {
        let q = &self.queue;
        let layers = q
            .slots
            .iter()
            .enumerate()
            .filter_map(|(p, s)| s.layer.as_ref().map(|l| (p, l)));
        self.selection.all(layers);
        !self.selection.is_empty()
    }

    fn select_hit(&mut self) {
        let Some(h) = self.find.current_hit() else {
            return;
        };
        self.selection.clear();
        self.selection.anchor = Some((h.page, h.start));
        self.selection.ranges.insert(h.page, (h.start, h.end));
        let at = self.layer(h.page).and_then(|l| {
            let runs = l.selection_runs(h.start, h.end);
            let r = runs.first()?;
            Some([
                (r[0] + r[2]) / 2.0 / l.width,
                (r[1] + r[3]) / 2.0 / l.height,
            ])
        });
        self.reveal = Some(Reveal {
            page: h.page,
            at: at.unwrap_or([0.5, 0.0]),
            asked: false,
        });
    }

    pub(crate) fn hits_on(&self, page: usize) -> &[Hit] {
        if self.find.active() {
            self.find.on_page(page)
        } else {
            &[]
        }
    }

    /// The pages whose layers find searches.
    fn layers(&self) -> impl Iterator<Item = (usize, &Shared)> {
        self.queue
            .slots
            .iter()
            .enumerate()
            .filter_map(|(p, s)| s.layer.as_ref().map(|l| (p, l)))
    }

    /// Whether the content overflows sideways and isn't at its left edge.
    pub(crate) fn can_pan_left(&self) -> bool {
        self.feed.get().offset[0] > 0.5
    }

    pub(crate) fn can_pan_right(&self) -> bool {
        let p = self.feed.get();
        p.offset[0] < p.max[0] - 0.5
    }
}

fn to_layer(layer: &Shared, r: &[f32; 4], p: [f32; 2]) -> (f32, f32) {
    let kx = layer.width / r[2].max(1.0);
    let ky = layer.height / r[3].max(1.0);
    ((p[0] - r[0]) * kx, (p[1] - r[1]) * ky)
}

impl HostedPreview for PdfState {
    fn searchable(&self) -> bool {
        self.searchable && self.content == Content::Pages && self.doc.pages() > 0
    }

    fn find(&mut self, query: &str) {
        if !self.searchable {
            return;
        }
        // Each edit to the query clears the selection; the first match then takes it.
        self.selection.clear();
        self.reveal = None;
        let mut find = std::mem::take(&mut self.find);
        find.run(query, self.layers());
        self.find = find;
        if self.find.active() {
            let generation = self.find.generation;
            for s in self.queue.slots.iter_mut().filter(|s| s.layer.is_some()) {
                s.searched = generation;
            }
        }
        self.select_hit();
        let want = self.want();
        self.queue.reconcile(&self.doc, &want);
        self.wake_if_idle();
    }

    fn find_step(&mut self, forward: bool) {
        if self.find.step(forward).is_some() {
            self.select_hit();
        }
    }

    fn find_status(&self) -> Option<FindStatus> {
        self.searchable.then(|| FindStatus {
            current: self.find.current.map(|c| c + 1),
            total: self.find.hits.len(),
        })
    }

    fn find_end(&mut self) {
        // The selected match stays selected for Ctrl+C.
        self.find.clear();
        self.reveal = None;
        let want = self.want();
        self.queue.reconcile(&self.doc, &want);
    }
}
