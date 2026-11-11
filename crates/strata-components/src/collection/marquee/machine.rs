//! The marquee state machine: a band begun, moved, ticked and ended, turned into C1a band
//! operations and [`Outcome`]s. Plain data, no Rune elements.

use std::time::Duration;

use rune::{Mods, Spring};

use super::settings::MarqueeSettings;
use super::speed::{edge_speed, glide_lead, stop_lead};
use crate::collection::selection::{BandMode, Changed, Selection};
use crate::contract::{CollectionGeometry, EntryId, EntrySource, IndexRanges, Point, Rect};

/// The longest gap between two ticks still measured as one frame step; a longer one (the
/// first tick of a glide) takes [`MarqueeSettings::frame`].
const MAX_STEP: f64 = 0.1;

/// How far (points) the scroll offset must move between two steps to count as scrolling.
const MOVED: f32 = 0.01;

/// What the machine asks for besides selection changes.
///
/// Variants may be added.
///
/// Used by: the C1e adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Outcome {
    /// Glide the viewport so its top shows content y `y`, on `spring`: one step of edge
    /// auto-scroll. Sent every tick while it scrolls; each keeps the glide's speed.
    Glide {
        /// The content offset to aim for.
        y: f32,
        /// The spring to glide on.
        spring: Spring,
    },
    /// Bring the auto-scroll glide to rest at content offset `y`, on `spring`, without passing
    /// it. A glide request like [`Outcome::Glide`]; its own variant so the element can map it
    /// to a stop of its own.
    StopGlide {
        /// Where the glide comes to rest.
        y: f32,
        /// The spring to glide on.
        spring: Spring,
    },
    /// Stop the view dead at content offset `y`, where it shows now: a jump, which also ends a
    /// glide in progress. Sent when the band ends (release, Escape, another selection) while
    /// its auto-scroll still moves the view, so the row under the pointer stays there, as
    /// Strata's timer stops.
    StopScroll {
        /// The offset to stay at.
        y: f32,
    },
    /// Call [`MarqueeMachine::tick`] again after this long (zero: the next frame).
    WakeAfter(Duration),
    /// The band was shown or hidden: the view must commit to add or drop it. Its rect moves
    /// without a commit, through the live rect [`MarqueeMachine::drawn`] fills.
    Repaint,
}

/// What the machine reads and changes: the entries, where they sit, the viewport, the clock,
/// the selection, and where its outcomes go. C1e's `Ctx` is one; tests use a plain one.
///
/// Used by: the C1e adapter.
pub trait Surface {
    /// The entries, in display order.
    fn source(&self) -> &dyn EntrySource;

    /// Where the entries sit, mounted or not.
    fn geometry(&self) -> &dyn CollectionGeometry;

    /// The part of the content the viewport shows, in content coordinates.
    fn visible(&self) -> Rect;

    /// The host's clock, in seconds.
    fn now(&self) -> f64;

    /// How tall the content is: the bottom of the last entry, 0 with none (C1e:
    /// `Ctx::content_extent`).
    fn content_extent(&self) -> f32;

    /// The settings the host gave (C1e: `Collection::settings`), else the defaults.
    fn settings(&self) -> MarqueeSettings;

    /// The selection, to read.
    fn selection(&self) -> &Selection;

    /// Runs one selection operation, and returns what it changed.
    fn apply(&mut self, op: &mut dyn FnMut(&mut Selection, &dyn EntrySource) -> Changed)
    -> Changed;

    /// Hands on an outcome.
    fn emit(&mut self, outcome: Outcome);
}

/// How the modifiers held at the press combine the band with the selection: Ctrl toggles
/// (Ctrl+Shift too, as Strata's `band_selection` lets Ctrl win), Shift adds, anything else
/// (Alt, which only forces a marquee) replaces.
///
/// Used by: the C1e adapter, C2.
pub fn mode_for(mods: Mods) -> BandMode {
    if mods.ctrl {
        BandMode::Toggle
    } else if mods.shift {
        BandMode::Add
    } else {
        BandMode::Replace
    }
}

/// The marquee: one band at a time over one collection.
///
/// Call [`MarqueeMachine::begin`] when the pointer controller starts a marquee, then
/// [`moved`](Self::moved) for each pointer move, [`tick`](Self::tick) from the host's poll
/// (every frame while it asks, with [`Outcome::WakeAfter`]), and [`end`](Self::end) on the
/// release or [`cancel`](Self::cancel) on Escape or a cancelled gesture.
///
/// Used by: the C1e adapter.
#[derive(Debug, Default)]
pub struct MarqueeMachine {
    band: Option<Band>,
}

/// An open band.
#[derive(Clone, Debug)]
struct Band {
    /// The press, in content coordinates.
    anchor: Point,
    /// The pointer, in viewport coordinates: it stays put while the content scrolls under it.
    pointer: Point,
    /// The viewport at the last step, in content coordinates.
    visible: Rect,
    /// What the band crossed when last applied.
    crossed: IndexRanges,
    /// The selection's revision after the last apply: another change (a sync, a rename)
    /// moves it, and the band is applied again over the new listing.
    revision: u64,
    /// The listing's length and the ids at the ends of what was applied: a re-sort or a
    /// reload changes them without a selection change, and the band then takes the rows now
    /// under it, as Strata's band hit-tests by position.
    listing: Listing,
    /// When the band was last applied.
    applied_at: f64,
    /// The auto-scroll speed the last step asked for, points a second.
    speed: f32,
    /// A soft stop ([`Outcome::StopGlide`]) may still be moving the view: the ticks left with
    /// the view still before it counts as landed. Two, since the tick right after the stop
    /// comes before Rune steps it.
    coasting: u8,
    /// Ticks (frames) since the band was last applied.
    frames: u32,
    /// When the last tick ran.
    ticked_at: Option<f64>,
    /// A key took the view ([`MarqueeMachine::yield_scroll`]): no edge scroll until the
    /// pointer next moves.
    held: bool,
}

impl Band {
    fn current(&self) -> Point {
        Point::new(
            self.pointer.x + self.visible.x,
            self.pointer.y + self.visible.y,
        )
    }

    fn rect(&self) -> Rect {
        Rect::from_corners(self.anchor, self.current())
    }

    /// Whether the selection or the listing changed since the band was last applied, so the
    /// same indices now mean other entries.
    fn stale(&self, s: &dyn Surface) -> bool {
        s.selection().revision() != self.revision
            || Listing::of(s.source(), &self.crossed) != self.listing
    }
}

/// A cheap fingerprint of the listing under a band: its length and the ids at the ends of
/// the crossed ranges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Listing {
    len: usize,
    first: Option<EntryId>,
    last: Option<EntryId>,
}

impl Listing {
    fn of(source: &dyn EntrySource, crossed: &IndexRanges) -> Self {
        let len = source.len();
        let id = |i: usize| (i < len).then(|| source.id_at(i));
        let ranges = crossed.ranges();
        Self {
            len,
            first: ranges.first().and_then(|r| id(r.start)),
            last: ranges.last().and_then(|r| id(r.end.saturating_sub(1))),
        }
    }
}

/// Which input a step is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Move,
    Tick,
}

impl MarqueeMachine {
    /// A machine with no band open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a band is open.
    pub fn is_active(&self) -> bool {
        self.band.is_some()
    }

    /// The band, in content coordinates, as of the last step.
    pub fn band(&self) -> Option<Rect> {
        self.band.as_ref().map(Band::rect)
    }

    /// The band to draw, in content coordinates: [`MarqueeMachine::band`] cut to the viewport
    /// and one viewport around it, so a band over 100,000 rows never draws whole. `None` when
    /// no band is open or none of it is that near the viewport.
    pub fn drawn(&self) -> Option<Rect> {
        let band = self.band.as_ref()?;
        let v = band.visible;
        let near = Rect::new(v.x - v.w, v.y - v.h, 3.0 * v.w, 3.0 * v.h);
        intersect(band.rect(), near)
    }

    /// Starts a band at content point `at`, with `mods` held at the press ([`mode_for`]). A
    /// band already open ends first, keeping what it selected.
    pub fn begin(&mut self, at: Point, mods: Mods, s: &mut dyn Surface) {
        if self.band.is_some() {
            self.end(s);
        }
        let mode = mode_for(mods);
        s.apply(&mut |sel, _| sel.begin_band(mode));
        let visible = s.visible();
        self.band = Some(Band {
            anchor: at,
            pointer: Point::new(at.x - visible.x, at.y - visible.y),
            visible,
            crossed: IndexRanges::new(),
            revision: s.selection().revision(),
            listing: Listing::default(),
            applied_at: f64::NEG_INFINITY,
            speed: 0.0,
            coasting: 0,
            frames: 0,
            ticked_at: None,
            held: false,
        });
        s.emit(Outcome::Repaint);
        self.step(s, Step::Move);
    }

    /// The pointer moved to content point `at`. True when the selection changed.
    pub fn moved(&mut self, at: Point, s: &mut dyn Surface) -> bool {
        let visible = s.visible();
        let Some(band) = self.band.as_mut() else {
            return false;
        };
        band.pointer = Point::new(at.x - visible.x, at.y - visible.y);
        band.held = false;
        self.step(s, Step::Move)
    }

    /// Runs one frame: follows a scroll under the band, glides on near an edge, and applies a
    /// band update that waited. True when it changed what the view reads (the selection, or
    /// the band shown or hidden).
    pub fn tick(&mut self, s: &mut dyn Surface) -> bool {
        self.step(s, Step::Tick)
    }

    /// The button came up: applies the band where it ends, keeps what it selected, and stops
    /// the auto-scroll dead, so the last row it selected stays under the pointer.
    pub fn end(&mut self, s: &mut dyn Surface) {
        let Some(mut band) = self.band.take() else {
            return;
        };
        if s.selection().is_banding() {
            band.visible = s.visible();
            let crossed = s.geometry().range_in(band.rect());
            if crossed != band.crossed || band.stale(&*s) {
                s.apply(&mut |sel, src| sel.band(src, &crossed));
            }
            s.apply(&mut |sel, _| {
                sel.end_band();
                Changed::NONE
            });
        }
        halt(&band, s.visible(), s);
        s.emit(Outcome::Repaint);
    }

    /// Escape, or a gesture that ended without a release: puts back the selection the band
    /// started from (D2 in the module docs) and stops the auto-scroll dead. False if no band
    /// was open, so the key goes on.
    pub fn cancel(&mut self, s: &mut dyn Surface) -> bool {
        let Some(band) = self.band.take() else {
            return false;
        };
        let open = s.selection().is_banding();
        if open {
            s.apply(&mut |sel, _| sel.cancel_band());
        }
        halt(&band, s.visible(), s);
        s.emit(Outcome::Repaint);
        open
    }

    /// Drops an open band without touching the selection, which the next folder replaces.
    pub fn reset(&mut self) {
        self.band = None;
    }

    /// A key moved the cursor and the view is revealing it: stops the edge scroll with no
    /// scroll request of its own (a stop on a later tick would undo the reveal), and scrolls
    /// no more until the pointer next moves. True when the edge scroll was moving the view, so
    /// the caller stops it dead before the reveal.
    pub fn yield_scroll(&mut self) -> bool {
        let Some(band) = self.band.as_mut() else {
            return false;
        };
        let moving = band.speed != 0.0 || band.coasting > 0;
        band.speed = 0.0;
        band.coasting = 0;
        band.held = true;
        moving
    }

    fn step(&mut self, s: &mut dyn Surface, step: Step) -> bool {
        let settings = s.settings();
        let visible = s.visible();
        // Another selecting operation (Ctrl+A, a click) ended the band in the selection.
        if !s.selection().is_banding() {
            let Some(band) = self.band.take() else {
                return false;
            };
            halt(&band, visible, s);
            s.emit(Outcome::Repaint);
            return true;
        }
        let Some(band) = self.band.as_mut() else {
            return false;
        };
        let now = s.now();
        let view_moved = (visible.x - band.visible.x).abs() > MOVED
            || (visible.y - band.visible.y).abs() > MOVED;
        band.visible = visible;
        if step == Step::Tick {
            band.frames = band.frames.saturating_add(1);
            // A soft stop has landed once the view stays still for its ticks.
            if !view_moved {
                band.coasting = band.coasting.saturating_sub(1);
            }
        }

        let was = band.speed;
        let speed = if band.held {
            0.0
        } else {
            scroll_speed(band, &*s, &settings)
        };
        if step == Step::Tick {
            if speed != 0.0 {
                let dt = band
                    .ticked_at
                    .map(|t| now - t)
                    .filter(|dt| *dt > 0.0 && *dt <= MAX_STEP)
                    .map_or(settings.frame, Duration::from_secs_f64);
                let lead = if settings.animations {
                    glide_lead(speed, settings.glide, dt)
                } else {
                    speed * dt.as_secs_f32()
                };
                s.emit(Outcome::Glide {
                    y: visible.y + lead,
                    spring: settings.glide,
                });
                s.emit(Outcome::WakeAfter(Duration::ZERO));
            }
            band.ticked_at = Some(now);
        } else if speed != 0.0 && was == 0.0 {
            // The next frame's tick starts the glide.
            s.emit(Outcome::WakeAfter(Duration::ZERO));
        }
        if speed == 0.0 && was != 0.0 {
            // Leaving the edge mid-band: come to rest softly, as the band goes on. With
            // animations off Rune lands a glide at once, so aim where the view is.
            let ahead = if settings.animations {
                stop_lead(was, settings.glide)
            } else {
                0.0
            };
            s.emit(Outcome::StopGlide {
                y: visible.y + ahead,
                spring: settings.glide,
            });
            band.coasting = 2;
        }
        band.speed = speed;

        let crossed = s.geometry().range_in(band.rect());
        if crossed == band.crossed && !band.stale(&*s) {
            return false;
        }
        // A glide that ran into this step, or a scroll by hand since the last one. A move that
        // only now reaches the edge isn't scrolling yet, so it applies at once. While it
        // scrolls, an apply waits for both the interval and the frames (D1).
        let scrolling = view_moved || was != 0.0;
        let due = band.applied_at + settings.apply_every.as_secs_f64();
        if scrolling && (now < due || band.frames < settings.apply_frames) {
            if speed == 0.0 {
                let wait = (due - now).max(0.0);
                s.emit(Outcome::WakeAfter(Duration::from_secs_f64(wait)));
            }
            return false;
        }
        let changed = s.apply(&mut |sel, src| sel.band(src, &crossed));
        band.crossed = crossed;
        band.revision = s.selection().revision();
        band.listing = Listing::of(s.source(), &band.crossed);
        band.applied_at = now;
        band.frames = 0;
        changed.any()
    }
}

/// The auto-scroll speed for the band's pointer, or zero where the content can't scroll that
/// way: at its top or bottom, or when it fits the viewport (Strata's `scrollable_step`).
fn scroll_speed(band: &Band, s: &dyn Surface, settings: &MarqueeSettings) -> f32 {
    let v = band.visible;
    let speed = edge_speed(band.pointer.y, v.h, settings.edge);
    let extent = s.content_extent();
    let at_end = if speed < 0.0 {
        v.y <= 0.5
    } else {
        v.bottom() >= extent - 0.5
    };
    if speed == 0.0 || extent <= v.h || at_end {
        0.0
    } else {
        speed
    }
}

/// Stops `band`'s auto-scroll dead where the view shows now, if it is still moving the view.
fn halt(band: &Band, visible: Rect, s: &mut dyn Surface) {
    if band.speed != 0.0 || band.coasting > 0 {
        s.emit(Outcome::StopScroll { y: visible.y });
    }
}

/// The area two rects share, or `None` if they share none.
fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    (right >= x && bottom >= y).then(|| Rect::new(x, y, right - x, bottom - y))
}
