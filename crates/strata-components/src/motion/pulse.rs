//! The target's pulses (Strata's `trash-receiving`, `trash-impact`, `trash-release` and
//! `trash-rebel-shudder`): a scale on the target itself, through [`dress_target`], and an
//! accent ring over it in the motion layer.

use rune::{Beat, Easing, El, Sequence, el};
use strata_kit::look::Look;

use super::model::Effects;

/// How long the impact and release pulses play (Strata's `BOUNCE`).
pub const BOUNCE: f64 = 0.28;
/// How long the shake plays (Strata's `trash-rebel-shudder`).
pub const SHAKE: f64 = 0.24;
/// How long the receiving ring takes to light while icons fly in (`trash-receiving`).
pub const RECEIVING: f64 = 0.42;

/// What the target plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PulseKind {
    /// Icons landed in it: it swells and settles, and a ring spreads and fades.
    Impact,
    /// Icons left it: it dips and swells, and a softer ring spreads.
    Release,
    /// Icons lifted off rows in its own view: it shudders (in scale only: U-C23-2).
    Shake,
}

impl PulseKind {
    pub(super) fn duration(self) -> f64 {
        match self {
            PulseKind::Impact | PulseKind::Release => BOUNCE,
            PulseKind::Shake => SHAKE,
        }
    }

    fn target_beats(self) -> &'static [Beat] {
        match self {
            PulseKind::Impact => IMPACT,
            PulseKind::Release => RELEASE,
            PulseKind::Shake => SHUDDER,
        }
    }

    fn ring_beats(self) -> Option<&'static [Beat]> {
        match self {
            PulseKind::Impact => Some(IMPACT_RING),
            PulseKind::Release => Some(RELEASE_RING),
            PulseKind::Shake => None,
        }
    }
}

// Strata's keyframes (`style.css`), each interval eased as its animation is.
const SPRINGY: Easing = Easing::cubic(0.2, 0.9, 0.3, 1.2);
const SETTLE: Easing = Easing::cubic(0.23, 1.0, 0.32, 1.0);
const IMPACT: &[Beat] = &[
    Beat::new(0.0).scale(0.96),
    Beat::new(BOUNCE * 0.34).scale(1.14).easing(SPRINGY),
    Beat::new(BOUNCE * 0.68).scale(0.97).easing(SPRINGY),
    Beat::new(BOUNCE).easing(SPRINGY),
];
const RELEASE: &[Beat] = &[
    Beat::new(0.0).scale(0.94),
    Beat::new(BOUNCE * 0.45).scale(1.08).easing(SETTLE),
    Beat::new(BOUNCE).easing(SETTLE),
];
// Strata's shudder also turns ±8°; a Beat has no rotation (U-C23-2).
const SHUDDER: &[Beat] = &[
    Beat::new(0.0),
    Beat::new(SHAKE * 0.18).scale(0.92).easing(SPRINGY),
    Beat::new(SHAKE * 0.38).scale(1.14).easing(SPRINGY),
    Beat::new(SHAKE * 0.58).scale(0.95).easing(SPRINGY),
    Beat::new(SHAKE * 0.78).scale(1.05).easing(SPRINGY),
    Beat::new(SHAKE).easing(SPRINGY),
];
// The rings' opacities are the accent alphas of Strata's box shadows, and their scales its
// spreads: impact 0.34 at 1 px, 0.18 at 5 px, gone by 8 px; release 0.16 at 4 px, then only a
// glow; receiving 0.16 at 3 px at 55 %, then 0.28 at 1 px.
const IMPACT_RING: &[Beat] = &[
    Beat::new(0.0).opacity(0.34),
    Beat::new(BOUNCE * 0.34)
        .opacity(0.18)
        .scale(1.25)
        .easing(SPRINGY),
    Beat::new(BOUNCE * 0.68)
        .opacity(0.0)
        .scale(1.4)
        .easing(SPRINGY),
    Beat::new(BOUNCE).opacity(0.0).scale(1.45).easing(SPRINGY),
];
const RELEASE_RING: &[Beat] = &[
    Beat::new(0.0).opacity(0.16).scale(1.15),
    Beat::new(BOUNCE * 0.45)
        .opacity(0.2)
        .scale(1.3)
        .easing(SETTLE),
    Beat::new(BOUNCE).opacity(0.0).scale(1.3).easing(SETTLE),
];
const RECEIVING_RING: &[Beat] = &[
    Beat::new(0.0).opacity(0.0),
    Beat::new(RECEIVING * 0.55)
        .opacity(0.16)
        .scale(1.1)
        .easing(SETTLE),
    Beat::new(RECEIVING).opacity(0.28).easing(SETTLE),
];

/// Each target's pulses: how many it has started, and its latest one's kind and whether that
/// still plays. A new count is a new key, which replays a sequence.
#[derive(Debug, Default)]
pub(super) struct Pulses(Vec<Pulse>);

#[derive(Debug)]
struct Pulse {
    target: String,
    n: u32,
    kind: PulseKind,
    /// Its flight hasn't ended. Rune replays a sequence whenever its key mounts, so a spent
    /// pulse drops its sequence, or a target that remounts would play it again.
    live: bool,
}

impl Pulses {
    /// Starts a pulse of `kind` on `target`, and returns its count.
    pub fn start(&mut self, target: &str, kind: PulseKind) -> u32 {
        match self.0.iter_mut().find(|p| p.target == target) {
            Some(p) => {
                p.n += 1;
                p.kind = kind;
                p.live = true;
                p.n
            }
            None => {
                self.0.push(Pulse {
                    target: target.to_owned(),
                    n: 1,
                    kind,
                    live: true,
                });
                1
            }
        }
    }

    /// Ends `target`'s pulse `n`, unless a later one has started.
    pub fn spend(&mut self, target: &str, n: u32) {
        if let Some(p) = self.0.iter_mut().find(|p| p.target == target && p.n == n) {
            p.live = false;
        }
    }

    /// `target`'s latest pulse: its count, and its kind while it plays.
    fn last(&self, target: &str) -> Option<(u32, Option<PulseKind>)> {
        self.0
            .iter()
            .find(|p| p.target == target)
            .map(|p| (p.n, p.live.then_some(p.kind)))
    }
}

/// The target element `el`, keyed `key`, dressed for its pulses: pass C8's Trash icon box
/// through it (`PlacesSidebar::trash_icon(|el| dress_target(&fx, &key, el))`).
///
/// Until its first pulse it is `el` itself. From then on, for good, it sits in a wrapper keyed
/// by its pulse count, so each pulse mounts it afresh and plays the scale. Its element path
/// gains the wrapper's `c23-pulse-<n>` step. Once the pulse's flight ends (or
/// [`Effects::settle`] ends it), the wrapper stays but plays nothing, so a target that
/// remounts later (its sidebar shown again, its page reopened) doesn't replay it.
pub fn dress_target(fx: &Effects, key: &str, el: El) -> El {
    match fx.pulses.last(key) {
        None => el,
        Some((n, kind)) => {
            let wrapper = rune::el().key(format!("c23-pulse-{n}"));
            match kind {
                Some(kind) => wrapper.sequence(Sequence::new(kind.target_beats())),
                None => wrapper,
            }
            .child(el)
        }
    }
}

/// The ring's key over the target keyed `target` for its pulse `n`: one per pulse, so each
/// replays. The target is escaped (`_` as `__`, `/` as `_s`), so no two targets share one.
pub fn ring_key(target: &str, n: u32) -> String {
    let target = target.replace('_', "__").replace('/', "_s");
    format!("c23-ring-{target}-{n}")
}

fn ring(look: &Look, key: String, rect: [f32; 4], beats: &'static [Beat]) -> El {
    let k = &look.kit;
    let mut e = el()
        .key(key)
        .absolute(rect[0], rect[1])
        .size(rect[2], rect[3])
        .radius(k.radius.md)
        .border(1.5, k.c.icon_accent)
        .sequence(Sequence::new(beats));
    if look.glow.on {
        // The ring's opacity scales its glow too, to about Strata's glow alphas (0.12–0.22).
        e = e.shadow(0.0, 12.0, look.glow.color.alpha(1.0));
    }
    e
}

/// The ring over a target at `rect` that lights while icons fly in.
pub(super) fn receiving(look: &Look, flight: u64, rect: [f32; 4]) -> El {
    ring(
        look,
        format!("c23-receiving-{flight}"),
        rect,
        RECEIVING_RING,
    )
}

/// The ring of the target's latest pulse, while it plays and if it has one.
pub(super) fn pulse_ring(look: &Look, fx: &Effects, target: &str, rect: [f32; 4]) -> Option<El> {
    let (n, kind) = fx.pulses.last(target)?;
    Some(ring(look, ring_key(target, n), rect, kind?.ring_beats()?))
}
