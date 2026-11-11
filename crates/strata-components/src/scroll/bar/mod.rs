//! Strata's overlay scroll bar: a thin accent thumb on a faint rounded trough over a
//! viewport's edge, hidden at rest and shown while the view scrolls or the pointer moves over
//! it. See the [scroll module](super) for why it is C24's own and not rune-kit's.

mod geometry;

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use rune::{Color, El, Live, Phase, PointerButton, ScrollPos, Spring, col};
use strata_kit::look::Look;
use strata_kit::slots::scroll as tokens;

pub use geometry::{offset_at, thumb};

/// How long an overlay bar stays up after the last scroll, the pointer's last move over its
/// listing, or the release of its thumb, before it fades.
pub const HOLD: Duration = Duration::from_secs(1);

/// The fade in and out: Strata's `transition: opacity 180ms ease-out`.
const FADE: Spring = Spring::new(0.18, 0.0);

/// The trough's change of strength under the pointer.
const TINT: Spring = Spring::new(0.15, 0.0);

/// How a bar looks and where it sits, from the [`Look`]'s `scroll.*` slot tokens.
///
/// Fields may be added: start from a constructor.
///
/// Used by: C2 ([`BarLook::column`], [`BarLook::strip`]), C3 ([`BarLook::listing`]), C4
/// ([`BarLook::list`]), hosts ([`BarLook::app`]).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BarLook {
    /// Along the viewport's bottom edge, for one that scrolls sideways; else down its right.
    pub horizontal: bool,
    /// The trough's thickness.
    pub thickness: f32,
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
    /// The thumb's and the trough's corner radii (`scroll.thumb.radius`,
    /// `scroll.trough.radius`; a listing's thumb is fully rounded).
    pub thumb_radius: f32,
    pub trough_radius: f32,
    /// Whether the bar fades (`Look::animations`); off, it shows and hides at once.
    pub animations: bool,
}

impl BarLook {
    /// The app-wide bar: a 4 pt accent thumb at least 32 long on a faint 4 pt trough with no
    /// gaps at its ends (`scroll.bar.*`, `scroll.thumb.*`, `scroll.trough.*`).
    pub fn app(look: &Look) -> Self {
        let w = tokens::bar_width(look);
        Self {
            horizontal: false,
            thickness: w,
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

    /// The Icons listing's vertical bar: a 9 pt trough, fainter, with a fully rounded 7 pt
    /// thumb 4 pt in from its ends (`scroll.listing.*`).
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

    /// A Columns column's bar: the app-wide bar, in from the column's right edge and top
    /// (`scroll.column.*`).
    pub fn column(look: &Look) -> Self {
        Self {
            edge: tokens::column_right_gap(look),
            start: tokens::column_top_gap(look),
            ..Self::app(look)
        }
    }

    /// The column strip's sideways bar: the app-wide trough on an opaque band of the
    /// background, with a faint ring inside it (`scroll.strip.*`).
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

    fn band(&self) -> f32 {
        self.thickness + 2.0 * self.margin
    }

    /// `[x, y, w, h]` for a rect laid out down a vertical bar (`across` from its left, `along`
    /// from its top), turned for a horizontal one.
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
    /// Hidden at rest; shown while the view scrolls, the pointer moves over the listing, or the
    /// pointer is over the bar or holds its thumb (Strata, GTK's overlay scrolling).
    #[default]
    Overlay,
    /// Shown whenever the content overflows (a desktop that turns overlay scrolling off).
    Always,
}

struct Inner {
    look: Cell<Option<BarLook>>,
    mode: Cell<BarMode>,
    pos: Cell<ScrollPos>,
    /// The scroll around the viewport, across the bar (C4's sideways scroll), once heard.
    outer: Cell<Option<ScrollPos>>,
    target: RefCell<String>,
    over: Cell<bool>,
    dragging: Cell<bool>,
    grab: Cell<f32>,
    band: Live<[f32; 4]>,
    trough: Live<[f32; 4]>,
    thumb: Live<[f32; 4]>,
    fill: Live<Color>,
    opacity: Live<f32>,
}

/// One scroll bar's state: where its viewport stands, whether the pointer is over it or holds
/// its thumb, and the live rects, fill and opacity that follow them, so scrolling and hovering
/// never build the view. The host keeps one per bar in its state; clones share it.
///
/// A bar is fed its viewport's position ([`Bar::follow`], from `on_scroll_offset` or
/// `Collection::on_scroll`), drawn over the viewport ([`Bar::build`]: through
/// `Collection::edge`, or [`overlay`] for a plain viewport), and revealed by the pointer over
/// the listing ([`Bar::area`] around it). Its thumb drags the viewport and a press on its
/// trough jumps there; neither changes a selection, Alt or not, nor moves the keyboard focus,
/// a middle press on it starts no autoscroll, and a secondary press on it does nothing. It is hidden from assistive technology: the viewport itself scrolls
/// for it.
///
/// Used by: C2, C3, C4, hosts.
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

    /// Follows the viewport to `p`: call it from the viewport's `on_scroll_offset` (or
    /// `Collection::on_scroll`). A scroll shows an overlay bar, which fades [`HOLD`] after.
    pub fn follow(&self, p: ScrollPos) {
        self.0.follow(p);
    }

    /// Follows the scroll `p` around the viewport, across the bar: a vertical bar over a
    /// viewport inside a wider sideways scroll (C4's table in its pane) keeps to the trailing
    /// edge of what that scroll shows, not to its own viewport's, which may be off screen. Call
    /// it from the outer scroll's `on_scroll_offset`; it moves the bar in the same frame.
    pub fn follow_across(&self, p: ScrollPos) {
        self.0.outer.set(Some(p));
        self.0.layout();
    }

    /// Lets go of the bar: its thumb no longer held, the pointer no longer over it, an overlay
    /// bar hidden. Call it when the bar leaves the view (a view switch) mid-drag or under the
    /// pointer: Rune tells an element removed under a drag or the pointer nothing (Rune draft
    /// R-C4-2), so without it the bar comes back held, and shows until the pointer leaves it
    /// again.
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

    /// `listing`, wrapped so the pointer moving over it shows an overlay bar. Size it as the
    /// listing would be; the wrapper fills its parent.
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

    /// The bar, keyed `key`, scrolling the viewport keyed `target` (a key path such as
    /// `viewport_path(scope)`), drawn as `look`, in the viewport's coordinates: put it over
    /// the viewport with `Collection::edge` or [`overlay`]. Its trough is keyed `trough` and
    /// its thumb `thumb`, under `key`. `S` is the app's state type: the bar takes secondary
    /// presses (doing nothing with them), and Rune types that handler by the state.
    pub fn build<S: 'static>(&self, key: &str, target: &str, look: BarLook) -> El {
        let inner = &self.0;
        *inner.target.borrow_mut() = target.to_owned();
        inner.restyle(look);
        let fade = if look.animations { FADE } else { Spring::NONE };
        let tint = if look.animations { TINT } else { Spring::NONE };
        let thumb = col()
            .key("thumb")
            .absolute(0.0, 0.0)
            .live_rect(&inner.thumb)
            .radius(look.thumb_radius)
            .fill(look.thumb);
        let mut trough = col()
            .key("trough")
            .absolute(0.0, 0.0)
            .live_rect(&inner.trough)
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
            .a11y_hidden()
            // A press on the bar leaves the keyboard focus (and any text selection) where it
            // was, as GTK's scrollbar does: without it Rune focuses the nearest ancestor with
            // `on_key`, the collection. (`preserve_focus_on_press` keeps only a blur from
            // happening, not that.)
            .keeps_text_selection()
            .pointer_buttons([PointerButton::Middle])
            // A secondary press on the bar is the bar's: no row's menu, no selection.
            .on_secondary_press(|_: &mut S, _, tx: &mut rune::Tx| tx.live_only());
        if let Some(base) = look.base {
            band = band.fill(base);
        }
        let (hover, drag) = (self.clone(), self.clone());
        band.on_hover_change_any(move |inside, tx| {
            tx.live_only();
            hover.0.hover(inside);
        })
        .on_drag_any(move |d, tx| {
            tx.live_only();
            if tx.button() != PointerButton::Primary {
                // A middle press on the bar is the bar's: it starts no autoscroll.
                return;
            }
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

/// `viewport`, with `bars` over it in its coordinates: a column for a plain scroll viewport
/// whose bars aren't drawn by a collection. Size it as the viewport would be sized, and let
/// the viewport fill it (`.w_full().h_full()`).
///
/// Each bar hangs from an [`anchor`] at the viewport's top-left, not from a layer the
/// viewport's size: the bar places its own parts, and a layer over the whole viewport would be
/// what a hit test finds there (a system drag's, P1's) instead of the content under it.
///
/// Used by: C2 (its column strip), hosts.
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

/// `bar` (a [`Bar::build`]) hung from a zero-size point at (`x`, `y`) in its parent: the bar
/// lays its band, trough and thumb out in the viewport's coordinates from there, and nothing
/// but those parts takes the parent's area. Put a bar over a viewport this way rather than in
/// a layer pinned over the whole of it, which hit tests would find over the content.
///
/// Used by: C1e (`Collection::edge`), C6 (the crumbs' bar row), [`overlay`].
pub fn anchor(x: f32, y: f32, bar: El) -> El {
    col()
        .pin(Some(x), Some(y), None, None)
        .w(0.0)
        .h(0.0)
        .child(bar)
}

impl Inner {
    /// The look the last view built the bar with. Before the first build there is none, and
    /// the bar lays out and shows nothing: no default look reaches the screen.
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

    /// Lays the band, trough and thumb out for the viewport as it stands.
    fn layout(&self) {
        let Some(look) = self.look() else {
            return;
        };
        let p = self.pos.get();
        let a = look.along();
        let (trough, track, shows) = self.track();
        let (trough, track) = if shows { (trough, track) } else { (0.0, 0.0) };
        let band = look.band();
        let across = self.width_across(a, p) - look.edge - band;
        self.band
            .snap(look.turned([across, look.start, band, trough]));
        self.trough
            .snap(look.turned([look.margin, 0.0, look.thickness, trough]));
        let (start, len) = thumb(p.offset[a], p.max[a], p.viewport[a], track, look.min_length);
        let inset = (look.thickness - look.thumb_width) / 2.0;
        self.thumb
            .snap(look.turned([inset, look.ends + start, look.thumb_width, len]));
    }

    /// How far across the viewport the bar's far edge is: the viewport's own size, or the
    /// trailing edge of what a scroll around it shows ([`Bar::follow_across`]), whichever is
    /// nearer.
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

    /// The opacity for the mode, without a pulse: shown when `Always` and overflowing, hidden
    /// when it doesn't overflow.
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

    /// Shows an overlay bar, held while the pointer is over it or holds it, else fading after
    /// [`HOLD`].
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

    fn hover(&self, inside: bool) {
        self.over.set(inside);
        self.tint();
        // Held while inside; once it leaves (and nothing holds the thumb), the fade's hold
        // starts again.
        self.reveal();
    }

    fn release(&self) {
        self.over.set(false);
        self.dragging.set(false);
        if let Some(look) = self.look() {
            self.fill.snap(look.trough);
        }
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
            self.dragging.set(false);
            self.tint();
            self.reveal();
            return;
        }
        if !shows {
            return;
        }
        let along = if look.horizontal { d.x } else { d.y } - look.ends;
        let (start, len) = thumb(p.offset[a], p.max[a], p.viewport[a], track, look.min_length);
        if d.phase == Phase::Down {
            self.dragging.set(true);
            self.tint();
            self.reveal();
            let on_thumb = (start..=start + len).contains(&along);
            // A press on the thumb holds it where it was caught; one on the trough jumps the
            // thumb's middle there (GTK's primary-button warp), and the drag goes on from it.
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
        let target = self.target.borrow().clone();
        if look.horizontal {
            tx.scroll_to_instant(target, to, p.offset[1]);
        } else {
            tx.scroll_to_instant(target, p.offset[0], to);
        }
    }
}
