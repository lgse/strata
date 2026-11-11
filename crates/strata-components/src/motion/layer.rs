//! [`MotionLayer`]: the element the effects draw in, over the whole window.

use std::rc::Rc;

use rune::{Cx, El, Spring, SpringPair, Transition, el, memo};
use strata_kit::look::Look;

use super::dissolve::{self, DissolveRun};
use super::flight::{self, FlightRun, Mode, Phase};
use super::model::Effects;
use super::pulse;

/// The layer's key.
pub const LAYER: &str = "c23-motion";

type GhostFn<S> = Rc<dyn Fn(&S, u64, &Look) -> El>;

/// The overlay the effects draw in. Mount it as the last child of the view's root: it pins
/// itself to the whole window, takes no input and is hidden from assistive technology.
///
/// Used by: hosts (Strata's window, the gallery).
pub struct MotionLayer<S> {
    get: Rc<dyn Fn(&S) -> &Effects>,
    ghost: Option<GhostFn<S>>,
}

impl<S: 'static> MotionLayer<S> {
    /// The layer over the effects `get` reads.
    pub fn new(get: impl Fn(&S) -> &Effects + 'static) -> Self {
        Self {
            get: Rc::new(get),
            ghost: None,
        }
    }

    /// How a dissolving row looks, for the token its [`Dissolve`](super::Dissolve) row
    /// carries: its icon, label and selection fill at the row's size, as the collection drew
    /// it. Fragments are pieces of it. Without one, fragments are flat shards.
    pub fn ghost(mut self, f: impl Fn(&S, u64, &Look) -> El + 'static) -> Self {
        self.ghost = Some(Rc::new(f));
        self
    }

    /// The layer, for state `s`.
    pub fn build(&self, s: &S, look: &Look, cx: &mut Cx<S>) -> El {
        let fx = (self.get)(s);
        let root = el()
            .key(LAYER)
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .inert()
            .a11y_hidden();
        if !fx.playing() {
            return root;
        }
        let [w, h] = cx.layout_size(LAYER).unwrap_or([0.0, 0.0]);
        let window = [0.0, 0.0, w, h];
        // Rune's own motion is off: a matched pair would cross-fade in place, and live values
        // ignore it, so nothing draws and every effect ends at its next ticks.
        let reduced = cx.is_reduced_motion();
        let mut out = Drawn::default();
        for f in &fx.flights {
            flight_view(f, fx, look, cx, window, reduced, &mut out);
        }
        let mut out = out.els;
        if let Some(d) = &fx.dissolve {
            let ghost = self.ghost.as_ref().map(|g| (g.clone(), s));
            out.extend(dissolve_view(d, ghost, look, cx, window, reduced));
        }
        root.children(out)
    }
}

/// Whether `rect` shows in the window and, when given, in the viewport `within` names.
fn shows<S>(rect: [f32; 4], window: [f32; 4], within: Option<&str>, cx: &Cx<S>) -> bool {
    let clip = match within {
        Some(key) => match cx.window_layout(key) {
            Some(v) => intersect(window, v),
            None => return false,
        },
        None => Some(window),
    };
    clip.and_then(|c| intersect(c, rect)).is_some()
}

/// Whether the middle of `rect` shows in the window and, when given, in `within`: a target
/// scrolled out of its viewport, or off the window, takes no flight.
fn target_shows<S>(rect: [f32; 4], window: [f32; 4], within: Option<&str>, cx: &Cx<S>) -> bool {
    let mid = [rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0, 0.0, 0.0];
    let inside = |c: [f32; 4]| {
        mid[0] >= c[0] && mid[0] <= c[0] + c[2] && mid[1] >= c[1] && mid[1] <= c[1] + c[3]
    };
    inside(window)
        && match within {
            Some(key) => cx.window_layout(key).is_some_and(inside),
            None => true,
        }
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> Option<[f32; 4]> {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    let r = (a[0] + a[2]).min(b[0] + b[2]);
    let btm = (a[1] + a[3]).min(b[1] + b[3]);
    (r > x && btm > y).then_some([x, y, r - x, btm - y])
}

/// What the flights draw.
#[derive(Default)]
struct Drawn {
    els: Vec<El>,
    /// The targets whose ring is drawn: flights into one target share its latest pulse, and
    /// so its ring's key.
    rung: Vec<String>,
}

fn flight_view<S>(
    f: &FlightRun,
    fx: &Effects,
    look: &Look,
    cx: &mut Cx<S>,
    window: [f32; 4],
    reduced: bool,
    out: &mut Drawn,
) {
    let target = cx
        .window_layout(&f.target)
        .filter(|&r| target_shows(r, window, f.target_in.as_deref(), cx));
    f.target_seen.set(target.is_some());
    let size = look.icon_size.md;
    // Where a leg's row side is, while it shows.
    let row_rect = |cx: &Cx<S>, key: &str| {
        cx.window_layout(key)
            .filter(|&r| shows(r, window, f.rows_in.as_deref(), cx))
            .map(|r| flight::meet(r, [size.min(r[2]), size.min(r[3])]))
    };
    match f.phase {
        Phase::Mount { .. } => {
            f.viewed.set(true);
            for (i, leg) in f.legs.iter().enumerate() {
                let rect = if reduced {
                    None
                } else {
                    match f.mode {
                        Mode::Inbound => target.and(row_rect(cx, &leg.row)),
                        // The target shakes as they lift: one out of sight flies nothing.
                        Mode::Release => target.and(row_rect(cx, &leg.row)),
                        Mode::Outbound => {
                            let to = row_rect(cx, &leg.row);
                            target
                                .zip(to)
                                .map(|(t, to)| flight::meet(t, [to[2], to[3]]))
                        }
                    }
                };
                leg.seen.set(rect);
                if let Some(rect) = rect {
                    out.els.push(
                        flight::launch(f.id, i, look, leg.icon, rect)
                            .transition(Transition::none()),
                    );
                }
            }
        }
        Phase::Fly { landed, .. } => {
            if reduced {
                return;
            }
            if landed.is_none() {
                let n = f.flying.len();
                for (k, &i) in f.flying.iter().enumerate() {
                    let leg = &f.legs[i];
                    let Some(seen) = leg.seen.get() else { continue };
                    let rect = match f.mode {
                        Mode::Inbound => target.map(|t| flight::meet(t, [seen[2], seen[3]])),
                        Mode::Outbound => row_rect(cx, &leg.row),
                        Mode::Release => Some(flight::release_end(seen, k, n)),
                    };
                    if let Some(rect) = rect {
                        out.els.push(
                            flight::flyer(f.id, i, k, f.mode, look, leg.icon, rect)
                                .transition(quick_fade()),
                        );
                    }
                }
                if f.mode == Mode::Inbound
                    && let Some(t) = target
                {
                    out.els.push(pulse::receiving(look, f.id.0, t));
                }
            }
            let pulsing = landed.is_some() || !f.mode.pulse().1;
            if pulsing
                && !out.rung.contains(&f.target)
                && let Some(t) = target
                && let Some(ring) = pulse::pulse_ring(look, fx, &f.target, t)
            {
                out.rung.push(f.target.clone());
                out.els.push(ring);
            }
        }
    }
}

/// A flyer taken away mid-flight (its target went) fades out quickly where it is.
fn quick_fade() -> Transition {
    let s = Spring::new(0.12, 0.0);
    Transition::opacity().springs(SpringPair::new(s, s))
}

fn dissolve_view<S>(
    d: &DissolveRun,
    ghost: Option<(GhostFn<S>, &S)>,
    look: &Look,
    cx: &mut Cx<S>,
    window: [f32; 4],
    reduced: bool,
) -> Option<El> {
    // A key's first layout read is `None` until the commit's layout re-runs the view, so the
    // plan follows every run while the fragments mount, and holds once they play.
    if matches!(d.phase, dissolve::Phase::Mount { .. }) || d.plan.borrow().is_none() {
        let clip = match &d.within {
            Some(key) => cx.window_layout(key).and_then(|v| intersect(window, v)),
            None => Some(window),
        };
        let rects = match clip {
            Some(c) if !reduced => d
                .rows
                .iter()
                .enumerate()
                .filter_map(|(i, (key, _))| {
                    let r = cx.window_layout(key)?;
                    intersect(c, r).map(|_| (i, r))
                })
                .collect(),
            _ => Vec::new(),
        };
        let clip = d.within.as_ref().and(clip);
        *d.plan.borrow_mut() = Some(Rc::new(dissolve::plan(rects, clip)));
        d.plans.set(d.plans.get() + 1);
    }
    let plan = d.plan.borrow().clone()?;
    if plan.pieces.is_empty() || reduced {
        return None;
    }
    // Built once: the fragments then move by their live values alone.
    Some(memo(
        format!("c23-dissolve-{}", d.id.0),
        (d.id, d.plans.get()),
        || {
            let origin = plan.clip.map_or([0.0, 0.0], |c| [c[0], c[1]]);
            let ghosts: Vec<Option<El>> = plan
                .rows
                .iter()
                .map(|&(i, _)| ghost.as_ref().map(|(g, s)| g(s, d.rows[i].1, look)))
                .collect();
            let pieces = (0..plan.pieces.len()).map(|i| {
                dissolve::piece(&plan, i, origin, ghosts[plan.pieces[i].row].as_ref(), look)
            });
            let mut group = el().key(format!("c23-dissolve-{}", d.id.0));
            group = match plan.clip {
                Some(c) => group.absolute(c[0], c[1]).size(c[2], c[3]).clip_soft(0.0),
                None => group.pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)),
            };
            group
                .transition(Transition::none())
                .children(pieces.collect::<Vec<_>>())
        },
    ))
}
