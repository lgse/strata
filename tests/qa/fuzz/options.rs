// SPDX-License-Identifier: MIT

//! What `./scripts/qa-fuzz.sh` passes on: a command and its selections.

use std::{path::PathBuf, time::Duration};

use super::{
    observe::{REST, Rest},
    perturb::Perturbation,
    targets::Targets,
};
use crate::target::Target;

pub(crate) const USAGE: &str = "\
usage: ./scripts/qa-fuzz.sh <command> [selections]

  run --alphabet AREA --seeds SEEDS   generate each seed's sequence, replay it --repeats
                                      times and compare the replays
  replay [--alphabet AREA] [FILE...]  the same for kept sequences: an area's corpus, every
                                      area's without a selection, or the files named
  replay --parked [--alphabet AREA]   a survey: the parked sequences, replayed on purpose
  shrink --alphabet AREA --seeds SEED shrink one seed's divergence, or a FILE's, and confirm
  shrink FILE                         it three times out of three
  generate --alphabet AREA --seeds SEED   print the sequence and replay nothing
  scenario FILE                       run a sequence file as the scenario it is
  compare FILE FILE                   compare two kept streams of one sequence
  check                               read every alphabet, the map, the mask and the corpus,
                                      and print their counts

  --alphabet AREA     an alphabet of tests/qa/fuzz/alphabets
  --seeds SEEDS       7, 1..200 (both ends) or 3,9,0x2a
  --length N          actions in a generated sequence (default 40)
  --repeats N         replays of each sequence (default 2)
  --jobs N            sequences replayed at once (default: one per CPU, and half that on
                      two targets; at most 256 replays at once, a display each)
  --target T          gtk or rune, the target every replay starts, or gtk,rune: the
                      differential mode, each sequence replayed on both side by side and
                      every Rune replay compared with the first GTK one (default: the
                      STRATA_QA_TARGET the script sets, else gtk)
  --keep              keep the streams of sequences that agree too
  --corpus            shrink: write a confirmed sequence into the area's corpus
  --limit N           shrink: replays tried before the confirmations (default 200)
  --perturb JSON      change one observation of the last replay on purpose, to prove the
                      comparator and the shrinker (docs/testing.md, \"Fuzzing\")
  --rest-samples N --rest-poll-ms N --rest-budget-ms N
                      how rest is told; calibration set the defaults
";

/// The X displays the runner has (`DISPLAY_TRIES` in `tests/qa.rs`): a replay holds one.
const MOST_AT_ONCE: usize = crate::DISPLAY_TRIES as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Run,
    Replay,
    Shrink,
    Generate,
    Scenario,
    Compare,
    Check,
}

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub(crate) command: Command,
    pub(crate) alphabet: Option<String>,
    pub(crate) seeds: Vec<u64>,
    pub(crate) length: usize,
    pub(crate) repeats: usize,
    pub(crate) jobs: usize,
    pub(crate) targets: Targets,
    pub(crate) keep: bool,
    /// `replay`: the parked sequences, and only those.
    pub(crate) parked: bool,
    pub(crate) corpus: bool,
    pub(crate) limit: usize,
    pub(crate) perturbation: Option<Perturbation>,
    pub(crate) rest: Rest,
    pub(crate) files: Vec<PathBuf>,
}

impl Options {
    pub(crate) fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut arguments = arguments.into_iter();
        let command = match arguments.next().as_deref() {
            Some("run") => Command::Run,
            Some("replay") => Command::Replay,
            Some("shrink") => Command::Shrink,
            Some("generate") => Command::Generate,
            Some("scenario") => Command::Scenario,
            Some("compare") => Command::Compare,
            Some("check") => Command::Check,
            Some(other) => return Err(format!("no fuzz command {other:?}")),
            None => return Err("a fuzz command is needed".to_owned()),
        };
        let mut options = Self {
            command,
            alphabet: None,
            seeds: Vec::new(),
            length: 40,
            repeats: 2,
            jobs: 0,
            targets: Targets::one(Target::from_environment()?),
            keep: false,
            parked: false,
            corpus: false,
            limit: 200,
            perturbation: None,
            rest: REST,
            files: Vec::new(),
        };
        while let Some(argument) = arguments.next() {
            let mut value = || {
                arguments
                    .next()
                    .ok_or_else(|| format!("{argument} needs a value"))
            };
            match argument.as_str() {
                "--alphabet" => options.alphabet = Some(value()?),
                "--seeds" | "--seed" => options.seeds = seeds(&value()?)?,
                "--length" => options.length = positive(&argument, &value()?)?,
                "--repeats" => options.repeats = positive(&argument, &value()?)?,
                "--jobs" => options.jobs = positive(&argument, &value()?)?,
                "--target" => options.targets = Targets::parse(&value()?)?,
                "--limit" => options.limit = positive(&argument, &value()?)?,
                "--keep" => options.keep = true,
                "--parked" => options.parked = true,
                "--corpus" => options.corpus = true,
                "--perturb" => options.perturbation = Some(Perturbation::parse(&value()?)?),
                "--rest-samples" => {
                    options.rest.samples = u32::try_from(positive(&argument, &value()?)?)
                        .map_err(|error| error.to_string())?;
                }
                "--rest-poll-ms" => options.rest.poll = milliseconds(&argument, &value()?)?,
                "--rest-budget-ms" => options.rest.budget = milliseconds(&argument, &value()?)?,
                flag if flag.starts_with('-') => return Err(format!("no option {flag}")),
                file => options.files.push(PathBuf::from(file)),
            }
        }
        if options.jobs == 0 {
            let processors = std::thread::available_parallelism().map_or(1, usize::from);
            options.jobs = (processors / options.targets.all().len()).max(1);
        }
        options.check()?;
        Ok(options)
    }

    /// Whether the command writes its sequences from seeds, and so takes no file.
    fn generates(&self) -> bool {
        matches!(self.command, Command::Run | Command::Generate)
            || (self.command == Command::Shrink && self.files.is_empty())
    }

    /// What is wrong with the selections for this command, the first fault found.
    fn check(&self) -> Result<(), String> {
        let faults = [self.sequence_faults(), self.option_faults()].concat();
        match faults.iter().find(|(fault, _)| *fault) {
            Some((_, message)) => Err((*message).to_owned()),
            None => Ok(()),
        }
    }

    /// Faults in which sequences the command names.
    fn sequence_faults(&self) -> Vec<(bool, &'static str)> {
        let generates = self.generates();
        let named = self.seeds.len() + self.files.len();
        let one_sequence = matches!(self.command, Command::Shrink | Command::Generate);
        vec![
            (
                generates && (self.alphabet.is_none() || self.seeds.is_empty()),
                "this command takes --alphabet and --seeds",
            ),
            (
                generates && !self.files.is_empty(),
                "this command generates its sequences and takes no file",
            ),
            (
                one_sequence && named != 1,
                "this command takes one seed or one file",
            ),
            (
                self.command == Command::Scenario && self.files.len() != 1,
                "scenario takes one sequence file",
            ),
            (
                self.command == Command::Compare && self.files.len() != 2,
                "compare takes two stream files",
            ),
        ]
    }

    /// Faults in how the command is to replay them.
    fn option_faults(&self) -> Vec<(bool, &'static str)> {
        let compares = matches!(
            self.command,
            Command::Run | Command::Replay | Command::Shrink
        );
        let survey_of_files = self.command != Command::Replay || !self.files.is_empty();
        let replays_at_once = self.jobs * self.targets.all().len();
        vec![
            (
                compares && self.repeats < 2,
                "a comparison needs --repeats 2 or more",
            ),
            (
                replays_at_once > MOST_AT_ONCE,
                "--jobs is at most 256, and 128 on two targets: every replay has a display of \
                 its own, and the runner has 256",
            ),
            (
                self.targets.is_differential() && !compares,
                "--target gtk,rune is for run, replay and shrink: the commands that compare",
            ),
            (
                self.parked && survey_of_files,
                "--parked is replay's survey of an area's parked sequences, and takes no file",
            ),
            (
                self.corpus && self.perturbation.is_some(),
                "a perturbed divergence is not the application's and never joins the corpus",
            ),
        ]
    }
}

fn positive(flag: &str, text: &str) -> Result<usize, String> {
    text.parse()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("{flag} takes a positive number, not {text:?}"))
}

fn milliseconds(flag: &str, text: &str) -> Result<Duration, String> {
    text.parse()
        .map(Duration::from_millis)
        .map_err(|_| format!("{flag} takes milliseconds, not {text:?}"))
}

/// `7`, `1..200` with both ends, or `3,9,0x2a`.
pub(crate) fn seeds(text: &str) -> Result<Vec<u64>, String> {
    let one = |text: &str| {
        let parsed = match text.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16),
            None => text.parse(),
        };
        parsed.map_err(|_| format!("a seed is a number, not {text:?}"))
    };
    if let Some((first, last)) = text.split_once("..") {
        let (first, last) = (one(first)?, one(last)?);
        if first > last {
            return Err(format!("the seeds {text} run backwards"));
        }
        return Ok((first..=last).collect());
    }
    text.split(',').map(one).collect()
}
