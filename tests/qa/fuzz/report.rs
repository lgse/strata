// SPDX-License-Identifier: MIT

//! What a fuzz command prints and keeps: a line for each sequence, a summary that says what
//! ran, what agreed and how many rules and masks stood, and the artifacts of a sequence that
//! did not agree.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde_json::json;

use super::{
    ARTIFACTS,
    command::rule_counts,
    compare::Divergence,
    corpus::Parked,
    observe::Outcome,
    options::{Command, Options},
    replay::{Replayed, Stream},
    rules::Ignored,
    sequence::Sequence,
    shrink::{CONFIRMATIONS, Shrunk},
};
use crate::{REPOSITORY, target::Target};

/// One sequence after its replays.
pub(crate) struct Examined {
    pub(crate) area: String,
    pub(crate) name: String,
    pub(crate) steps: usize,
    pub(crate) streams: Vec<Stream>,
    /// Where two replays differ outside the map and the mask.
    pub(crate) divergence: Option<Divergence>,
    /// Where they differ with nothing left out: what calibration counts.
    pub(crate) unmasked: Option<Divergence>,
    /// Why the sequence could not be replayed, which is the harness's fault.
    pub(crate) error: Option<String>,
    pub(crate) kept: Option<PathBuf>,
}

impl Examined {
    pub(crate) fn new(area: &str, sequence: &Sequence) -> Self {
        Self {
            area: area.to_owned(),
            name: sequence.name.clone(),
            steps: sequence.steps.len(),
            streams: Vec::new(),
            divergence: None,
            unmasked: None,
            error: None,
            kept: None,
        }
    }

    fn count(&self, outcome: Outcome) -> usize {
        self.streams
            .iter()
            .map(|stream| stream.count(outcome))
            .sum()
    }

    fn unsettled(&self) -> usize {
        self.streams.iter().map(Stream::unsettled).sum()
    }

    fn criticals(&self) -> usize {
        self.streams.iter().map(Stream::criticals).sum()
    }

    /// Why a target went away in some replay.
    fn gone(&self) -> Option<&str> {
        self.streams
            .iter()
            .find_map(|stream| stream.gone.as_deref())
    }

    /// A GLib critical or a lost target is a finding whether or not the replays agree on it.
    fn finding(&self) -> Option<String> {
        match (self.gone(), self.criticals()) {
            (Some(gone), _) => Some(format!("the target was lost: {}", first_line(gone))),
            (None, 0) => None,
            (None, criticals) => Some(format!("{criticals} GLib warnings or worse")),
        }
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

pub(crate) fn differ_line(name: &str, divergence: &Divergence) -> String {
    let first = divergence.fields.first();
    format!(
        "DIFFER {name} at step {} {}: {} ({} | {})",
        divergence.step,
        divergence.action,
        divergence.field(),
        first.map_or(json!(null), |difference| difference.left.clone()),
        first.map_or(json!(null), |difference| difference.right.clone()),
    )
}

fn load_average() -> String {
    fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "?".to_owned())
}

fn label(target: Target) -> &'static str {
    match target {
        Target::Gtk => "gtk",
        Target::Rune => "rune",
    }
}

/// The counts of one command over all its sequences.
pub(crate) struct Run {
    /// Which sequences: the alphabet and its versions, or the corpus.
    pub(crate) what: String,
    command: &'static str,
    header: Vec<String>,
    perturbed: bool,
    load: String,
    sequences: usize,
    agree: usize,
    masked: usize,
    differ: usize,
    errors: usize,
    parked: usize,
    findings: usize,
    /// Sequences that differed, could not be replayed or showed a finding.
    failed: usize,
    replays: usize,
    observed: usize,
    compared: usize,
    counts: BTreeMap<&'static str, usize>,
    fields: BTreeMap<String, usize>,
    areas: BTreeSet<String>,
}

impl Run {
    pub(crate) fn new(options: &Options, target: Target, ignored: &Ignored) -> Self {
        let rest = options.rest;
        let mut header = vec![
            format!(
                "FUZZ replays: target {}, {} of each sequence, {} sequences at once",
                label(target),
                options.repeats,
                options.jobs
            ),
            format!(
                "FUZZ rest: {} equal samples {} ms apart, budget {} ms",
                rest.samples,
                rest.poll.as_millis(),
                rest.budget.as_millis()
            ),
            rule_counts(ignored),
        ];
        if options.perturbation.is_some() {
            header.push(
                "FUZZ PERTURBED: the last replay of each sequence is changed on purpose; a \
                 difference here is the harness's, not the application's"
                    .to_owned(),
            );
        }
        Self {
            what: String::new(),
            command: match options.command {
                Command::Run => "run",
                Command::Replay => "replay",
                Command::Shrink => "shrink",
                _ => "",
            },
            header,
            perturbed: options.perturbation.is_some(),
            load: load_average(),
            sequences: 0,
            agree: 0,
            masked: 0,
            differ: 0,
            errors: 0,
            parked: 0,
            findings: 0,
            failed: 0,
            replays: 0,
            observed: 0,
            compared: 0,
            counts: BTreeMap::new(),
            fields: BTreeMap::new(),
            areas: BTreeSet::new(),
        }
    }

    pub(crate) fn parked(&mut self, name: &str, parked: &Parked) {
        self.parked += 1;
        println!(
            "PARKED {name}: {} {}, not replayed",
            parked.code, parked.issue
        );
    }

    pub(crate) fn start(&self, sequences: usize) {
        println!(
            "FUZZ {}: {}; {sequences} sequences",
            self.command, self.what
        );
        for line in &self.header {
            println!("{line}");
        }
    }

    pub(crate) fn add(&mut self, examined: &Examined) {
        self.count(examined);
        let kept = examined
            .kept
            .as_ref()
            .map(|kept| format!("; kept {}", kept.display()))
            .unwrap_or_default();
        let finding = examined.finding();
        if let Some(finding) = &finding {
            self.findings += 1;
            println!("FINDING {}: {finding}{kept}", examined.name);
        }
        if finding.is_some() || examined.error.is_some() || examined.divergence.is_some() {
            self.failed += 1;
        }
        match (&examined.error, &examined.divergence, &examined.unmasked) {
            (Some(error), _, _) => {
                self.errors += 1;
                println!("ERROR {}: could not replay: {error}", examined.name);
            }
            (None, Some(divergence), _) => {
                self.differ += 1;
                println!("{}{kept}", differ_line(&examined.name, divergence));
            }
            (None, None, Some(unmasked)) => {
                self.masked += 1;
                println!(
                    "MASKED {} at step {} {}: {}{kept}",
                    examined.name,
                    unmasked.step,
                    unmasked.action,
                    unmasked.signature()
                );
            }
            (None, None, None) => {
                self.agree += 1;
                println!(
                    "AGREE {}: {} steps, {} replays",
                    examined.name,
                    examined.steps,
                    examined.streams.len()
                );
            }
        }
    }

    fn count(&mut self, examined: &Examined) {
        self.sequences += 1;
        self.areas.insert(examined.area.clone());
        self.replays += examined.streams.len();
        let lengths = || {
            examined
                .streams
                .iter()
                .map(|stream| stream.observations.len())
        };
        self.observed += lengths().sum::<usize>();
        if let Some(shortest) = lengths().min() {
            self.compared += shortest * (examined.streams.len() - 1);
        }
        for outcome in [
            Outcome::Refused,
            Outcome::Skipped,
            Outcome::Failed,
            Outcome::Left,
            Outcome::Closed,
        ] {
            *self.counts.entry(outcome.name()).or_default() += examined.count(outcome);
        }
        *self.counts.entry("unsettled").or_default() += examined.unsettled();
        *self.counts.entry("criticals").or_default() += examined.criticals();
        let fields: BTreeSet<String> = examined
            .unmasked
            .iter()
            .flat_map(|unmasked| &unmasked.fields)
            .map(|difference| super::project::pattern_of(&difference.field))
            .collect();
        for field in fields {
            *self.fields.entry(field).or_default() += 1;
        }
    }

    /// Prints the summary. An error when a sequence differed, could not be replayed or
    /// showed a finding: the command then fails.
    pub(crate) fn finish(self) -> Result<(), String> {
        let count = |name: &str| self.counts.get(name).copied().unwrap_or_default();
        println!(
            "FUZZ result: {} sequences: {} agree, {} agree under a mask, {} differ, {} could \
             not be replayed; {} parked; {} with a finding",
            self.sequences,
            self.agree,
            self.masked,
            self.differ,
            self.errors,
            self.parked,
            self.findings
        );
        println!(
            "FUZZ steps: {} observations in {} replays, {} compared; {} refused, {} skipped, \
             {} failed, {} unsettled, {} GLib warnings or worse",
            self.observed,
            self.replays,
            self.compared,
            count(Outcome::Refused.name()),
            count(Outcome::Skipped.name()),
            count(Outcome::Failed.name()),
            count("unsettled"),
            count("criticals"),
        );
        println!(
            "FUZZ replays that ended early: {} left the fixture, {} closed the window",
            count(Outcome::Left.name()),
            count(Outcome::Closed.name()),
        );
        let fields: Vec<String> = self
            .fields
            .iter()
            .map(|(field, count)| format!("{field} {count}"))
            .collect();
        println!(
            "FUZZ fields that differed first, with nothing masked, by sequence: {}",
            if fields.is_empty() {
                "none".to_owned()
            } else {
                fields.join(", ")
            }
        );
        println!(
            "FUZZ load average: {} at the start, {} at the end",
            self.load,
            load_average()
        );
        for line in &self.header[2..] {
            println!("{line}");
        }
        for area in &self.areas {
            println!("FUZZ artifacts: {ARTIFACTS}/{area}/");
        }
        if self.failed == 0 {
            return Ok(());
        }
        Err(format!(
            "{} of {} sequences did not agree{}",
            self.failed,
            self.sequences,
            if self.perturbed {
                " (perturbed on purpose)"
            } else {
                ""
            }
        ))
    }
}

fn artifacts(area: &str, name: &str) -> PathBuf {
    Path::new(REPOSITORY).join(ARTIFACTS).join(area).join(name)
}

fn relative(path: &Path) -> PathBuf {
    path.strip_prefix(REPOSITORY).unwrap_or(path).to_owned()
}

fn write_replays(directory: &Path, replays: &[Replayed], target: Target, perturbed: bool) {
    for (index, replayed) in replays.iter().enumerate() {
        let changed = if perturbed && index + 1 == replays.len() {
            "-perturbed"
        } else {
            ""
        };
        let stem = format!("{}-{}{changed}", label(target), index + 1);
        for (suffix, text) in [
            ("stream.jsonl", replayed.stream.text()),
            ("transcript.jsonl", replayed.transcript.clone()),
            ("strata.log", replayed.log.clone()),
            ("input.log", replayed.input.clone()),
        ] {
            let _ = fs::write(directory.join(format!("{stem}.{suffix}")), text);
        }
    }
}

/// What a sequence's replays left to keep.
pub(crate) struct Evidence<'a> {
    pub(crate) sequence: &'a Sequence,
    pub(crate) replays: &'a [Replayed],
    pub(crate) target: Target,
}

impl Evidence<'_> {
    fn has_finding(&self) -> bool {
        self.replays
            .iter()
            .any(|replayed| replayed.stream.gone.is_some() || replayed.stream.criticals() > 0)
    }
}

/// Keeps a sequence's file, streams, transcripts and logs when its replays did not agree,
/// showed a finding, or `--keep` asks for all. Returns where, under the checkout.
pub(crate) fn keep(examined: &Examined, evidence: &Evidence, options: &Options) -> Option<PathBuf> {
    let directory = artifacts(&examined.area, &examined.name);
    let _ = fs::remove_dir_all(&directory);
    let differs = examined.unmasked.is_some() || examined.error.is_some();
    if !(options.keep || differs || evidence.has_finding()) {
        return None;
    }
    fs::create_dir_all(&directory).ok()?;
    let perturbed = options.perturbation.is_some();
    let _ = fs::write(directory.join("sequence.jsonl"), evidence.sequence.text());
    write_replays(&directory, evidence.replays, evidence.target, perturbed);
    let report = json!({
        "sequence": examined.name,
        "error": examined.error,
        "divergence": examined.divergence.as_ref().map(Divergence::report),
        "unmasked": examined.unmasked.as_ref().map(Divergence::report),
        "title": examined.divergence.as_ref().map(|found| found.title(&examined.area)),
        "perturbed": perturbed,
    });
    let _ = fs::write(
        directory.join("divergence.json"),
        crate::run::pretty(&report),
    );
    Some(relative(&directory))
}

/// Prints a shrink's result and keeps the shortest sequence beside the two replays that
/// last showed its divergence.
pub(crate) fn shrunk(area: &str, shrunk: &Shrunk, shortest: &Sequence, evidence: &Evidence) {
    let original = evidence.sequence;
    println!(
        "SHRUNK {} from {} steps to {} in {} replays of two; confirmed {} of {CONFIRMATIONS}",
        original.name,
        original.steps.len(),
        shrunk.steps.len(),
        shrunk.asked,
        shrunk.confirmed
    );
    for step in &shrunk.steps {
        println!("  {step}");
    }
    println!("{}", differ_line(&shortest.name, &shrunk.divergence));
    println!("FUZZ title: {}", shrunk.divergence.title(area));
    let directory = artifacts(area, &original.name);
    let _ = fs::remove_dir_all(&directory);
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let _ = fs::write(directory.join("sequence.jsonl"), original.text());
    let _ = fs::write(directory.join("shrunk.jsonl"), shortest.text());
    let report = json!({
        "sequence": original.name,
        "divergence": shrunk.divergence.report(),
        "title": shrunk.divergence.title(area),
        "steps": { "before": original.steps.len(), "after": shrunk.steps.len() },
        "asked": shrunk.asked,
        "confirmed": shrunk.confirmed,
    });
    let _ = fs::write(
        directory.join("divergence.json"),
        crate::run::pretty(&report),
    );
    write_replays(&directory, evidence.replays, evidence.target, false);
    println!("FUZZ artifacts: {}", relative(&directory).display());
}
