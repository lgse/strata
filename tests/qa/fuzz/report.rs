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
    project::pattern_of,
    replay::{Replayed, Stream},
    rules::Ignored,
    sequence::Sequence,
    shrink::{CONFIRMATIONS, Shrunk},
};
use crate::{REPOSITORY, target::Target};

/// What every line and file of a run that perturbs carries, so that a difference the
/// harness put in is never read as the application's.
pub(crate) const PERTURBED: &str = "PERTURBED";

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
    /// The parked line of a sequence that `replay --parked` replays on purpose.
    pub(crate) parked: Option<Parked>,
    pub(crate) kept: Option<PathBuf>,
}

/// How a sequence came out, the worst first.
enum Verdict<'a> {
    Error(&'a str),
    Differ(&'a Divergence),
    /// Some step did not come to rest, so its state was not compared.
    Unsettled(usize),
    /// The replays differ on masked fields only.
    Masked(&'a Divergence),
    Agree,
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
            parked: None,
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

    fn verdict(&self) -> Verdict<'_> {
        match (&self.error, &self.divergence, &self.unmasked) {
            (Some(error), _, _) => Verdict::Error(error),
            (None, Some(divergence), _) => Verdict::Differ(divergence),
            (None, None, _) if self.unsettled() > 0 => Verdict::Unsettled(self.unsettled()),
            (None, None, Some(unmasked)) => Verdict::Masked(unmasked),
            (None, None, None) => Verdict::Agree,
        }
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// `DIFFER <name> at step <n> <action>: <field> (<left> | <right>)`, under `PERTURBED` when
/// the harness put the difference in.
pub(crate) fn differ_line(name: &str, divergence: &Divergence, perturbed: bool) -> String {
    let first = divergence.fields.first();
    format!(
        "{}DIFFER {name} at step {} {}: {} ({} | {})",
        mark(perturbed),
        divergence.step,
        divergence.action,
        divergence.field(),
        first.map_or(json!(null), |difference| difference.left.clone()),
        first.map_or(json!(null), |difference| difference.right.clone()),
    )
}

fn mark(perturbed: bool) -> String {
    if perturbed {
        format!("{PERTURBED} ")
    } else {
        String::new()
    }
}

fn load_average() -> String {
    fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "?".to_owned())
}

/// The counts of one command over all its sequences.
pub(crate) struct Run {
    /// Which sequences: the alphabet and its versions, or the corpus.
    pub(crate) what: String,
    command: &'static str,
    header: Vec<String>,
    perturbed: bool,
    /// `replay --parked`: a survey of parked sequences, which fails only when it cannot run.
    survey: bool,
    load: String,
    sequences: usize,
    tally: Tally,
    replays: usize,
    observed: usize,
    compared: usize,
    counts: BTreeMap<&'static str, usize>,
    fields: BTreeMap<String, usize>,
    areas: BTreeSet<String>,
    /// Observations that changed after their step's reply, and how long each such change
    /// was in coming after the read before it.
    late: usize,
    quiet: Vec<u64>,
}

/// How many sequences came out each way.
#[derive(Default)]
struct Tally {
    agree: usize,
    masked: usize,
    differ: usize,
    unsettled: usize,
    errors: usize,
    parked: usize,
    findings: usize,
    /// Sequences that differed, did not come to rest, could not be replayed or showed a
    /// finding.
    failed: usize,
}

impl Run {
    pub(crate) fn new(options: &Options, target: Target, ignored: &Ignored) -> Self {
        let rest = options.rest;
        let mut header = vec![
            format!(
                "FUZZ replays: target {}, {} of each sequence, {} sequences at once",
                target.name(),
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
            header.push(format!(
                "FUZZ {PERTURBED}: the last replay of each sequence is changed on purpose; a \
                 difference here is the harness's, not the application's"
            ));
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
            survey: options.parked,
            load: load_average(),
            sequences: 0,
            tally: Tally::default(),
            replays: 0,
            observed: 0,
            compared: 0,
            counts: BTreeMap::new(),
            fields: BTreeMap::new(),
            areas: BTreeSet::new(),
            late: 0,
            quiet: Vec::new(),
        }
    }

    /// Notes a sequence that is parked and so not replayed.
    pub(crate) fn parked(&mut self, name: &str, parked: &Parked) {
        self.tally.parked += 1;
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
        for line in self.judge(examined) {
            println!("{line}");
        }
    }

    /// Counts a sequence and says how it came out, in the lines the command prints.
    pub(crate) fn judge(&mut self, examined: &Examined) -> Vec<String> {
        self.count(examined);
        if let Some(parked) = &examined.parked {
            return vec![self.survey_line(examined, parked)];
        }
        let kept = examined
            .kept
            .as_ref()
            .map(|kept| format!("; kept {}", kept.display()))
            .unwrap_or_default();
        let mut lines = Vec::new();
        let finding = examined.finding();
        if let Some(finding) = &finding {
            self.tally.findings += 1;
            lines.push(format!("FINDING {}: {finding}{kept}", examined.name));
        }
        let (failed, line) = self.verdict_line(examined, &kept);
        if failed || finding.is_some() {
            self.tally.failed += 1;
        }
        lines.push(line);
        lines
    }

    /// The line for a sequence's verdict, and whether the verdict fails the command.
    fn verdict_line(&mut self, examined: &Examined, kept: &str) -> (bool, String) {
        let name = &examined.name;
        match examined.verdict() {
            Verdict::Error(error) => {
                self.tally.errors += 1;
                (true, format!("ERROR {name}: could not replay: {error}"))
            }
            Verdict::Differ(divergence) => {
                self.tally.differ += 1;
                let line = differ_line(name, divergence, self.perturbed);
                (true, format!("{line}{kept}"))
            }
            Verdict::Unsettled(observations) => {
                self.tally.unsettled += 1;
                let line = format!(
                    "UNSETTLED {name}: {observations} observations did not come to rest within \
                     the budget, so their state was not compared{kept}"
                );
                (true, line)
            }
            Verdict::Masked(unmasked) => {
                self.tally.masked += 1;
                let line = format!(
                    "MASKED {name} at step {} {}: {}{kept}",
                    unmasked.step,
                    unmasked.action,
                    unmasked.signature()
                );
                (false, line)
            }
            Verdict::Agree => {
                self.tally.agree += 1;
                let line = format!(
                    "AGREE {name}: {} steps, {} replays",
                    examined.steps,
                    examined.streams.len()
                );
                (false, line)
            }
        }
    }

    /// A parked sequence replayed on purpose: whether what it was parked for still shows.
    /// One that agrees is not reported as fixed, since a race agrees most of the time; the
    /// line says how many replays agreed, and the issue is where to say more.
    fn survey_line(&mut self, examined: &Examined, parked: &Parked) -> String {
        let issue = format!("{} {}", parked.code, parked.issue);
        match examined.verdict() {
            Verdict::Error(error) => {
                self.tally.errors += 1;
                self.tally.failed += 1;
                format!("ERROR {}: could not replay: {error}", examined.name)
            }
            Verdict::Differ(divergence) => {
                self.tally.differ += 1;
                format!(
                    "STILL DIFFERS, PARKED ({issue}): {}",
                    differ_line(&examined.name, divergence, self.perturbed)
                )
            }
            Verdict::Unsettled(_) | Verdict::Masked(_) | Verdict::Agree => {
                self.tally.agree += 1;
                format!(
                    "AGREES IN {} REPLAYS, STILL PARKED ({issue}): {}",
                    examined.streams.len(),
                    examined.name
                )
            }
        }
    }

    /// Adds a sequence's replays, steps and first differing fields to the totals.
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
            .map(|difference| pattern_of(&difference.field))
            .collect();
        for field in fields {
            *self.fields.entry(field).or_default() += 1;
        }
        self.count_late_changes(examined);
    }

    fn count_late_changes(&mut self, examined: &Examined) {
        let observations = examined
            .streams
            .iter()
            .flat_map(|stream| &stream.observations);
        for observation in observations {
            self.late += usize::from(!observation.changes.is_empty());
            let mut before = 0;
            for change in &observation.changes {
                self.quiet.push(change.saturating_sub(before));
                before = *change;
            }
        }
    }

    /// The summary: what ran, what agreed, the steps, what calibration reads, and the rule
    /// and mask counts once more.
    pub(crate) fn summary(&self) -> Vec<String> {
        let count = |name: &str| self.counts.get(name).copied().unwrap_or_default();
        let tally = &self.tally;
        let mut lines = vec![
            format!(
                "FUZZ result: {} sequences: {} agree, {} agree under a mask, {} differ, {} \
                 unsettled, {} could not be replayed; {} parked; {} with a finding",
                self.sequences,
                tally.agree,
                tally.masked,
                tally.differ,
                tally.unsettled,
                tally.errors,
                tally.parked,
                tally.findings
            ),
            format!(
                "FUZZ steps: {} observations in {} replays, {} compared; {} refused, {} \
                 skipped, {} failed, {} unsettled, {} GLib warnings or worse",
                self.observed,
                self.replays,
                self.compared,
                count(Outcome::Refused.name()),
                count(Outcome::Skipped.name()),
                count(Outcome::Failed.name()),
                count("unsettled"),
                count("criticals"),
            ),
            format!(
                "FUZZ replays that ended early: {} left the fixture, {} closed the window",
                count(Outcome::Left.name()),
                count(Outcome::Closed.name()),
            ),
            self.late_changes(),
            self.first_fields(),
            format!(
                "FUZZ load average: {} at the start, {} at the end",
                self.load,
                load_average()
            ),
        ];
        lines.extend(self.header[2..].iter().cloned());
        lines.extend(
            self.areas
                .iter()
                .map(|area| format!("FUZZ artifacts: {ARTIFACTS}/{area}/")),
        );
        lines
    }

    fn first_fields(&self) -> String {
        let fields: Vec<String> = self
            .fields
            .iter()
            .map(|(field, count)| format!("{field} {count}"))
            .collect();
        format!(
            "FUZZ fields that differed first, with nothing masked, by sequence: {}",
            if fields.is_empty() {
                "none".to_owned()
            } else {
                fields.join(", ")
            }
        )
    }

    /// How long the target went on changing after its replies, which is what the quiet
    /// window has to outlast. A change is seen only while the rest wait still runs, so a
    /// longer window (`--rest-samples`) sees further.
    fn late_changes(&self) -> String {
        let mut quiet = self.quiet.clone();
        quiet.sort_unstable();
        let Some(longest) = quiet.last() else {
            return format!("FUZZ late changes: none in {} observations", self.observed);
        };
        let at = |share: usize| quiet[(quiet.len() * share / 100).min(quiet.len() - 1)];
        format!(
            "FUZZ late changes: {} of {} observations changed after their step's reply; \
             quiet before a late change: median {} ms, 99th percentile {} ms, longest \
             {longest} ms",
            self.late,
            self.observed,
            at(50),
            at(99),
        )
    }

    /// An error when no sequence was replayed, or when one differed, did not come to rest,
    /// could not be replayed or showed a finding: the command then fails. A survey of
    /// parked sequences fails only when it could not run.
    pub(crate) fn status(&self) -> Result<(), String> {
        if self.sequences == 0 {
            return Err("The selection replayed no sequence.".to_owned());
        }
        if self.tally.failed == 0 {
            return Ok(());
        }
        let what = if self.survey {
            "could not be replayed"
        } else {
            "did not agree"
        };
        let on_purpose = if self.perturbed {
            " (perturbed on purpose)"
        } else {
            ""
        };
        Err(format!(
            "{} of {} sequences {what}{on_purpose}",
            self.tally.failed, self.sequences
        ))
    }

    pub(crate) fn finish(self) -> Result<(), String> {
        for line in self.summary() {
            println!("{line}");
        }
        self.status()
    }
}

fn artifacts(area: &str, name: &str) -> PathBuf {
    Path::new(REPOSITORY).join(ARTIFACTS).join(area).join(name)
}

fn relative(path: &Path) -> PathBuf {
    path.strip_prefix(REPOSITORY).unwrap_or(path).to_owned()
}

/// Writes each replay's stream, transcript and logs as `<target>-<n>`, with `-perturbed`
/// on the one the harness changed.
fn write_replays(directory: &Path, replays: &[Replayed]) {
    for (index, replayed) in replays.iter().enumerate() {
        let header = &replayed.stream.header;
        let changed = if header.perturbed { "-perturbed" } else { "" };
        let stem = format!("{}-{}{changed}", header.target, index + 1);
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
}

impl Evidence<'_> {
    /// Whether some replay lost the target, logged a GLib warning or did not come to rest.
    fn has_finding(&self) -> bool {
        self.replays.iter().any(|replayed| {
            let stream = &replayed.stream;
            stream.gone.is_some() || stream.criticals() > 0 || stream.unsettled() > 0
        })
    }

    fn perturbed(&self) -> bool {
        self.replays
            .iter()
            .any(|replayed| replayed.stream.header.perturbed)
    }
}

/// Keeps a sequence's file, streams, transcripts and logs when its replays did not agree,
/// showed a finding, or `--keep` asks for all. Returns where, under the checkout.
pub(crate) fn keep(examined: &Examined, evidence: &Evidence, keep_all: bool) -> Option<PathBuf> {
    let directory = artifacts(&examined.area, &examined.name);
    let _ = fs::remove_dir_all(&directory);
    let differs = examined.unmasked.is_some() || examined.error.is_some();
    if !(keep_all || differs || evidence.has_finding()) {
        return None;
    }
    fs::create_dir_all(&directory).ok()?;
    let perturbed = evidence.perturbed();
    let _ = fs::write(directory.join("sequence.jsonl"), evidence.sequence.text());
    write_replays(&directory, evidence.replays);
    let title = examined
        .divergence
        .as_ref()
        .filter(|_| !perturbed)
        .map(|found| found.title(&examined.area));
    let report = json!({
        "sequence": examined.name,
        "error": examined.error,
        "divergence": examined.divergence.as_ref().map(Divergence::report),
        "unmasked": examined.unmasked.as_ref().map(Divergence::report),
        "title": title,
        "perturbed": perturbed,
    });
    let _ = fs::write(
        directory.join("divergence.json"),
        crate::run::pretty(&report),
    );
    Some(relative(&directory))
}

/// Prints a shrink's result and keeps the shortest sequence beside the two replays that
/// last showed its divergence. A perturbed divergence is marked on every line and in every
/// file, and gets no issue title: it is not the application's.
pub(crate) fn shrunk(area: &str, shrunk: &Shrunk, shortest: &Sequence, evidence: &Evidence) {
    let original = evidence.sequence;
    let perturbed = evidence.perturbed();
    println!(
        "{}SHRUNK {} from {} steps to {} in {} replays of two; confirmed {} of {CONFIRMATIONS}",
        mark(perturbed),
        original.name,
        original.steps.len(),
        shrunk.steps.len(),
        shrunk.asked,
        shrunk.confirmed
    );
    for step in &shrunk.steps {
        println!("  {step}");
    }
    println!(
        "{}",
        differ_line(&shortest.name, &shrunk.divergence, perturbed)
    );
    let title = (!perturbed).then(|| shrunk.divergence.title(area));
    if let Some(title) = &title {
        println!("FUZZ title: {title}");
    }
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
        "title": title,
        "perturbed": perturbed,
        "steps": { "before": original.steps.len(), "after": shrunk.steps.len() },
        "asked": shrunk.asked,
        "confirmed": shrunk.confirmed,
    });
    let _ = fs::write(
        directory.join("divergence.json"),
        crate::run::pretty(&report),
    );
    write_replays(&directory, evidence.replays);
    println!("FUZZ artifacts: {}", relative(&directory).display());
}
