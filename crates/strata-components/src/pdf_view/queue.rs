//! The pages the view holds (an image, a text layer, a request in flight) and the requests it
//! asks the host for, under two byte budgets.
//!
//! The host pulls: it pops a request whenever its renderer is free, so Strata's one page at a
//! time across previews is the host's to keep. A request is ranked when popped, from where the
//! view stands then, so a page that has left the view by the time the renderer is free is never
//! asked for: Strata's "queued pages that leave the view are cancelled before rendering". A
//! request already handed out for a page no longer wanted goes to
//! [`Queue::take_cancelled`].

use std::ops::Range;
use std::rc::Rc;

use rune::Pixels;

use super::layer::Layer;
use super::model::{PageReply, PdfDocument, RenderCaps, RenderKind, RenderRequest, Shared, Ticket};

/// Default image budget: 192 MiB of RGBA.
pub const IMAGE_BUDGET: usize = 192 << 20;
/// Default text-layer budget: 32 MiB.
pub const TEXT_BUDGET: usize = 32 << 20;

/// A page's held image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Held {
    pub ticket: Ticket,
    pub width: u32,
    pub height: u32,
}

impl Held {
    fn bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Inflight {
    pub ticket: Ticket,
    pub width: u32,
    pub kind: RenderKind,
}

#[derive(Debug, Default)]
pub(crate) struct Slot {
    /// Made once the page first has an image, then filled in place.
    pub pixels: Option<Pixels>,
    pub held: Option<Held>,
    pub layer: Option<Shared>,
    /// Whether `layer` came with the image the page shows (else from a text-only render).
    pub layer_from_image: bool,
    /// A reply said whether the page has text.
    pub text_known: bool,
    pub inflight: Option<Inflight>,
    pub failed: bool,
    /// A failed page that has since left the view: it asks once more when it comes back, as
    /// Strata's rows ask again when bound again.
    pub failed_away: bool,
    /// The find run ([`Want::search`]) that has counted this page's text.
    pub searched: u64,
}

impl Slot {
    /// An image the page shows, with no text layer to select on: the budget dropped it. The
    /// page asks for a `Page` render again, so its layer comes with the image it draws on.
    fn lost_text(&self) -> bool {
        self.held.is_some() && self.layer.is_none() && !self.text_known
    }
}

/// Where the view stands, for ranking requests.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Want {
    pub visible: Range<usize>,
    /// The page under the viewport's centre.
    pub centre: usize,
    /// Scrolling towards the end (the margin page is the one after the visible pages).
    pub forward: bool,
    /// The drawn width of a page in device pixels.
    pub width_px: f32,
    /// The zoom has rested: a page holding an image at another width may ask again.
    pub settled: bool,
    /// Find is open: pages with no text yet are asked for theirs.
    pub text_lane: bool,
    /// The find run whose count pages join: a page counted by it isn't asked again.
    pub search: u64,
}

/// What [`Queue::complete`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landed {
    /// Taken, and the view shows something new (a first image, a layer, a failure): commit.
    Shown,
    /// Taken in place: a sharper image of a page that had one draws without a commit.
    Replaced,
    /// From an earlier document, or older than what the page holds: dropped.
    Dropped,
}

impl Landed {
    /// Whether the host's poll should return true.
    pub fn commits(self) -> bool {
        self == Landed::Shown
    }
}

#[derive(Debug)]
pub(crate) struct Queue {
    pub generation: u32,
    serial: u64,
    pub slots: Vec<Slot>,
    cancelled: Vec<Ticket>,
    pub caps: RenderCaps,
    pub image_budget: usize,
    pub text_budget: usize,
    image_bytes: usize,
    text_bytes: usize,
}

impl Default for Queue {
    fn default() -> Self {
        Queue {
            generation: 0,
            serial: 0,
            slots: Vec::new(),
            cancelled: Vec::new(),
            caps: RenderCaps::default(),
            image_budget: IMAGE_BUDGET,
            text_budget: TEXT_BUDGET,
            image_bytes: 0,
            text_bytes: 0,
        }
    }
}

impl Queue {
    /// Starts over for a document of `pages` pages: every ticket handed out before is stale,
    /// and every one still in flight is cancelled.
    pub fn reset(&mut self, pages: usize) {
        for s in &self.slots {
            if let Some(f) = s.inflight {
                self.cancelled.push(f.ticket);
            }
        }
        self.generation = self.generation.wrapping_add(1);
        self.slots = (0..pages).map(|_| Slot::default()).collect();
        self.image_bytes = 0;
        self.text_bytes = 0;
    }

    pub fn take_cancelled(&mut self) -> Vec<Ticket> {
        std::mem::take(&mut self.cancelled)
    }

    pub fn image_bytes(&self) -> usize {
        self.image_bytes
    }

    pub fn text_bytes(&self) -> usize {
        self.text_bytes
    }

    pub fn is_wanted(&self, ticket: Ticket) -> bool {
        ticket.generation == self.generation
            && self
                .slots
                .get(ticket.page as usize)
                .and_then(|s| s.inflight)
                .is_some_and(|f| f.ticket == ticket)
    }

    /// The width page `page` is worth rendering at.
    fn width_for(&self, doc: &PdfDocument, page: usize, want: &Want) -> u32 {
        let size = doc.size(page);
        self.caps.width_for(want.width_px.ceil(), size[1] / size[0])
    }

    /// Whether `page` needs an image request now, at `w` pixels.
    fn needs_image(&self, page: usize, w: u32, want: &Want) -> bool {
        let s = &self.slots[page];
        if s.failed {
            return false;
        }
        if let Some(f) = s.inflight
            && f.kind == RenderKind::Page
            && (f.width == w || !want.settled)
        {
            return false;
        }
        match s.held {
            None => true,
            Some(_) if s.lost_text() => true,
            Some(h) => {
                want.settled
                    && ((h.width as f32) < w as f32 * 0.98 || h.width as f32 > w as f32 * 1.5)
            }
        }
    }

    /// The pages wanted now, best first: the visible pages from the centre out, then the
    /// margin page, then (with find open) the pages whose text isn't known, from the centre
    /// out.
    fn ranked(&self, doc: &PdfDocument, want: &Want) -> Vec<(usize, u32, RenderKind)> {
        let n = self.slots.len();
        let mut out = Vec::new();
        if n == 0 {
            return out;
        }
        let vis = want.visible.start.min(n)..want.visible.end.min(n);
        let centre = want
            .centre
            .clamp(vis.start, vis.end.saturating_sub(1).max(vis.start));
        let mut order: Vec<usize> = vis.clone().collect();
        order.sort_by_key(|&p| p.abs_diff(centre));
        for p in order {
            let w = self.width_for(doc, p, want);
            if self.needs_image(p, w, want) {
                out.push((p, w, RenderKind::Page));
            }
        }
        let margin = if want.forward {
            Some(vis.end).filter(|&p| p < n)
        } else {
            vis.start.checked_sub(1)
        };
        if let Some(p) = margin {
            let s = &self.slots[p];
            if (s.held.is_none() || s.lost_text()) && !s.failed && s.inflight.is_none() {
                out.push((p, self.width_for(doc, p, want), RenderKind::Page));
            }
        }
        if want.text_lane {
            for d in 0..n {
                for p in [centre.checked_sub(d), (d > 0).then_some(centre + d)]
                    .into_iter()
                    .flatten()
                    .filter(|&p| p < n)
                {
                    let s = &self.slots[p];
                    // A page whose text is known (a scan: the renderer sends no layer) has
                    // nothing to count; one whose layer the budget dropped is asked again.
                    if s.layer.is_none()
                        && !s.text_known
                        && s.searched != want.search
                        && !s.failed
                        && s.inflight.is_none()
                    {
                        out.push((p, self.width_for(doc, p, want), RenderKind::TextOnly));
                    }
                }
            }
        }
        out
    }

    /// Cancels requests in flight for pages no longer wanted: off the view and its margin
    /// (and not asked for their text), or asked at a width the rested zoom no longer wants;
    /// and lets a failed page back in view ask once more. True when either happened.
    pub fn reconcile(&mut self, doc: &PdfDocument, want: &Want) -> bool {
        let n = self.slots.len();
        let vis = want.visible.start.min(n)..want.visible.end.min(n);
        let margin = if want.forward {
            vis.end
        } else {
            vis.start.wrapping_sub(1)
        };
        let mut any = false;
        for p in 0..n {
            let near = vis.contains(&p) || p == margin;
            let s = &mut self.slots[p];
            if s.failed && !near {
                s.failed_away = true;
            } else if s.failed && s.failed_away && near {
                // Back in view after a failure: ask once more.
                s.failed = false;
                s.failed_away = false;
                any = true;
            }
            let Some(f) = self.slots[p].inflight else {
                continue;
            };
            let keep = match f.kind {
                RenderKind::Page if vis.contains(&p) => {
                    !want.settled || f.width == self.width_for(doc, p, want)
                }
                RenderKind::Page => near,
                RenderKind::TextOnly => want.text_lane || near,
            };
            if !keep {
                self.cancelled.push(f.ticket);
                self.slots[p].inflight = None;
                any = true;
            }
        }
        any
    }

    /// The next request, or none: what's wanted most and not yet in flight.
    pub fn pop(&mut self, doc: &PdfDocument, want: &Want) -> Option<RenderRequest> {
        let (page, width, kind) = self.ranked(doc, want).into_iter().next()?;
        if let Some(old) = self.slots[page].inflight.take() {
            // A page asked again at a new width: the old request is cancelled.
            self.cancelled.push(old.ticket);
        }
        self.serial += 1;
        let ticket = Ticket {
            generation: self.generation,
            serial: self.serial,
            page: page as u32,
            kind,
        };
        self.slots[page].inflight = Some(Inflight {
            ticket,
            width,
            kind,
        });
        Some(RenderRequest {
            ticket,
            page,
            box_px: [width, self.caps.max_height],
            kind,
        })
    }

    /// Whether any request is wanted now (so the host should be woken to pop).
    pub fn has_work(&self, doc: &PdfDocument, want: &Want) -> bool {
        !self.ranked(doc, want).is_empty()
    }

    /// Takes a reply. A reply for an earlier document, or older than the image the page holds,
    /// is dropped; a text-only reply never replaces an image.
    pub fn complete(&mut self, doc: &mut PdfDocument, ticket: Ticket, reply: PageReply) -> Landed {
        let page = ticket.page as usize;
        if ticket.generation != self.generation || page >= self.slots.len() {
            return Landed::Dropped;
        }
        let slot = &mut self.slots[page];
        let wanted = slot.inflight.is_some_and(|f| f.ticket == ticket);
        if wanted {
            slot.inflight = None;
        }
        let text_only = ticket.kind == RenderKind::TextOnly;
        match reply {
            PageReply::Failed => {
                // A render the view no longer wanted (cancelled, or asked again since) says
                // nothing about the page: a host may report a stopped render as a failure.
                if !wanted || slot.held.is_some_and(|h| h.ticket > ticket) {
                    return Landed::Dropped;
                }
                slot.text_known = true;
                if text_only {
                    return Landed::Shown;
                }
                slot.failed = true;
                Landed::Shown
            }
            PageReply::Rendered {
                width,
                height,
                rgba,
                text,
            } => {
                let newer_image = slot.held.is_none_or(|h| h.ticket < ticket);
                let take_image = newer_image
                    && (!text_only || slot.held.is_none())
                    && width > 0
                    && height > 0
                    && rgba.len() == width as usize * height as usize * 4;
                let mut shown = false;
                if take_image {
                    let had = slot.held.is_some();
                    let held = Held {
                        ticket,
                        width,
                        height,
                    };
                    self.image_bytes =
                        self.image_bytes - slot.held.map_or(0, |h| h.bytes()) + held.bytes();
                    slot.pixels
                        .get_or_insert_with(Pixels::new)
                        .set(width, height, rgba);
                    slot.held = Some(held);
                    slot.failed = false;
                    shown |= !had;
                    shown |= doc.set_size(page, [width as f32, height as f32]);
                }
                // A layer comes with the image it was rendered with; a text-only render's,
                // or an older render's, only fills a page that has none.
                if take_image || slot.layer.is_none() {
                    let new: Option<Shared> = text.map(|t| Rc::new(Layer::new(t)));
                    let same = match (&slot.layer, &new) {
                        (Some(a), Some(b)) => a.text == b.text,
                        (None, None) => true,
                        _ => false,
                    };
                    self.text_bytes -= slot.layer.as_ref().map_or(0, |l| l.bytes());
                    self.text_bytes += new.as_ref().map_or(0, |l| l.bytes());
                    shown |= !same || slot.layer_from_image != take_image;
                    slot.layer = new;
                    slot.layer_from_image = take_image;
                }
                slot.text_known = true;
                if shown {
                    Landed::Shown
                } else if take_image {
                    Landed::Replaced
                } else {
                    Landed::Dropped
                }
            }
        }
    }

    /// Drops what's over budget, farthest from the centre first, never the visible pages or
    /// the margin, never a layer `keep_layer` keeps (a selected page's). True when a mounted
    /// page has to show its placeholder again.
    ///
    /// Find's count outlives a dropped layer: its matches stay counted, and the page isn't asked
    /// for its text again while the same query runs ([`Slot::searched`]).
    pub fn enforce(&mut self, want: &Want, keep_layer: impl Fn(usize) -> bool) -> bool {
        let n = self.slots.len();
        let vis = want.visible.start.min(n)..want.visible.end.min(n);
        let near = |p: usize| vis.contains(&p) || p + 1 == vis.start || p == vis.end;
        let mut changed = false;
        if self.image_bytes > self.image_budget {
            let mut held: Vec<usize> = (0..n)
                .filter(|&p| self.slots[p].held.is_some() && !near(p))
                .collect();
            held.sort_by_key(|&p| std::cmp::Reverse(p.abs_diff(want.centre)));
            for p in held {
                if self.image_bytes <= self.image_budget {
                    break;
                }
                let s = &mut self.slots[p];
                if let Some(h) = s.held.take() {
                    self.image_bytes -= h.bytes();
                    // Frees the pixels now: a mounted row may still draw the handle.
                    if let Some(px) = &s.pixels {
                        px.set(1, 1, vec![0; 4]);
                    }
                    changed = true;
                }
            }
        }
        if self.text_bytes > self.text_budget {
            let mut held: Vec<usize> = (0..n)
                .filter(|&p| self.slots[p].layer.is_some() && !near(p) && !keep_layer(p))
                .collect();
            held.sort_by_key(|&p| std::cmp::Reverse(p.abs_diff(want.centre)));
            for p in held {
                if self.text_bytes <= self.text_budget {
                    break;
                }
                let s = &mut self.slots[p];
                if let Some(l) = s.layer.take() {
                    self.text_bytes -= l.bytes();
                    s.text_known = false;
                    s.layer_from_image = false;
                    changed = true;
                }
            }
        }
        changed
    }
}
