// SPDX-License-Identifier: MIT

//! The shrinker: replays shorter and shorter sequences while the same divergence shows,
//! then confirms the shortest three times out of three.
//!
//! It knows nothing of targets: it is given the steps, the divergence, and a way to ask
//! whether some steps still diverge. A divergence that does not confirm is noise, and goes
//! to calibration, not to an issue.

use super::compare::Divergence;

/// How often the shortest sequence must diverge again, and may not fail to.
pub(crate) const CONFIRMATIONS: usize = 3;

/// Replays some steps on both sides and says where they diverge, if they do.
pub(crate) type Oracle<'a> = dyn FnMut(&[String]) -> Result<Option<Divergence>, String> + 'a;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shrunk {
    pub(crate) steps: Vec<String>,
    /// The divergence as the shortest sequence shows it.
    pub(crate) divergence: Divergence,
    /// How many times the oracle was asked.
    pub(crate) asked: usize,
    /// How many of `CONFIRMATIONS` replays of the shortest sequence diverged the same way.
    pub(crate) confirmed: usize,
}

impl Shrunk {
    pub(crate) fn is_confirmed(&self) -> bool {
        self.confirmed == CONFIRMATIONS
    }
}

struct Search<'a, 'b> {
    oracle: &'a mut Oracle<'b>,
    signature: String,
    asked: usize,
    limit: usize,
}

impl Search<'_, '_> {
    /// The divergence the steps show, when it is the one being shrunk.
    fn same(&mut self, steps: &[String]) -> Result<Option<Divergence>, String> {
        self.asked += 1;
        Ok((self.oracle)(steps)?.filter(|divergence| divergence.signature() == self.signature))
    }

    fn spent(&self) -> bool {
        self.asked >= self.limit
    }
}

/// Shrinks `steps`, which diverged as `found`, asking the oracle at most `limit` times
/// before the confirmations.
///
/// Steps after the divergence are dropped first, since nothing after it can matter. Then
/// runs of steps are taken out, halves first and single steps last, keeping every removal
/// after which the same field still differs first.
pub(crate) fn shrink(
    steps: &[String],
    found: &Divergence,
    limit: usize,
    oracle: &mut Oracle,
) -> Result<Shrunk, String> {
    let mut search = Search {
        oracle,
        signature: found.signature(),
        asked: 0,
        limit,
    };
    let mut divergence = found.clone();
    let mut current = steps[..found.step.min(steps.len())].to_vec();
    let mut chunks = 2;
    while current.len() >= 2 && !search.spent() {
        let size = current.len().div_ceil(chunks);
        let mut shorter = None;
        for start in (0..current.len()).step_by(size) {
            let end = (start + size).min(current.len());
            let mut candidate = current[..start].to_vec();
            candidate.extend_from_slice(&current[end..]);
            if let Some(still) = search.same(&candidate)? {
                candidate.truncate(still.step);
                shorter = Some((candidate, still));
                break;
            }
            if search.spent() {
                break;
            }
        }
        match shorter {
            Some((candidate, still)) => {
                current = candidate;
                divergence = still;
                chunks = (chunks - 1).max(2);
            }
            None if size == 1 => break,
            None => chunks = (chunks * 2).min(current.len()),
        }
    }
    let mut confirmed = 0;
    for _ in 0..CONFIRMATIONS {
        if let Some(again) = search.same(&current)? {
            divergence = again;
            confirmed += 1;
        }
    }
    Ok(Shrunk {
        steps: current,
        divergence,
        asked: search.asked,
        confirmed,
    })
}
