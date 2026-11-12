// SPDX-License-Identifier: MIT

//! From a seed to a sequence. The generator never looks at the application: the same seed,
//! alphabet version and generator version write the same file for every target and on every
//! machine, and a step's `when` is what lets each target skip what it cannot do.

use super::{
    alphabet::{Action, Alphabet},
    random::Random,
    sequence::{PREFIX, Sequence},
};

/// Raised by every change here that makes a seed write another sequence. It is part of
/// what an alphabet's version is pinned to, so every alphabet's version rises with it.
pub(crate) const GENERATOR: u32 = 1;

/// Where a sequence came from, which is what its name says:
/// `fuzz-<area>-v<version>-<seed>`, the alphabet's version and the seed as sixteen hex
/// digits. A seed means nothing without the version: another version of the alphabet makes
/// another sequence of the same seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Origin {
    pub(crate) area: String,
    pub(crate) version: u32,
    pub(crate) seed: u64,
}

impl Origin {
    pub(crate) fn name(&self) -> String {
        format!(
            "{PREFIX}-{}-v{}-{:016x}",
            self.area, self.version, self.seed
        )
    }

    /// The origin a sequence name holds, when it is one a seed could have written.
    pub(crate) fn of(name: &str) -> Option<Self> {
        let rest = name.strip_prefix(PREFIX)?.strip_prefix('-')?;
        let (rest, seed) = rest.rsplit_once('-')?;
        let (area, version) = rest.rsplit_once("-v")?;
        let origin = Self {
            area: area.to_owned(),
            version: version.parse().ok().filter(|version| *version > 0)?,
            seed: (seed.len() == 16)
                .then(|| u64::from_str_radix(seed, 16).ok())
                .flatten()?,
        };
        (!origin.area.is_empty() && origin.name() == name).then_some(origin)
    }
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
    let name = Origin {
        area: alphabet.area.clone(),
        version: alphabet.version,
        seed,
    }
    .name();
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
