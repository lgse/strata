//! The marquee machine on a plain [`Surface`] and a virtual clock. Each test names the
//! feature-map bullet (or the decision) it checks. Rows are 28 pt tall and ids are the rows'
//! first indices, so `[4, 5]` reads as "the fifth and sixth rows as first listed".

use std::time::Duration;

use rune::{Mods, Spring};

use super::*;
use crate::collection::selection::{BandMode, Changed, CursorGone, Selection, SyncRule};
use crate::contract::{
    CollectionGeometry, EntryId, EntryKind, EntrySource, ListEntry, ListSource, Point, Rect,
    UniformRows,
};

const ROW: f32 = 28.0;
const WIDTH: f32 = 400.0;
const VIEW: f32 = 560.0;
const FRAME: f64 = 1.0 / 60.0;

struct Fake {
    source: ListSource,
    geometry: UniformRows,
    visible: Rect,
    now: f64,
    sel: Selection,
    out: Vec<Outcome>,
    /// Every selection change, as (clock, changed).
    applied: Vec<(f64, Changed)>,
    settings: MarqueeSettings,
}

impl Surface for Fake {
    fn source(&self) -> &dyn EntrySource {
        &self.source
    }

    fn geometry(&self) -> &dyn CollectionGeometry {
        &self.geometry
    }

    fn visible(&self) -> Rect {
        self.visible
    }

    fn now(&self) -> f64 {
        self.now
    }

    fn content_extent(&self) -> f32 {
        match self.source.len() {
            0 => 0.0,
            n => self.geometry.rect_of(n - 1).bottom(),
        }
    }

    fn settings(&self) -> MarqueeSettings {
        self.settings
    }

    fn selection(&self) -> &Selection {
        &self.sel
    }

    fn apply(
        &mut self,
        op: &mut dyn FnMut(&mut Selection, &dyn EntrySource) -> Changed,
    ) -> Changed {
        let changed = op(&mut self.sel, &self.source);
        if changed.any() {
            self.applied.push((self.now, changed));
        }
        changed
    }

    fn emit(&mut self, outcome: Outcome) {
        self.out.push(outcome);
    }
}

fn entry(raw: u64) -> ListEntry {
    ListEntry::new(EntryId::new(raw), format!("e{raw}"), EntryKind::File, "")
}

/// `n` rows in a viewport scrolled to the top.
fn fake(n: u64) -> Fake {
    Fake {
        source: ListSource::new((0..n).map(entry).collect()),
        geometry: UniformRows::new(n as usize, ROW, WIDTH).viewport_height(VIEW),
        visible: Rect::new(0.0, 0.0, WIDTH, VIEW),
        now: 0.0,
        sel: Selection::new(),
        out: Vec::new(),
        applied: Vec::new(),
        settings: MarqueeSettings::default(),
    }
}

fn machine() -> MarqueeMachine {
    MarqueeMachine::new()
}

/// The content point in the middle of row `row`'s height, in its blank space.
fn at_row(row: usize) -> Point {
    Point::new(300.0, row as f32 * ROW + ROW / 2.0)
}

/// The content point at viewport y `y`, as the viewport shows now.
fn at_view(f: &Fake, y: f32) -> Point {
    Point::new(300.0, f.visible.y + y)
}

fn selected(f: &Fake) -> Vec<u64> {
    f.sel
        .selected_ids(&f.source)
        .into_iter()
        .map(EntryId::get)
        .collect()
}

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

fn glides(out: &[Outcome]) -> Vec<f32> {
    out.iter()
        .filter_map(|o| match o {
            Outcome::Glide { y, .. } => Some(*y),
            _ => None,
        })
        .collect()
}

fn stops(out: &[Outcome]) -> usize {
    out.iter()
        .filter(|o| matches!(o, Outcome::StopGlide { .. }))
        .count()
}

/// A viewport glide as Rune steps one (`runtime/scroll/offset.rs` at the pinned rev): a new
/// target keeps the speed unless it would overshoot (`retarget_v`), then the spring's exact
/// solution moves the offset.
#[derive(Default)]
struct Glide {
    v: f32,
    to: Option<f32>,
}

impl Glide {
    fn aim(&mut self, spring: Spring, offset: f32, to: f32) {
        let d = offset - to;
        let most = spring.omega() * d.abs();
        if self.v * d < 0.0 && self.v.abs() > most {
            self.v = -d.signum() * most;
        }
        self.to = Some(to);
    }

    fn step(&mut self, spring: Spring, offset: f32, max: f32, dt: f32) -> f32 {
        let Some(to) = self.to else {
            return offset;
        };
        let to = to.clamp(0.0, max);
        let (d, v) = spring.sample(offset - to, self.v, 0.0, dt);
        self.v = v;
        if d.abs() < 0.1 && v.abs() < 4.0 {
            self.to = None;
            self.v = 0.0;
            return to;
        }
        to + d
    }
}

/// Applies the scroll requests in `f.out` as Rune does: a glide aims the glide, a stop jumps
/// and ends it. With `jumps` (reduced motion), a glide lands at once too.
fn scroll(f: &mut Fake, glide: &mut Glide, jumps: bool) {
    let spring = f.settings.glide;
    let max = f.geometry.len as f32 * ROW - VIEW;
    for o in std::mem::take(&mut f.out) {
        match o {
            Outcome::Glide { y, .. } | Outcome::StopGlide { y, .. } if jumps => {
                f.visible.y = y.clamp(0.0, max);
            }
            Outcome::Glide { y, .. } | Outcome::StopGlide { y, .. } => {
                glide.aim(spring, f.visible.y, y);
            }
            Outcome::StopScroll { y } => {
                *glide = Glide::default();
                f.visible.y = y;
            }
            _ => {}
        }
    }
}

/// One frame as the element runs it, `dt` long: the host's poll ticks the machine, then Rune
/// steps the glide the machine asked for. Returns the tick's result.
fn frame_of(f: &mut Fake, m: &mut MarqueeMachine, glide: &mut Glide, dt: f64) -> bool {
    f.now += dt;
    let changed = m.tick(f);
    scroll(f, glide, false);
    let max = f.geometry.len as f32 * ROW - VIEW;
    f.visible.y = glide.step(f.settings.glide, f.visible.y, max, dt as f32);
    changed
}

/// One 60 Hz frame.
fn frame(f: &mut Fake, m: &mut MarqueeMachine, glide: &mut Glide) -> bool {
    frame_of(f, m, glide, FRAME)
}

#[test]
fn a_band_selects_every_row_it_crosses_mounted_or_not() {
    // Hit-testing by index: the band takes every row it crosses, mounted or not (the
    // marquee-drag script checks the drag from blank space).
    let mut f = fake(10_000);
    let mut m = machine();
    m.begin(at_row(0), Mods::default(), &mut f);
    m.moved(at_view(&f, 300.0), &mut f);
    assert_eq!(selected(&f), (0..=10).collect::<Vec<_>>());
    // The view jumps to the end (the scrollbar, End): rows 11 to 9,979 were never shown.
    f.visible.y = 10_000.0 * ROW - VIEW;
    m.moved(at_view(&f, VIEW - 1.0), &mut f);
    m.end(&mut f);
    assert_eq!(f.sel.len(), 10_000, "a range by index, not by mounted rows");
    assert!(!m.is_active());
}

#[test]
fn a_plain_band_replaces_the_selection_and_moving_back_unselects() {
    // browser/selection/marquee: A plain marquee replaces the selection with the entries under the band.
    let mut f = fake(20);
    f.sel.select_only(&f.source, 1);
    f.sel.toggle(&f.source, 12);
    let mut m = machine();
    m.begin(at_row(5), Mods::default(), &mut f);
    m.moved(at_row(8), &mut f);
    assert_eq!(selected(&f), [5, 6, 7, 8]);
    m.moved(at_row(6), &mut f);
    assert_eq!(selected(&f), [5, 6]);
    m.end(&mut f);
    assert_eq!(selected(&f), [5, 6]);
}

#[test]
fn a_ctrl_band_toggles_against_the_selection_at_press() {
    // browser/selection/marquee: A Ctrl-marquee toggles each crossed entry relative to the selection at press.
    for mods in [
        ctrl(),
        Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        },
    ] {
        let mut f = fake(20);
        f.sel.select_only(&f.source, 5);
        f.sel.toggle(&f.source, 6);
        f.sel.toggle(&f.source, 15);
        let mut m = machine();
        m.begin(at_row(4), mods, &mut f);
        m.moved(at_row(7), &mut f);
        assert_eq!(selected(&f), [4, 7, 15], "Ctrl wins over Shift (D5)");
        // Moving the band back off 6 and 7 puts them back as they were at the press.
        m.moved(at_row(5), &mut f);
        assert_eq!(selected(&f), [4, 6, 15]);
        m.end(&mut f);
        assert_eq!(selected(&f), [4, 6, 15]);
    }
}

#[test]
fn a_shift_band_adds_to_the_selection_at_press() {
    // browser/selection/marquee: A Shift-marquee adds the crossed entries to the selection at press.
    let mut f = fake(20);
    f.sel.select_only(&f.source, 1);
    f.sel.toggle(&f.source, 5);
    let mut m = machine();
    m.begin(at_row(4), shift(), &mut f);
    m.moved(at_row(6), &mut f);
    assert_eq!(selected(&f), [1, 4, 5, 6]);
    m.moved(at_row(4), &mut f);
    assert_eq!(selected(&f), [1, 4, 5], "5 was selected at the press");
    m.end(&mut f);
    assert_eq!(selected(&f), [1, 4, 5]);
}

#[test]
fn the_mode_mapping_lets_alt_replace() {
    // The modifiers to band mode: Alt only forces a marquee (C1b's decision), so an Alt band
    // replaces and a Ctrl+Alt band toggles.
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    assert_eq!(mode_for(alt), BandMode::Replace);
    let ctrl_alt = Mods {
        alt: true,
        ..ctrl()
    };
    assert_eq!(mode_for(ctrl_alt), BandMode::Toggle);
}

#[test]
fn the_selection_survives_pointer_movement_after_release() {
    // browser/selection/marquee: The selection survives pointer movement after release.
    let mut f = fake(20);
    let mut m = machine();
    m.begin(at_row(2), Mods::default(), &mut f);
    m.moved(at_row(4), &mut f);
    m.end(&mut f);
    assert!(!m.moved(at_row(12), &mut f));
    assert!(!m.tick(&mut f));
    assert_eq!(selected(&f), [2, 3, 4]);
}

#[test]
fn the_band_never_moves_the_cursor_or_the_anchor() {
    // D3: the band leaves the cursor and the anchor alone (the marquee-drag script checks the
    // preview and the intents on the page).
    let mut f = fake(20);
    f.sel.select_only(&f.source, 2);
    let (cursor, anchor) = (f.sel.cursor(), f.sel.anchor());
    let mut m = machine();
    m.begin(at_row(4), ctrl(), &mut f);
    m.moved(at_row(9), &mut f);
    m.end(&mut f);
    assert_eq!(selected(&f), [2, 4, 5, 6, 7, 8, 9]);
    assert_eq!((f.sel.cursor(), f.sel.anchor()), (cursor, anchor));
    assert!(f.applied.iter().all(|(_, c)| !c.cursor && !c.anchor));
}

#[test]
fn escape_puts_back_the_selection_the_band_started_from() {
    // D2: Escape, or a gesture cancelled without a release, restores the selection at press.
    let mut f = fake(20);
    f.sel.select_only(&f.source, 1);
    f.sel.toggle(&f.source, 2);
    let mut m = machine();
    m.begin(at_row(0), ctrl(), &mut f);
    m.moved(at_row(5), &mut f);
    assert_eq!(selected(&f), [0, 3, 4, 5]);
    assert!(m.cancel(&mut f));
    assert_eq!(selected(&f), [1, 2]);
    assert!(!m.cancel(&mut f), "with no band open the key goes on");

    // A plain band cleared the selection at its press, and goes back to that.
    m.begin(at_row(6), Mods::default(), &mut f);
    m.moved(at_row(8), &mut f);
    assert!(m.cancel(&mut f));
    assert!(f.sel.is_empty());
}

#[test]
fn another_selecting_operation_ends_the_band() {
    // C1a: any other selecting operation ends a band; the marquee drops its band with it.
    let mut f = fake(20);
    let mut m = machine();
    m.begin(at_row(2), Mods::default(), &mut f);
    m.moved(at_row(4), &mut f);
    f.out.clear();
    f.sel.select_all(&f.source);
    assert!(m.tick(&mut f), "the band is hidden");
    assert!(!m.is_active());
    assert_eq!(f.out, [Outcome::Repaint]);
    assert!(
        !m.cancel(&mut f),
        "Escape then clears the selection, not the band"
    );
    assert_eq!(f.sel.len(), 20);
}

#[test]
fn a_sync_during_the_band_applies_it_again_over_the_new_listing() {
    // C1a hands this to C1d: after a re-sort mid-band, the band covers the rows now under it.
    let mut f = fake(20);
    let mut m = machine();
    m.begin(at_row(2), Mods::default(), &mut f);
    m.moved(at_row(5), &mut f);
    assert_eq!(selected(&f), [2, 3, 4, 5]);
    f.source.sort_by(|a, b| b.id.cmp(&a.id));
    f.sel
        .sync(&f.source, SyncRule::complete(CursorGone::Neighbor));
    m.tick(&mut f);
    // Rows 2 to 5 now hold ids 17 to 14.
    assert_eq!(selected(&f), [17, 16, 15, 14]);
}

#[test]
fn the_speed_curve_is_stratas_auto_scroll_step() {
    // Strata's `auto_scroll_step`: zero to the edge line, linear to the most at the edge, held
    // beyond it.
    let e = EdgeScroll::STRATA;
    assert_eq!(edge_speed(VIEW / 2.0, VIEW, e), 0.0);
    assert_eq!(edge_speed(e.edge, VIEW, e), 0.0);
    let half = edge_speed(VIEW - e.edge / 2.0, VIEW, e);
    let full = edge_speed(VIEW, VIEW, e);
    assert!(half > 0.0 && (full - 2.0 * half).abs() < 1e-3);
    assert_eq!(full, e.max_speed);
    assert_eq!(
        edge_speed(VIEW + 200.0, VIEW, e),
        e.max_speed,
        "beyond the edge it holds"
    );
    assert_eq!(
        edge_speed(-50.0, VIEW, e),
        -e.max_speed,
        "the top edge scrolls up"
    );
    assert_eq!(
        edge_speed(5.0, 2.0 * e.edge, e),
        0.0,
        "too short a view never scrolls"
    );
    assert_eq!(edge_speed(f32::NAN, VIEW, e), 0.0);
}

#[test]
fn a_glide_aimed_by_the_lead_settles_at_the_speed_and_stops_without_overshoot() {
    let spring = rune::tok::motion::GLIDE;
    let step = Duration::from_secs_f64(FRAME);
    for speed in [100.0, 750.0, 1500.0] {
        let mut glide = Glide::default();
        let mut y = 0.0;
        let mut at_1s = 0.0;
        for i in 0..120 {
            glide.aim(spring, y, y + glide_lead(speed, spring, step));
            y = glide.step(spring, y, f32::MAX, FRAME as f32);
            if i == 59 {
                at_1s = y;
            }
        }
        let measured = (y - at_1s) / 1.0;
        assert!(
            (measured - speed).abs() < 0.01 * speed,
            "{speed} pt/s glided at {measured}"
        );
        let target = y + stop_lead(speed, spring);
        glide.aim(spring, y, target);
        let mut furthest = y;
        for _ in 0..60 {
            y = glide.step(spring, y, f32::MAX, FRAME as f32);
            furthest = furthest.max(y);
        }
        assert!(furthest <= target + 0.01, "no overshoot at {speed} pt/s");
        assert!((y - target).abs() < 0.5, "at rest at {speed} pt/s");
    }
}

#[test]
fn holding_at_the_bottom_edge_glides_and_extends_the_selection() {
    // browser/selection/marquee: Holding the pointer near a viewport edge scrolls the listing and extends the selection each frame.
    let mut f = fake(100_000);
    let mut m = machine();
    let mut glide = Glide::default();
    m.begin(at_row(1), Mods::default(), &mut f);
    m.moved(at_view(&f, VIEW - 2.0), &mut f);
    assert!(f.out.contains(&Outcome::WakeAfter(Duration::ZERO)));
    let mut counts = Vec::new();
    for _ in 0..120 {
        frame(&mut f, &mut m, &mut glide);
        counts.push(f.sel.len());
    }
    let scrolled = f.visible.y;
    assert!(scrolled > 1000.0, "it glided down ({scrolled} pt in 2 s)");
    assert!(counts.windows(2).all(|w| w[1] >= w[0]));
    assert!(counts[119] > counts[30] && counts[30] > counts[0]);
    // D1: while it scrolls, the band is applied at most every 100 ms.
    let during: Vec<f64> = f
        .applied
        .iter()
        .map(|(t, _)| *t)
        .filter(|t| *t > 0.0)
        .collect();
    assert!(during.windows(2).all(|w| w[1] - w[0] >= 0.1 - 1e-9));
    // The release applies the band where it ends: rows 1 to the one under the pointer.
    let pointer_row = ((f.visible.y + VIEW - 2.0) / ROW) as usize;
    m.end(&mut f);
    assert_eq!(f.sel.len(), pointer_row);
    assert!(
        f.out.contains(&Outcome::StopScroll { y: f.visible.y }),
        "the release stops the view dead"
    );
}

#[test]
fn leaving_the_edge_or_reaching_the_end_stops_the_glide() {
    let mut f = fake(1_000);
    let mut m = machine();
    let mut glide = Glide::default();
    m.begin(at_row(1), Mods::default(), &mut f);
    m.moved(at_view(&f, VIEW), &mut f);
    for _ in 0..10 {
        frame(&mut f, &mut m, &mut glide);
    }
    f.out.clear();
    m.moved(at_view(&f, VIEW / 2.0), &mut f);
    assert_eq!(stops(&f.out), 1);
    for _ in 0..30 {
        frame(&mut f, &mut m, &mut glide);
    }
    f.out.clear();
    frame(&mut f, &mut m, &mut glide);
    assert!(glides(&f.out).is_empty() && !f.out.contains(&Outcome::WakeAfter(Duration::ZERO)));

    // Back at the edge, it glides to the end of the content and stops there.
    m.moved(at_view(&f, VIEW), &mut f);
    for _ in 0..1_500 {
        frame(&mut f, &mut m, &mut glide);
    }
    let bottom = 1_000.0 * ROW - VIEW;
    assert!((f.visible.y - bottom).abs() < 1.0);
    f.out.clear();
    frame(&mut f, &mut m, &mut glide);
    assert!(glides(&f.out).is_empty(), "nothing more to scroll");
    // The top edge doesn't scroll a view at the top.
    let mut f = fake(1_000);
    let mut m = machine();
    let mut glide = Glide::default();
    m.begin(at_row(5), Mods::default(), &mut f);
    m.moved(at_view(&f, 0.0), &mut f);
    frame(&mut f, &mut m, &mut glide);
    assert!(glides(&f.out).is_empty());
}

#[test]
fn a_still_band_asks_for_no_frames() {
    // Rune warns when a resting window keeps waking; a held, still band costs nothing.
    let mut f = fake(100);
    let mut m = machine();
    m.begin(at_row(2), Mods::default(), &mut f);
    m.moved(at_row(6), &mut f);
    f.out.clear();
    f.now += 1.0;
    assert!(!m.tick(&mut f));
    assert!(f.out.is_empty());
}

#[test]
fn wheel_scrolling_during_a_held_band_extends_it_from_the_same_anchor() {
    // browser/selection/marquee: Wheel scrolling during a held marquee extends the selection without moving the anchor.
    let mut f = fake(1_000);
    let mut m = machine();
    m.begin(at_row(3), Mods::default(), &mut f);
    m.moved(at_view(&f, 300.0), &mut f);
    assert_eq!(f.sel.len(), 8, "rows 3 to 10");
    // Wheel ticks scroll 1,000 pt over four frames under a still pointer.
    for _ in 0..4 {
        f.now += FRAME;
        f.visible.y += 250.0;
        m.tick(&mut f);
    }
    // The scroll stops; the next frame applies what waited.
    f.now += FRAME;
    m.tick(&mut f);
    let pointer_row = ((1000.0 + 300.0) / ROW) as u64;
    assert_eq!(selected(&f), (3..=pointer_row).collect::<Vec<_>>());
}

#[test]
fn rows_scrolled_out_stay_selected_and_rows_above_the_anchor_stay_unselected() {
    // browser/selection/marquee: Entries selected before they scrolled out of view stay selected; entries above the original anchor stay unselected.
    let mut f = fake(1_000);
    let mut m = machine();
    m.begin(at_row(10), Mods::default(), &mut f);
    f.visible.y = 2_000.0;
    m.moved(at_view(&f, 200.0), &mut f);
    f.now += 1.0;
    m.tick(&mut f);
    let below = ((2_000.0 + 200.0) / ROW) as u64;
    assert_eq!(selected(&f), (10..=below).collect::<Vec<_>>());
    // Back up past the anchor: the rows between the anchor and the pointer, and none below.
    f.visible.y = 0.0;
    m.moved(at_row(4), &mut f);
    f.now += 1.0;
    m.tick(&mut f);
    assert_eq!(selected(&f), (4..=10).collect::<Vec<_>>());
}

#[test]
fn the_release_and_escape_stop_the_view_dead() {
    // S1 (review): Strata's auto-scroll timer stops with the band, so the view stays where it
    // is and the last row the band took stays under the pointer. A soft stop from leaving the
    // edge just before is cut short too.
    for (how, leave_first) in [("end", false), ("cancel", false), ("end", true)] {
        let mut f = fake(10_000);
        let mut m = machine();
        let mut glide = Glide::default();
        m.begin(at_row(1), Mods::default(), &mut f);
        m.moved(at_view(&f, VIEW - 2.0), &mut f);
        for _ in 0..60 {
            frame(&mut f, &mut m, &mut glide);
        }
        if leave_first {
            m.moved(at_view(&f, VIEW - 40.0), &mut f);
            frame(&mut f, &mut m, &mut glide);
        }
        let pointer_y = if leave_first { VIEW - 40.0 } else { VIEW - 2.0 };
        let at = f.visible.y;
        if how == "end" {
            m.end(&mut f);
        } else {
            assert!(m.cancel(&mut f));
        }
        assert!(
            f.out.contains(&Outcome::StopScroll { y: at }),
            "{how}: {:?}",
            f.out
        );
        assert!(
            glides(&f.out).is_empty() && stops(&f.out) == 0,
            "no glide after it"
        );
        for _ in 0..60 {
            frame(&mut f, &mut m, &mut glide);
        }
        assert!(
            (f.visible.y - at).abs() < 1.0,
            "{how}: moved {}",
            f.visible.y - at
        );
        if how == "end" {
            let under = ((f.visible.y + pointer_y) / ROW) as u64;
            assert_eq!(
                selected(&f).last(),
                Some(&under),
                "the row under the pointer"
            );
        }
    }
}

#[test]
fn with_animations_off_the_edge_scroll_keeps_the_curves_speed() {
    // S2 (review): under reduced motion Rune lands each glide at once. With the host's
    // `animations` off, each step aims one frame's travel ahead, so the speed stays the curve's.
    let mut f = fake(100_000);
    f.settings.animations = false;
    let mut m = machine();
    let mut glide = Glide::default();
    m.begin(at_row(1), Mods::default(), &mut f);
    m.moved(at_view(&f, VIEW), &mut f);
    let mut frames = Vec::new();
    for _ in 0..60 {
        f.now += FRAME;
        m.tick(&mut f);
        let before = f.visible.y;
        scroll(&mut f, &mut glide, true);
        frames.push((f.visible.y - before) / FRAME as f32);
    }
    let most = EdgeScroll::STRATA.max_speed;
    assert!(
        frames[1..].iter().all(|v| (v - most).abs() < 0.02 * most),
        "{frames:?}"
    );
}

#[test]
fn with_animations_off_leaving_the_edge_stops_where_the_view_is() {
    // Review round 2: Rune lands a glide at once under reduced motion, so a soft stop aimed
    // ahead would jump the view; with animations off it aims where the view is.
    for animations in [true, false] {
        let mut f = fake(10_000);
        f.settings.animations = animations;
        let mut m = machine();
        let mut glide = Glide::default();
        m.begin(at_row(1), Mods::default(), &mut f);
        m.moved(at_view(&f, VIEW), &mut f);
        for _ in 0..30 {
            frame(&mut f, &mut m, &mut glide);
        }
        m.moved(at_view(&f, VIEW / 2.0), &mut f);
        let to = f.out.iter().find_map(|o| match o {
            Outcome::StopGlide { y, .. } => Some(*y),
            _ => None,
        });
        let to = to.expect("a soft stop");
        if animations {
            assert!(to > f.visible.y + 1.0, "with animations it eases on");
        } else {
            assert_eq!(to, f.visible.y, "without, it stays");
        }
    }
}

#[test]
fn below_sixty_hertz_the_band_waits_for_frames_too() {
    // D1: Rune's warning counts frames, so at 30 Hz the band applies once in 6 frames (0.2 s),
    // not every 100 ms.
    let mut f = fake(100_000);
    let mut m = machine();
    let mut glide = Glide::default();
    m.begin(at_row(1), Mods::default(), &mut f);
    m.moved(at_view(&f, VIEW - 2.0), &mut f);
    let start = f.applied.len();
    for _ in 0..60 {
        frame_of(&mut f, &mut m, &mut glide, 1.0 / 30.0);
    }
    let times: Vec<f64> = f.applied[start..].iter().map(|(t, _)| *t).collect();
    assert!(times.len() >= 5, "it still extends: {times:?}");
    assert!(
        times.windows(2).all(|w| w[1] - w[0] >= 6.0 / 30.0 - 1e-9),
        "{times:?}"
    );
}

// The adapter on C1e's seam, through a `Ctx` with no runtime.

mod adapter {
    use std::rc::Rc;

    use rune::Mods;
    use strata_kit::look::Look;

    use super::{ROW, VIEW, WIDTH, at_row, entry};
    use crate::collection::element::{Ctx, Effect, MarqueeController, Settings};
    use crate::collection::marquee::{EdgeScroll, Marquee, MarqueeSettings};
    use crate::collection::selection::Selection;
    use crate::contract::{ListSource, Point, Rect, UniformRows};

    /// Runs `f` with a context over 1,000 rows at the top, with the host's `settings`.
    fn with_cx<R>(
        sel: &mut Selection,
        settings: Option<MarqueeSettings>,
        now: f64,
        f: impl FnOnce(&mut Ctx<'_>) -> R,
    ) -> (R, Vec<Effect>) {
        with_cx_at(sel, settings, now, 0.0, f)
    }

    /// As [`with_cx`], with the view scrolled to content y `top`.
    fn with_cx_at<R>(
        sel: &mut Selection,
        settings: Option<MarqueeSettings>,
        now: f64,
        top: f32,
        f: impl FnOnce(&mut Ctx<'_>) -> R,
    ) -> (R, Vec<Effect>) {
        let source = ListSource::new((0..1_000).map(entry).collect());
        let geometry = UniformRows::new(1_000, ROW, WIDTH).viewport_height(VIEW);
        let mut values = Settings::default();
        if let Some(s) = settings {
            values.insert(s);
        }
        let visible = Rect::new(0.0, top, WIDTH, VIEW);
        let mut cx = Ctx::new(&source, &geometry, visible, now, sel).with_settings(Rc::new(values));
        let r = f(&mut cx);
        let effects = cx.effects().to_vec();
        (r, effects)
    }

    fn glides(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Glide { .. }))
            .count()
    }

    #[test]
    fn the_adapter_reads_the_collections_settings() {
        // D6: the host's MarqueeSettings, through `Collection::settings`, reach the machine. A
        // pointer 160 pt above the bottom glides only with a 200 pt edge band.
        let wide = MarqueeSettings {
            edge: EdgeScroll {
                edge: 200.0,
                max_speed: 1000.0,
            },
            ..MarqueeSettings::default()
        };
        for (settings, glide) in [(None, false), (Some(wide), true)] {
            let mut sel = Selection::new();
            let mut m = Marquee::new();
            with_cx(&mut sel, settings, 0.0, |cx| {
                m.begin(at_row(1), Mods::default(), cx);
                m.moved(Point::new(300.0, VIEW - 160.0), cx);
            });
            let (_, effects) = with_cx(&mut sel, settings, 1.0 / 60.0, |cx| m.tick(cx));
            assert_eq!(glides(&effects) > 0, glide, "{settings:?}");
        }
    }

    #[test]
    fn the_band_draws_while_open_and_escape_hides_it_and_stops_the_glide() {
        // D2 through the seam: Escape restores the selection at press and stops the glide.
        let look = Look::fallback(false);
        let mut sel = Selection::new();
        let mut m = Marquee::new();
        assert!(m.overlay(&look).is_none());
        with_cx(&mut sel, None, 0.0, |cx| {
            m.begin(at_row(2), Mods::default(), cx);
            m.moved(Point::new(300.0, VIEW - 1.0), cx);
        });
        assert!(m.active() && m.overlay(&look).is_some());
        let (_, effects) = with_cx(&mut sel, None, 1.0 / 60.0, |cx| m.tick(cx));
        assert_eq!(glides(&effects), 1, "it glides at the edge");
        assert!(!sel.is_empty());
        let (taken, effects) = with_cx(&mut sel, None, 2.0 / 60.0, |cx| m.cancel(cx));
        assert!(taken);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::ScrollTo { y, glide: false } if *y == 0.0)),
            "a dead stop where the view is: {effects:?}"
        );
        assert!(sel.is_empty(), "the plain band's press had cleared it");
        assert!(!m.active() && m.overlay(&look).is_none());
    }

    #[test]
    fn a_band_wholly_away_from_the_view_draws_nothing() {
        // S5 (review): when nothing of the band is near the view, the drawn rect goes, rather
        // than staying where the band last drew.
        let mut sel = Selection::new();
        let mut m = Marquee::new();
        with_cx(&mut sel, None, 0.0, |cx| {
            m.begin(Point::new(100.0, 13.0), Mods::default(), cx);
            m.moved(at_row(10), cx);
        });
        let [_, _, w, h] = m.drawn_rect();
        assert!(w * h > 0.0, "it draws near the view");
        // Scrolled 20,000 pt down by hand, with the pointer two viewports above the window:
        // the band ends 1,120 pt above the view, more than a viewport away.
        let top = 20_000.0;
        with_cx_at(&mut sel, None, 1.0 / 60.0, top, |cx| {
            m.moved(Point::new(300.0, top - 2.0 * VIEW), cx);
        });
        assert!(m.band().is_some(), "the band is still open");
        let [_, _, w, h] = m.drawn_rect();
        assert_eq!(w * h, 0.0, "nothing drawn");
    }

    #[test]
    fn a_key_that_takes_the_view_holds_the_edge_scroll_until_the_pointer_moves() {
        // C1d's S-B (C2's E6): after `yield_scroll`, ticks glide no more until the band's
        // pointer next moves.
        let mut sel = Selection::new();
        let mut m = Marquee::new();
        with_cx(&mut sel, None, 0.0, |cx| {
            m.begin(at_row(2), Mods::default(), cx);
            m.moved(Point::new(300.0, VIEW - 1.0), cx);
        });
        let (_, effects) = with_cx(&mut sel, None, 1.0 / 60.0, |cx| m.tick(cx));
        assert_eq!(glides(&effects), 1, "it glides at the edge");
        assert!(m.yield_scroll(), "it was moving the view");
        let (_, effects) = with_cx(&mut sel, None, 2.0 / 60.0, |cx| m.tick(cx));
        assert_eq!(glides(&effects), 0, "held after the key");
        with_cx(&mut sel, None, 3.0 / 60.0, |cx| {
            m.moved(Point::new(300.0, VIEW - 2.0), cx);
        });
        let (_, effects) = with_cx(&mut sel, None, 4.0 / 60.0, |cx| m.tick(cx));
        assert_eq!(glides(&effects), 1, "again once the pointer moves");
    }
}
