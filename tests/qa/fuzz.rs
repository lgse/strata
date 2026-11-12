// SPDX-License-Identifier: MIT

//! The fuzz driver: a seed picks a sequence of actions from an area's alphabet, the runner
//! replays it on a fresh fixture, and what a user could observe is read after every step,
//! once the application has come to rest (`docs/testing.md`, "Fuzzing").
//!
//! A sequence is a scenario file, so the ordinary loader checks it and the ordinary runner
//! can run it. What this adds is the generator, the observation at rest, the comparator over
//! two streams of observations and the shrinker. `./scripts/qa-fuzz.sh` is the way in.

#[path = "fuzz/alphabet.rs"]
pub(crate) mod alphabet;

#[path = "fuzz/command.rs"]
mod command;

#[path = "fuzz/compare.rs"]
pub(crate) mod compare;

#[path = "fuzz/compare_tests.rs"]
mod compare_tests;

#[path = "fuzz/corpus.rs"]
pub(crate) mod corpus;

#[path = "fuzz/generate.rs"]
pub(crate) mod generate;

#[path = "fuzz/generate_tests.rs"]
mod generate_tests;

#[path = "fuzz/observe.rs"]
pub(crate) mod observe;

#[path = "fuzz/observe_tests.rs"]
mod observe_tests;

#[path = "fuzz/options.rs"]
pub(crate) mod options;

#[path = "fuzz/perturb.rs"]
pub(crate) mod perturb;

#[path = "fuzz/project.rs"]
pub(crate) mod project;

#[path = "fuzz/random.rs"]
pub(crate) mod random;

#[path = "fuzz/replay.rs"]
pub(crate) mod replay;

#[path = "fuzz/report.rs"]
mod report;

#[path = "fuzz/rules.rs"]
pub(crate) mod rules;

#[path = "fuzz/sequence.rs"]
pub(crate) mod sequence;

#[path = "fuzz/shrink.rs"]
pub(crate) mod shrink;

#[path = "fuzz/tests.rs"]
mod tests;

pub(crate) use self::{command::main, tests::UNIT_TESTS};

/// Everything the fuzzer reads from the tree is under here: the alphabets, the corpus, the
/// normalisation map and the calibration mask.
pub(crate) const DATA: &str = "tests/qa/fuzz";
/// Where a run leaves its streams, transcripts and summaries, under the checkout.
pub(crate) const ARTIFACTS: &str = "target/qa-fuzz";
/// The first argument that sends the runner here instead of to its scenarios.
pub(crate) const ARGUMENT: &str = "fuzz";
