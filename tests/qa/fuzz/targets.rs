// SPDX-License-Identifier: MIT

//! Which targets a command replays on, and what of their replays is compared with what.
//!
//! On one target, every replay of a sequence is compared with the first, which is what
//! calibration reads. On GTK and Rune, the differential mode, each target is first compared
//! with itself in the same way, and then every Rune replay with the first GTK one, with the
//! normalisation map applied: a field on which a target does not agree with itself cannot
//! be compared with the other target, so the comparisons with itself come first.

use super::{
    compare::{Divergence, compare},
    replay::Stream,
    rules::Ignored,
};
use crate::target::Target;

/// The targets a command replays on: one, or GTK and Rune, GTK first since it is the
/// reference every Rune replay is held to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Targets(Vec<Target>);

impl Targets {
    pub(crate) fn one(target: Target) -> Self {
        Self(vec![target])
    }

    /// `gtk`, `rune`, or `gtk,rune` for the differential mode.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        match text {
            "gtk" => Ok(Self::one(Target::Gtk)),
            "rune" => Ok(Self::one(Target::Rune)),
            "gtk,rune" => Ok(Self(vec![Target::Gtk, Target::Rune])),
            "rune,gtk" => Err(
                "--target gtk,rune names GTK first: it is the reference every Rune replay is \
                 compared with"
                    .to_owned(),
            ),
            other => Err(format!("--target is gtk, rune or gtk,rune, not {other:?}")),
        }
    }

    pub(crate) fn all(&self) -> &[Target] {
        &self.0
    }

    pub(crate) fn is_differential(&self) -> bool {
        self.0.len() > 1
    }

    /// The reference target, and the one held to it; one target is both.
    pub(crate) fn pair(&self) -> (Target, Target) {
        let first = self.0[0];
        (first, *self.0.last().unwrap_or(&first))
    }

    /// Which replays a shrink compares: the two targets, or the one with itself.
    pub(crate) fn between(&self) -> String {
        let (reference, other) = self.pair();
        between(reference.name(), other.name())
    }

    /// The target whose last replay a perturbing command changes.
    pub(crate) fn last(&self) -> Target {
        self.pair().1
    }

    /// How the artifacts' folder is named: `gtk`, `rune` or `gtk-rune`, so that a run on one
    /// target never removes what a run on another kept.
    pub(crate) fn label(&self) -> String {
        self.names().join("-")
    }

    /// `target gtk`, or `targets gtk and rune`.
    pub(crate) fn describe(&self) -> String {
        match self.names().as_slice() {
            [one] => format!("target {one}"),
            names => format!("targets {}", names.join(" and ")),
        }
    }

    fn names(&self) -> Vec<&'static str> {
        self.0.iter().map(|target| target.name()).collect()
    }
}

/// One comparison of a sequence's replays: which replays it held to which, and where they
/// first differ, as the map and the mask leave it and with nothing left out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Comparison {
    /// `on gtk`, `on rune`, or `gtk against rune`.
    pub(crate) between: String,
    /// Whether it holds one target's replays to the other's: a difference then is a
    /// divergence to triage, never noise for the calibration mask.
    pub(crate) across: bool,
    pub(crate) divergence: Option<Divergence>,
    pub(crate) unmasked: Option<Divergence>,
    /// Pairs of observations the comparison could read: the shorter stream of each pair.
    pub(crate) compared: usize,
}

/// What the comparisons of a sequence leave out: `same` holds the mask alone, for replays of
/// one target, and `across` the map and the mask, for replays of two.
pub(crate) struct Left<'a> {
    pub(crate) same: &'a Ignored,
    pub(crate) across: &'a Ignored,
}

/// Every comparison of a sequence's streams, each target with itself first, in the order
/// `targets` lists them, then each Rune stream with the first GTK one.
pub(crate) fn comparisons(streams: &[Stream], targets: &Targets, left: &Left) -> Vec<Comparison> {
    let of = |target: Target| -> Vec<&Stream> {
        streams
            .iter()
            .filter(|stream| stream.header.target == target.name())
            .collect()
    };
    let mut comparisons: Vec<Comparison> = targets
        .all()
        .iter()
        .filter_map(|&target| {
            let replays = of(target);
            let (first, others) = replays.split_first()?;
            Some(held_to(
                between(target.name(), target.name()),
                (first, others),
                left.same,
                false,
            ))
        })
        .collect();
    let (reference, other) = targets.pair();
    if reference != other
        && let Some(first) = of(reference).first()
    {
        comparisons.push(held_to(
            between(reference.name(), other.name()),
            (first, &of(other)),
            left.across,
            true,
        ));
    }
    comparisons
}

/// How a line names the replays it compares: `on gtk` for one target's, `gtk against rune`
/// for two, the reference first.
pub(crate) fn between(reference: &str, other: &str) -> String {
    if reference == other {
        format!("on {reference}")
    } else {
        format!("{reference} against {other}")
    }
}

/// Every stream of `others` compared with `first`, the first that differs reported.
fn held_to(
    between: String,
    (first, others): (&Stream, &[&Stream]),
    ignored: &Ignored,
    across: bool,
) -> Comparison {
    let against = |ignored: &Ignored| {
        others
            .iter()
            .find_map(|other| compare(first, other, ignored))
    };
    Comparison {
        between,
        across,
        divergence: against(ignored),
        unmasked: against(&Ignored::default()),
        compared: others
            .iter()
            .map(|other| first.observations.len().min(other.observations.len()))
            .sum(),
    }
}
