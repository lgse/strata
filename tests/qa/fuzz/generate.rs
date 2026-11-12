// SPDX-License-Identifier: MIT

//! From a seed to a sequence. The generator never looks at the application: the same seed,
//! alphabet version and generator version write the same file for every target and on every
//! machine, and a step's `when` is what lets each target skip what it cannot do.

use super::{
    alphabet::{Action, Alphabet},
    random::Random,
    sequence::{PREFIX, Sequence},
};

/// Raised by every change here that makes a seed write another sequence.
pub(crate) const GENERATOR: u32 = 1;

/// `fuzz-<area>-<seed>`, the seed as sixteen hex digits: a scenario name, so the seed names
/// the file a sequence is kept in.
pub(crate) fn name(area: &str, seed: u64) -> String {
    format!("{PREFIX}-{area}-{seed:016x}")
}

/// The seed a sequence name ends with.
pub(crate) fn seed_of(name: &str) -> Option<u64> {
    let digits = name.rsplit('-').next()?;
    (digits.len() == 16)
        .then(|| u64::from_str_radix(digits, 16).ok())
        .flatten()
}

/// `actions` picks from the alphabet. A longer sequence of one seed starts with the shorter
/// one: each action draws its numbers in turn and nothing is drawn ahead.
pub(crate) fn generate(alphabet: &Alphabet, seed: u64, actions: usize) -> Sequence {
    let mut random = Random::new(seed);
    let total = alphabet.weight();
    let mut steps = Vec::new();
    for _ in 0..actions {
        let action = pick(alphabet, random.below(total));
        // One draw for each name, in name order, whether or not a step uses it twice.
        let chosen: Vec<(&String, &str)> = action
            .choose
            .iter()
            .map(|(name, values)| {
                let index = usize::try_from(random.below(values.len() as u64))
                    .expect("an index below a list's length fits");
                (name, values[index].as_str())
            })
            .collect();
        for step in &action.steps {
            steps.push(action.fill(step, &|slot, _| {
                chosen
                    .iter()
                    .find(|(name, _)| name.as_str() == slot)
                    .map_or("", |(_, value)| *value)
            }));
        }
    }
    let name = name(&alphabet.area, seed);
    Sequence {
        header: alphabet.header_line(&name),
        name,
        steps,
    }
}

/// The action whose share of the weights holds `drawn`.
fn pick(alphabet: &Alphabet, drawn: u64) -> &Action {
    let mut passed = 0;
    alphabet
        .actions
        .iter()
        .find(|action| {
            passed += u64::from(action.weight);
            drawn < passed
        })
        .expect("a draw below the total weight falls in some action")
}
