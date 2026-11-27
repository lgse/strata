// SPDX-License-Identifier: MIT

mod geometry;

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use rune::{Color, Cursor, El, Live, Phase, ScrollPos, Spring, col};
use strata_kit::look::Look;
use strata_kit::slots::scroll as tokens;

pub use geometry::{offset_at, thumb};

pub const HOLD: Duration = Duration::from_secs(1);

/// The fade in and out: Strata's `transition: opacity 180ms ease-out`.
const FADE: Spring = Spring::new(0.18, 0.0);

/// The trough's change of strength under the pointer.
const TINT: Spring = Spring::new(0.15, 0.0);

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BarLook {
    /// Along the viewport's bottom edge, for one that scrolls sideways; else down its right.
    pub horizontal: bool,
    /// The trough's thickness.
    pub thickness: f32,
    /// Optional expanded trough thickness while hovered or dragged.
    pub hover_thickness: Option<f32>,
    /// The thumb's thickness, centred in the trough.
    pub thumb_width: f32,
    /// The thumb's least length.
    pub min_length: f32,
    /// The thumb's gap from the trough's ends.
    pub ends: f32,
    /// The bar's gap from the viewport edge it runs along (right, or bottom).
    pub edge: f32,
    /// The trough's gap from the viewport's top (or left).
    pub start: f32,
    /// A band of `base` this thick on each side of the trough, across it.
    pub margin: f32,
    /// The thumb's colour.
    pub thumb: Color,
    /// The trough's colour at rest, and under the pointer or while dragged.
    pub trough: Color,
    pub trough_hover: Color,
    /// An opaque band under the trough (the column strip's), if any.
    pub base: Option<Color>,
    /// A ring inside the trough, if any, and its width.
    pub ring: Option<Color>,
    pub ring_width: f32,
    pub thumb_radius: f32,
    pub trough_radius: f32,
    /// Whether the bar fades (`Look::animations`); off, it shows and hides at once.
    pub animations: bool,
}

impl BarLook {
    pub fn app(look: &Look) -> Self {
        let w = tokens::bar_width(look);
        Self {
            horizontal: false,
            thickness: w,
            hover_thickness: tokens::hover_width(look),
            thumb_width: w,
            min_length: tokens::thumb_min_length(look),
            ends: 0.0,
            edge: 0.0,
            start: 0.0,
            margin: 0.0,
            thumb: tokens::thumb_color(look),
            trough: tokens::trough_at(look, tokens::trough_opacity(look)),
            trough_hover: tokens::trough_at(look, tokens::trough_hover_opacity(look)),
            base: None,
            ring: None,
            ring_width: tokens::strip_ring_width(look),
            thumb_radius: tokens::thumb_radius(look),
            trough_radius: tokens::trough_radius(look),
            animations: look.animations,
        }
    }

    pub fn listing(look: &Look) -> Self {
        let thumb_width = tokens::listing_thumb_width(look);
        Self {
            thickness: tokens::listing_width(look),
            thumb_width,
            thumb_radius: thumb_width / 2.0,
            ends: tokens::listing_end_inset(look),
            trough: tokens::trough_at(look, tokens::listing_trough_opacity(look)),
            trough_hover: tokens::trough_at(look, tokens::listing_hover_opacity(look)),
            ..Self::app(look)
        }
    }

    /// The List listing's vertical bar: [`BarLook::listing`] with no gaps at its ends.
    pub fn list(look: &Look) -> Self {
        Self {
            ends: 0.0,
            ..Self::listing(look)
        }
    }

    pub fn column(look: &Look) -> Self {
        Self {
            edge: tokens::column_right_gap(look),
            start: tokens::column_top_gap(look),
            ..Self::app(look)
        }
    }

    pub fn strip(look: &Look) -> Self {
        Self {
            horizontal: true,
            margin: tokens::strip_margin(look),
            base: Some(look.kit.c.bg_base),
            ring: Some(tokens::trough_at(look, tokens::strip_ring_opacity(look))),
            ..Self::app(look)
        }
    }

    /// The same bar along the bottom edge, for a viewport that scrolls sideways.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// The index of the axis the bar runs along: 0 sideways, 1 down.
    fn along(&self) -> usize {
        usize::from(!self.horizontal)
    }

    /// The thumb's track: the trough less its end gaps.
    fn track(&self, trough: f32) -> f32 {
        trough - 2.0 * self.ends
    }

    /// The bar's cross-axis band thickness (`thickness + 2 * margin`).
    pub fn band(&self) -> f32 {
        self.thickness + 2.0 * self.margin
    }

    /// Expanded trough thickness when hover expansion is active and wider than rest.
    fn expanded_thickness(&self) -> f32 {
        self.hover_thickness
            .filter(|&w| w.is_finite() && w > self.thickness)
            .unwrap_or(self.thickness)
    }

    /// Effective `(thickness, thumb_width)` for rest vs. hovered/dragged state.
    fn active_widths(&self, held: bool) -> (f32, f32) {
        if !held {
            return (self.thickness, self.thumb_width);
        }
        let thickness = self.expanded_thickness();
        if thickness <= self.thickness {
            return (self.thickness, self.thumb_width);
        }
        let inset = (self.thickness - self.thumb_width).max(0.0);
        let thumb_width = (thickness - inset).max(self.thumb_width);
        (thickness, thumb_width)
    }

    fn turned(&self, [across, along, thick, long]: [f32; 4]) -> [f32; 4] {
        if self.horizontal {
            [along, across, long, thick]
        } else {
            [across, along, thick, long]
        }
    }
}

/// When an overlay bar shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BarMode {
    #[default]
    Overlay,
    /// Shown whenever the content overflows (a desktop that turns overlay scrolling off).
    Always,
}

struct BarRects {
    band: [f32; 4],
    backdrop: [f32; 4],
    trough: [f32; 4],
    thumb: [f32; 4],
}

struct Inner {
    look: Cell<Option<BarLook>>,
    mode: Cell<BarMode>,
    pos: Cell<ScrollPos>,
    outer: Cell<Option<ScrollPos>>,
    target: RefCell<String>,
    over: Cell<bool>,
    dragging: Cell<bool>,
    grab: Cell<f32>,
    band: Live<[f32; 4]>,
    backdrop: Live<[f32; 4]>,
    trough: Live<[f32; 4]>,
    thumb: Live<[f32; 4]>,
    fill: Live<Color>,
    opacity: Live<f32>,
}

#[derive(Clone)]
pub struct Bar(Rc<Inner>);

impl Default for Bar {
    fn default() -> Self {
        Self::new()
    }
}

impl Bar {
    /// An overlay bar that has heard nothing yet.
    pub fn new() -> Self {
        Self(Rc::new(Inner {
            look: Cell::new(None),
            mode: Cell::new(BarMode::Overlay),
            pos: Cell::new(ScrollPos::default()),
            outer: Cell::new(None),
            target: RefCell::new(String::new()),
            over: Cell::new(false),
            dragging: Cell::new(false),
            grab: Cell::new(0.0),
            band: Live::new([0.0; 4]),
            backdrop: Live::new([0.0; 4]),
            trough: Live::new([0.0; 4]),
            thumb: Live::new([0.0; 4]),
            fill: Live::new(Color::TRANSPARENT),
            opacity: Live::new(0.0),
        }))
    }

    /// Sets when it shows.
    pub fn set_mode(&self, mode: BarMode) {
        self.0.mode.set(mode);
        self.0.settle();
    }

    /// When it shows.
    pub fn mode(&self) -> BarMode {
        self.0.mode.get()
    }

    pub fn follow(&self, p: ScrollPos) {
        self.0.follow(p);
    }

    pub fn follow_across(&self, p: ScrollPos) {
        self.0.outer.set(Some(p));
        self.0.layout();
    }

    pub fn release(&self) {
        self.0.release();
    }

    /// A handler for `El::on_scroll_offset` that calls [`Bar::follow`].
    pub fn watch(&self) -> impl Fn(ScrollPos) + 'static {
        let bar = self.clone();
        move |p| bar.follow(p)
    }

    /// Whether the content overflows, so the bar can show.
    pub fn overflows(&self) -> bool {
        self.0.shows()
    }

    /// Whether the user is actively dragging this bar's thumb.
    pub fn is_dragging(&self) -> bool {
        self.0.dragging.get()
    }

    pub(crate) fn reveal(&self) {
        self.0.reveal();
    }

    /// The viewport's latest [`ScrollPos`].
    pub fn pos(&self) -> ScrollPos {
        self.0.pos.get()
    }

    #[cfg(test)]
    pub(crate) fn offset(&self) -> [f32; 2] {
        self.0.pos.get().offset
    }

    pub fn area(&self, listing: El) -> El {
        let bar = self.clone();
        col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .min_w(0.0)
            .on_pointer_move_any(move |_, tx| {
                tx.live_only();
                bar.0.reveal();
            })
            .child(listing)
    }

    pub fn build<S: 'static>(&self, key: &str, target: &str, look: BarLook) -> El {
        let inner = &self.0;
        *inner.target.borrow_mut() = target.to_owned();
        inner.restyle(look);
        let fade = if look.animations { FADE } else { Spring::NONE };
        let tint = if look.animations { TINT } else { Spring::NONE };
        let bind_rect = |live: &Live<[f32; 4]>| {
            if look.animations && look.hover_thickness.is_some() {
                live.smooth(tint)
            } else {
                live.into()
            }
        };
        let thumb = col()
            .key("thumb")
            .absolute(0.0, 0.0)
            .live_rect(bind_rect(&inner.thumb))
            .radius(look.thumb_radius)
            .fill(look.thumb);
        let mut trough = col()
            .key("trough")
            .absolute(0.0, 0.0)
            .live_rect(bind_rect(&inner.trough))
            .radius(look.trough_radius)
            .live_fill(inner.fill.smooth(tint))
            .child(thumb);
        if let Some(ring) = look.ring {
            trough = trough.outline(look.ring_width, -look.ring_width, ring);
        }
        let mut band = col()
            .key(key.to_owned())
            .absolute(0.0, 0.0)
            .live_rect(&inner.band)
            .live_opacity(inner.opacity.smooth(fade).rest(fade))
            .cursor(Cursor::Default)
            .a11y_hidden()
            .keeps_text_selection()
            .on_middle_press(|_: &mut S, tx: &mut rune::Tx| tx.live_only())
            .on_secondary_press(|_: &mut S, _, tx: &mut rune::Tx| tx.live_only());
        if let Some(base) = look.base {
            band = band.child(
                col()
                    .key("backdrop")
                    .absolute(0.0, 0.0)
                    .live_rect(bind_rect(&inner.backdrop))
                    .fill(base),
            );
        }
        let (hover, drag) = (self.clone(), self.clone());
        band.on_hover_change_any(move |inside, tx| {
            tx.live_only();
            hover.0.hover(inside);
        })
        .on_drag_any(move |d, tx| {
            tx.live_only();
            drag.0.drag(d, tx);
        })
        .child(trough)
    }
}

impl std::fmt::Debug for Bar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bar")
            .field("mode", &self.0.mode.get())
            .field("offset", &self.0.pos.get().offset)
            .finish()
    }
}

pub fn overlay(viewport: El, bars: impl IntoIterator<Item = El>) -> El {
    let mut out = col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .min_w(0.0)
        .clip_soft(0.0)
        .child(viewport);
    for bar in bars {
        out = out.child(anchor(0.0, 0.0, bar));
    }
    out
}

pub fn anchor(x: f32, y: f32, bar: El) -> El {
    col()
        .pin(Some(x), Some(y), None, None)
        .w(0.0)
        .h(0.0)
        .child(bar)
}

impl Inner {
    fn look(&self) -> Option<BarLook> {
        self.look.get()
    }

    /// The trough's length and whether a bar shows at all, for the viewport as it stands.
    fn track(&self) -> (f32, f32, bool) {
        let Some(look) = self.look() else {
            return (0.0, 0.0, false);
        };
        let p = self.pos.get();
        let a = look.along();
        let trough = (p.viewport[a] - look.start).max(0.0);
        let track = look.track(trough);
        (trough, track, p.max[a] > 0.5 && track >= look.min_length)
    }

    fn shows(&self) -> bool {
        self.track().2
    }

    fn held(&self) -> bool {
        self.over.get() || self.dragging.get()
    }

    fn rects(&self, look: BarLook) -> BarRects {
        let p = self.pos.get();
        let a = look.along();
        let (trough, track, shows) = self.track();
        let (trough, track) = if shows { (trough, track) } else { (0.0, 0.0) };
        let hit_thickness = look.expanded_thickness();
        let hit_band = hit_thickness + 2.0 * look.margin;
        let (vis_thickness, vis_thumb_width) = look.active_widths(self.held());
        let vis_band = vis_thickness + 2.0 * look.margin;
        let across = self.width_across(a, p) - look.edge - hit_band;
        let vis_offset = hit_band - vis_band;
        let (start, len) = thumb(p.offset[a], p.max[a], p.viewport[a], track, look.min_length);
        let inset = (vis_thickness - vis_thumb_width) / 2.0;
        BarRects {
            band: look.turned([across, look.start, hit_band, trough]),
            backdrop: look.turned([vis_offset, 0.0, vis_band, trough]),
            trough: look.turned([vis_offset + look.margin, 0.0, vis_thickness, trough]),
            thumb: look.turned([inset, look.ends + start, vis_thumb_width, len]),
        }
    }

    /// Lays the band, backdrop, trough and thumb out instantaneously (scroll tracking, resize, restyle).
    fn layout(&self) {
        let Some(look) = self.look() else {
            return;
        };
        let rects = self.rects(look);
        self.band.snap(rects.band);
        self.backdrop.snap(rects.backdrop);
        self.trough.snap(rects.trough);
        self.thumb.snap(rects.thumb);
    }

    /// Smoothly expands or contracts the bar across its thickness axis when hover/drag flips.
    fn layout_resize(&self) {
        let Some(look) = self.look() else {
            return;
        };
        if look.hover_thickness.is_none() {
            return;
        }
        let rects = self.rects(look);
        self.band.snap(rects.band);
        if look.animations {
            self.backdrop.set(rects.backdrop);
            self.trough.set(rects.trough);
            self.thumb.set(rects.thumb);
        } else {
            self.backdrop.snap(rects.backdrop);
            self.trough.snap(rects.trough);
            self.thumb.snap(rects.thumb);
        }
    }

    fn width_across(&self, a: usize, p: ScrollPos) -> f32 {
        let own = p.viewport[1 - a];
        match self.outer.get() {
            Some(o) => (o.offset[1 - a] + o.drawn[1 - a]).min(own),
            None => own,
        }
    }

    /// Takes the look a view built the bar with.
    fn restyle(&self, look: BarLook) {
        if self.look.replace(Some(look)) != Some(look) {
            self.fill.snap(if self.held() {
                look.trough_hover
            } else {
                look.trough
            });
            self.layout();
            self.settle();
        }
    }

    fn settle(&self) {
        if !self.shows() {
            self.opacity.snap(0.0);
        } else if self.mode.get() == BarMode::Always || self.held() {
            self.opacity.snap(1.0);
        }
    }

    fn follow(&self, mut p: ScrollPos) {
        p.viewport = p.drawn;
        let prev = self.pos.replace(p);
        self.layout();
        let a = self.look().map_or(1, |l| l.along());
        if !self.shows() || self.mode.get() == BarMode::Always {
            self.settle();
        } else if (prev.offset[a] - p.offset[a]).abs() >= 0.5 {
            self.reveal();
        }
    }

    fn reveal(&self) {
        if !self.shows() || self.mode.get() == BarMode::Always {
            return;
        }
        if self.held() {
            self.opacity.set(1.0);
        } else {
            self.opacity.pulse(1.0, 0.0, HOLD);
        }
    }

    fn finish_hold_change(&self, was_held: bool) {
        self.tint();
        if self.held() != was_held {
            self.layout_resize();
        }
        self.reveal();
    }

    fn hover(&self, inside: bool) {
        let was_held = self.held();
        self.over.set(inside);
        self.finish_hold_change(was_held);
    }

    fn release(&self) {
        self.over.set(false);
        self.dragging.set(false);
        if let Some(look) = self.look() {
            self.fill.snap(look.trough);
        }
        self.layout();
        if self.mode.get() == BarMode::Overlay {
            self.opacity.snap(0.0);
        }
        self.settle();
    }

    fn tint(&self) {
        let Some(look) = self.look() else {
            return;
        };
        self.fill.set(if self.held() {
            look.trough_hover
        } else {
            look.trough
        });
    }

    fn drag(&self, d: &mut rune::Drag, tx: &mut rune::Tx) {
        let Some(look) = self.look() else {
            return;
        };
        let p = self.pos.get();
        let a = look.along();
        let (_, track, shows) = self.track();
        if d.phase == Phase::Up {
            let was_held = self.held();
            let was_dragging = self.dragging.replace(false);
            self.finish_hold_change(was_held);
            if was_dragging {
                tx.wake_after(Duration::ZERO);
            }
            return;
        }
        if !shows {
            return;
        }
        let along = if look.horizontal { d.x } else { d.y } - look.ends;
        let (start, len) = thumb(p.offset[a], p.max[a], p.viewport[a], track, look.min_length);
        if d.phase == Phase::Down {
            let was_held = self.held();
            self.dragging.set(true);
            self.finish_hold_change(was_held);
            let on_thumb = (start..=start + len).contains(&along);
            self.grab
                .set(if on_thumb { along - start } else { len / 2.0 });
            if on_thumb {
                return;
            }
        }
        let to = offset_at(
            along - self.grab.get(),
            p.max[a],
            p.viewport[a],
            track,
            look.min_length,
        );
        let at_end = to == 0.0 || to == p.max[a];
        if to == p.offset[a] || (!at_end && (to - p.offset[a]).abs() < 0.5) {
            return;
        }
        let target = self.target.borrow().clone();
        if look.horizontal {
            tx.scroll_to_instant(target, to, p.offset[1]);
        } else {
            tx.scroll_to_instant(target, p.offset[0], to);
        }
    }
}
