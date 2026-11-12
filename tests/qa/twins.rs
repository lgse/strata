// SPDX-License-Identifier: MIT

//! Labelled twins (`docs/testing.md`, "Twins"). A run that cannot pass on the Rune target
//! under a named exception is `parallel` in the ledger, and a twin stands in for it there:
//! `<scenario>-rune.jsonl`, beside the scenario, with `"targets":["rune"]`. Both headers
//! carry a `parity` label that names the other file, the exception and its reference.
//!
//! This is where "parity" could come to mean little, so everything a check can hold is
//! held here, and `scripts/qa_parity_twins.py` holds the same for `./scripts/qa-parity.sh`:
//! `tests/qa/parity_cases.toml` is the table both are tested against.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    REPOSITORY,
    parity::{Entry, Ledger, Planned, Status, gate},
    scenario::{Header, header_line, one_line, scenario_files, scenario_name},
    target::Target,
    variables::{Run, runs},
};

/// What a twin's name adds to its original's.
pub(crate) const SUFFIX: &str = "-rune";
/// The exceptions of the parity plan that may carry a twin.
const TWIN_BEARING: [&str; 6] = ["timing", "a11y", "rune", "oracle", "toolkit", "deviation"];
/// The pending reasons that are no exception: they must reach zero.
const PENDING_ONLY: [&str; 3] = ["scope", "harness", "component"];
/// The Rune filing packet: a `rune` twin cites the entry it expires with.
const PACKET: &str = "docs/rune/upstream/rune";
/// The Rune mutation patches, each named for the scenario that must catch it.
const PATCHES: &str = "tests/qa/parity/mutations";
const ISSUES: &str = "lgse/strata#";

/// A header's `parity` label. `twin` is the other file of the pair: on the original its
/// twin, and on the twin its original.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Label {
    pub(crate) twin: String,
    pub(crate) exception: String,
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    /// On the twin only: which steps differ from the original's, and why.
    pub(crate) differs: Option<String>,
}

/// What is wrong with an exception code, for a header or a ledger entry.
pub(crate) fn exception_problem(code: &str) -> Option<String> {
    if TWIN_BEARING.contains(&code) {
        return None;
    }
    let why = if code == "platform" {
        "never carries a twin: the same file runs on the session host, so its run is pending \
         or required with that host"
    } else if PENDING_ONLY.contains(&code) {
        "is a pending reason and no exception: the run stays pending until its blocker is gone"
    } else {
        "is none: a twin is allowed under timing, a11y, rune, oracle, toolkit or deviation"
    };
    Some(format!("names the exception {code:?}, which {why}"))
}

/// What is wrong with the form of a reference. A `rune` twin cites its packet entry, and
/// every other one an issue.
pub(crate) fn reference_problem(code: &str, reference: &str) -> Option<String> {
    if code == "rune" {
        packet_entry(reference).is_none().then(|| {
            format!(
                "cites {reference:?}, which is no packet entry: a rune twin names the entry \
                 it expires with, {PACKET}/<number>-<name>.md"
            )
        })
    } else {
        (!is_issue(reference)).then(|| {
            format!("cites {reference:?}, which is no reference: it is the issue, {ISSUES}<number>")
        })
    }
}

fn is_issue(reference: &str) -> bool {
    reference.strip_prefix(ISSUES).is_some_and(|number| {
        !number.is_empty()
            && !number.starts_with('0')
            && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// The file a packet reference names: a numbered entry directly in the packet.
fn packet_entry(reference: &str) -> Option<&str> {
    let file = reference.strip_prefix(PACKET)?.strip_prefix('/')?;
    let numbered = file.starts_with(|first: char| first.is_ascii_digit());
    (numbered && file.ends_with(".md") && !file.contains('/')).then_some(file)
}

/// What is wrong with a header's `parity` and `targets`, read alone. A name that ends in
/// `-rune` is a twin's and no other scenario's.
pub(crate) fn label_problem(header: &Header) -> Option<String> {
    let original = header.scenario.strip_suffix(SUFFIX);
    let Some(label) = &header.parity else {
        return unlabelled(header, original);
    };
    exception_problem(&label.exception)
        .or_else(|| reference_problem(&label.exception, &label.reference))
        .or_else(|| match original {
            Some(original) => twin_problem(header, label, original),
            None => original_problem(header, label),
        })
}

fn unlabelled(header: &Header, original: Option<&str>) -> Option<String> {
    if let Some(original) = original {
        Some(format!(
            "is named as a twin and has no \"parity\" label: a twin names its original, \
             {original:?}, with the exception, the reference and what differs"
        ))
    } else if header.targets.is_some() {
        Some(
            "has \"targets\" and is no twin: every scenario runs on both targets, and a run \
             that cannot pass on Rune is pending, parallel with a twin, or excluded by a ruling"
                .to_owned(),
        )
    } else {
        None
    }
}

fn twin_problem(header: &Header, label: &Label, original: &str) -> Option<String> {
    let name = &header.scenario;
    let problem = if original.ends_with(SUFFIX) {
        format!(
            "is a twin of a twin: {original:?} is itself a twin, and a twin stands in for \
             runs of the file both targets share"
        )
    } else if label.twin != original {
        format!(
            "names {:?}, and is the twin of {original:?}: a twin is its original's name with \
             {SUFFIX}, beside it",
            label.twin
        )
    } else if !label.differs.as_deref().is_some_and(one_line) {
        format!(
            "is a twin without a one-line \"differs\": say which steps differ from \
             {original}'s and why"
        )
    } else if header.targets.as_deref() != Some(&[Target::Rune]) {
        "is a twin that also runs on GTK: its header needs \"targets\":[\"rune\"]".to_owned()
    } else if header.mutation.is_some() {
        format!(
            "is a twin with a \"mutation\", which names a GTK patch: a twin's proof that it \
             can fail is {PATCHES}/{name}.patch"
        )
    } else if header.known_bug.is_some() {
        "is a twin marked known_bug: a twin passes on Rune, and a run whose twin cannot is \
         pending"
            .to_owned()
    } else {
        return None;
    };
    Some(problem)
}

fn original_problem(header: &Header, label: &Label) -> Option<String> {
    let name = &header.scenario;
    if label.twin != format!("{name}{SUFFIX}") {
        Some(format!(
            "names {:?} as its twin: the twin of {name} is {name}{SUFFIX}, beside it",
            label.twin
        ))
    } else if label.differs.is_some() {
        Some(format!(
            "has \"differs\", which only the twin carries: {name}{SUFFIX}'s header says \
             which steps differ"
        ))
    } else if header.targets.is_some() {
        Some(
            "has \"targets\", and only a twin does: the original keeps running on both \
             targets, and the \"parity\" label is the one edit made to it"
                .to_owned(),
        )
    } else {
        None
    }
}

/// What a run starts from, by header key: the fixture, the preferences and the layout a
/// twin keeps from its original. The environment, the bus and the toolkit's settings are
/// how a target is started, and are left to the reviewer.
type Start = Vec<(&'static str, Value)>;

fn start(header: &Header, raw: &Value, run: &Run) -> Start {
    let variant = &run.variant;
    let joined = |first: &[Value], second: &[Value]| json!([first, second].concat());
    let mut prefs: Map<String, Value> = header.prefs.clone();
    prefs.extend(variant.prefs.clone());
    let images = raw.get("images").filter(|images| !images.is_null());
    vec![
        (
            "tree",
            json!(variant.tree.as_ref().or(header.tree.as_ref())),
        ),
        ("images", images.cloned().unwrap_or_else(|| json!({}))),
        ("fixture_root", json!(header.fixture_root)),
        ("setup", joined(&header.setup, &variant.setup)),
        ("teardown", joined(&header.teardown, &variant.teardown)),
        ("prefs", Value::Object(prefs)),
        (
            "layout",
            json!(variant.layout.as_ref().unwrap_or(&header.layout)),
        ),
        ("portal", json!(header.portal)),
    ]
}

/// A run of a scenario: its mode, its label and what it starts from.
struct Planning {
    mode: String,
    label: String,
    start: Start,
}

/// What a header says of its scenario, for the pair checks.
struct Read {
    label: Option<Label>,
    /// What is wrong with the label, read alone.
    fault: Option<String>,
    runs: Vec<Planning>,
}

impl Read {
    fn of(text: &str) -> Result<Self, String> {
        let unread = |error: serde_json::Error| format!("line 1, the header: {error}");
        let raw: Value = serde_json::from_str(text).map_err(unread)?;
        let header: Header = serde_json::from_value(raw.clone()).map_err(unread)?;
        let planning = |run: Run| Planning {
            start: start(&header, &raw, &run),
            mode: run.mode,
            label: run.label,
        };
        Ok(Self {
            fault: label_problem(&header),
            runs: runs(&header).into_iter().map(planning).collect(),
            label: header.parity,
        })
    }

    fn run(&self, label: &str) -> Option<&Planning> {
        self.runs.iter().find(|run| run.label == label)
    }
}

/// A scenario file, by its folder and its name.
struct Facts {
    folder: String,
    scenario: String,
    read: Result<Read, String>,
}

impl Facts {
    /// The file under `tests/qa`, as a problem names it.
    fn file(&self) -> String {
        format!("scenarios/{}/{}.jsonl", self.folder, self.scenario)
    }

    /// The header, when nothing is wrong with it alone: a pair is checked only then.
    fn sound(&self) -> Option<&Read> {
        self.read.as_ref().ok().filter(|read| read.fault.is_none())
    }

    /// What is wrong with the label, read alone.
    fn alone(&self) -> Option<String> {
        let fault = self.read.as_ref().ok()?.fault.as_ref()?;
        Some(format!("{} {fault}", self.file()))
    }

    /// Why the header cannot be read at all.
    fn unread(&self) -> Option<String> {
        let error = self.read.as_ref().err()?;
        Some(format!("{}: {error}", self.file()))
    }
}

/// What the ledger and the labels are checked against: every scenario header, the Rune
/// mutation patches and the entries of the Rune filing packet.
pub(crate) struct Corpus {
    scenarios: Vec<Facts>,
    /// The scenarios the patches under `tests/qa/parity/mutations` are named for.
    patches: BTreeSet<String>,
    /// The file names under `docs/rune/upstream/rune`.
    packet: BTreeSet<String>,
}

/// The names in a directory of the repository; none when it is not there.
fn names(directory: &str) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(Path::new(REPOSITORY).join(directory))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| PathBuf::from(entry.file_name()))
}

fn text(name: Option<&std::ffi::OsStr>) -> Option<String> {
    name.and_then(|name| name.to_str()).map(str::to_owned)
}

impl Corpus {
    /// A corpus with no scenario yet, by the patches and the packet entries there are.
    pub(crate) fn citing(patches: &[String], packet: &[String]) -> Self {
        Self {
            scenarios: Vec::new(),
            patches: patches.iter().cloned().collect(),
            packet: packet.iter().cloned().collect(),
        }
    }

    /// Adds a scenario by its header line, at its place in the runner's order.
    pub(crate) fn add(&mut self, folder: &str, scenario: &str, header: &str) {
        let facts = Facts {
            folder: folder.to_owned(),
            scenario: scenario.to_owned(),
            read: Read::of(header),
        };
        let file = facts.file();
        let place = self.scenarios.partition_point(|other| other.file() < file);
        self.scenarios.insert(place, facts);
    }

    pub(crate) fn load() -> Self {
        let patch = |name: PathBuf| {
            (name.extension()? == "patch")
                .then(|| text(name.file_stem()))
                .flatten()
        };
        let patches: Vec<String> = names(PATCHES).filter_map(patch).collect();
        let packet: Vec<String> = names(PACKET)
            .filter_map(|name| text(Some(name.as_os_str())))
            .collect();
        let mut corpus = Self::citing(&patches, &packet);
        for path in scenario_files() {
            let folder = text(path.parent().and_then(Path::file_name)).unwrap_or_default();
            let header = header_line(&path).unwrap_or_default();
            corpus.add(&folder, &scenario_name(&path), &header);
        }
        corpus
    }

    /// Every run the ledger lists, as the runner plans them. A twin's runs are not among
    /// them: each is listed once, as the `twin` of the parallel run it stands in for.
    pub(crate) fn planned(&self) -> Vec<Planned> {
        let shared = |facts: &&Facts| !facts.scenario.ends_with(SUFFIX);
        let mut planned = Vec::new();
        for facts in self.scenarios.iter().filter(shared) {
            for run in facts.read.iter().flat_map(|read| &read.runs) {
                planned.push(Planned {
                    folder: facts.folder.clone(),
                    scenario: facts.scenario.clone(),
                    gate: gate(&facts.folder, &facts.scenario, &run.mode, &run.label),
                    run: run.label.clone(),
                });
            }
        }
        planned
    }

    fn find(&self, scenario: &str) -> Option<&Facts> {
        self.scenarios
            .iter()
            .find(|facts| facts.scenario == scenario)
    }

    /// The headers that cannot be read, one line each. Their runs are unknown, so nothing
    /// is checked against a corpus that has one.
    pub(crate) fn unread(&self) -> Vec<String> {
        self.scenarios.iter().filter_map(Facts::unread).collect()
    }

    /// Everything wrong with the labels and the pairs, one line each: first each header
    /// alone, then each pair against its two headers, the ledger, the patches and the packet.
    pub(crate) fn problems(&self, ledger: &Ledger) -> Vec<String> {
        let mut problems: Vec<String> = self.scenarios.iter().filter_map(Facts::alone).collect();
        for facts in &self.scenarios {
            problems.extend(self.pair_problems(facts, ledger));
        }
        problems
    }

    fn pair_problems(&self, facts: &Facts, ledger: &Ledger) -> Vec<String> {
        let Some(read) = facts.sound() else {
            return Vec::new();
        };
        if let Some(original) = facts.scenario.strip_suffix(SUFFIX) {
            return self.orphaned(facts, original).into_iter().collect();
        }
        let entries = ledger.entries_of(&facts.folder, &facts.scenario);
        let twin = self.find(&format!("{}{SUFFIX}", facts.scenario));
        match twin {
            None => unpaired(facts, read, &entries),
            // A twin in another folder is reported from its own side.
            Some(twin) if twin.folder != facts.folder => Vec::new(),
            Some(twin) => twin.sound().map_or_else(Vec::new, |stand_in| {
                let pair = Pair {
                    original: facts,
                    twin,
                    shared: read,
                    stand_in,
                };
                self.paired(&pair, &entries)
            }),
        }
    }

    /// What is wrong with a twin whose original is not beside it.
    fn orphaned(&self, twin: &Facts, original: &str) -> Option<String> {
        let file = twin.file();
        match self.find(original) {
            None => Some(format!(
                "{file} is the twin of {original}, and no scenario has that name: a twin \
                 stands beside its original"
            )),
            Some(found) if found.folder != twin.folder => Some(format!(
                "{file} is the twin of {original}, which is in {}: a twin is a file beside \
                 its original",
                found.folder
            )),
            Some(_) => None,
        }
    }

    fn paired(&self, pair: &Pair, entries: &[&Entry]) -> Vec<String> {
        let (file, twin) = (pair.original.file(), &pair.twin.scenario);
        let (Some(label), Some(twin_label)) = (&pair.shared.label, &pair.stand_in.label) else {
            return vec![format!(
                "{file} has the twin {twin} and no \"parity\" label: give it the label, with \
                 the twin's exception and reference; its steps stay as they are"
            )];
        };
        let file = pair.twin.file();
        if pair.stand_in.runs.is_empty() {
            return vec![format!(
                "{file} has no run: a twin holds the modes or variants of {} it stands in for",
                pair.original.scenario
            )];
        }
        let mut problems = pair.disagreements(label, twin_label);
        problems.extend(self.unfounded(pair, twin_label));
        problems.extend(pair.strays());
        let mut problems: Vec<String> = problems
            .into_iter()
            .map(|problem| format!("{file} {problem}"))
            .collect();
        let folder = &pair.original.folder;
        problems.extend(entries.iter().filter_map(|entry| {
            let problem = pair.standing(entry, label)?;
            Some(format!(
                "{folder}.toml: {}/{} {problem}",
                entry.scenario, entry.run
            ))
        }));
        problems
    }

    /// What a twin lacks outside its header: the packet entry a `rune` twin expires with,
    /// and the patch that proves the twin can fail.
    fn unfounded(&self, pair: &Pair, label: &Label) -> Vec<String> {
        let twin = &pair.twin.scenario;
        let unfiled = label.exception == "rune"
            && packet_entry(&label.reference).is_some_and(|entry| !self.packet.contains(entry));
        let unfiled = unfiled.then(|| {
            format!(
                "cites {:?}, and {PACKET} has no such entry: a rune twin names the packet \
                 entry it expires with",
                label.reference
            )
        });
        let unproven = (!self.patches.contains(twin)).then(|| {
            format!(
                "has no proof that it can fail: add {PATCHES}/{twin}.patch, which breaks the \
                 behaviour in crates/strata-app, and check it with ./scripts/qa-parity.sh \
                 mutations"
            )
        });
        unfiled.into_iter().chain(unproven).collect()
    }
}

/// What is wrong with a scenario that has no twin beside it: a parallel run, or a label.
fn unpaired(facts: &Facts, read: &Read, entries: &[&Entry]) -> Vec<String> {
    let (folder, scenario) = (&facts.folder, &facts.scenario);
    let missing = format!("tests/qa/scenarios/{folder}/{scenario}{SUFFIX}.jsonl");
    let parallel: Vec<String> = entries
        .iter()
        .filter(|entry| entry.status == Status::Parallel)
        .map(|entry| {
            format!(
                "{folder}.toml: {scenario}/{} is parallel, and its twin's file does not \
                 exist: {missing}. Write the twin, or the run is pending",
                entry.run
            )
        })
        .collect();
    if !parallel.is_empty() || read.label.is_none() {
        return parallel;
    }
    vec![format!(
        "{} names the twin {scenario}{SUFFIX}, and {missing} does not exist: write the twin, \
         or take the \"parity\" label off",
        facts.file()
    )]
}

/// A scenario and the twin beside it, each with a header that is sound alone.
struct Pair<'a> {
    original: &'a Facts,
    twin: &'a Facts,
    /// What the original's header says, and what the twin's does.
    shared: &'a Read,
    stand_in: &'a Read,
}

impl Pair<'_> {
    /// Where the twin's label parts from its original's.
    fn disagreements(&self, label: &Label, twin: &Label) -> Vec<String> {
        let code = (label.exception != twin.exception).then(|| {
            format!(
                "is under the exception {:?} and its original under {:?}: the two headers \
                 carry one code",
                twin.exception, label.exception
            )
        });
        let reference = (label.reference != twin.reference).then(|| {
            format!(
                "cites {:?} and its original {:?}: the two headers carry one reference",
                twin.reference, label.reference
            )
        });
        code.into_iter().chain(reference).collect()
    }

    /// What is wrong with the twin's runs: one its original lacks, or one that starts from
    /// something else.
    fn strays(&self) -> Vec<String> {
        let original = &self.original.scenario;
        let stray = |run: &Planning| {
            let label = &run.label;
            let Some(shared) = self.shared.run(label) else {
                return Some(format!(
                    "has the run {label}, which {original} lacks: a twin holds only modes and \
                     variants of its original"
                ));
            };
            let differs = shared.start.iter().zip(&run.start);
            let (key, _) = differs
                .map(|(shared, own)| (shared.0, shared.1 == own.1))
                .find(|(_, same)| !same)?;
            Some(format!(
                "starts {label} with another {key:?} than {original}: a twin keeps what its \
                 original starts from, and only the steps \"differs\" names differ"
            ))
        };
        self.stand_in.runs.iter().filter_map(stray).collect()
    }

    /// What is wrong with how an entry of the original stands against the twin's runs.
    fn standing(&self, entry: &Entry, label: &Label) -> Option<String> {
        let (twin, run) = (&self.twin.scenario, &entry.run);
        let stands_in = self.stand_in.run(run).is_some();
        match (entry.status == Status::Parallel, stands_in) {
            (false, true) => Some(format!(
                "is {}, and {twin}/{run} stands in for it: a run with a twin is parallel, \
                 with the exception, the twin and the reference",
                entry.status.name()
            )),
            (true, false) => Some(format!(
                "is parallel, and {twin} has no run {run}: the twin holds every run it \
                 stands in for"
            )),
            (false, false) => None,
            (true, true) => disagreement(entry, label),
        }
    }
}

/// Where a parallel entry parts from its scenario's label. An entry without the field, or
/// with one of the wrong form, is the ledger's own problem and is said once, there.
fn disagreement(entry: &Entry, label: &Label) -> Option<String> {
    let code = entry.exception.as_deref()?;
    if exception_problem(code).is_some() {
        return None;
    }
    let formed = |reference: &&str| reference_problem(code, reference).is_none();
    let reference = entry.reference.as_deref().filter(formed);
    if code != label.exception {
        Some(format!(
            "is parallel under {code:?}, and its header says {:?}: the ledger and the two \
             headers carry one code",
            label.exception
        ))
    } else {
        let reference = reference.filter(|reference| *reference != label.reference)?;
        Some(format!(
            "cites {reference:?}, and its header {:?}: the ledger and the two headers carry \
             one reference",
            label.reference
        ))
    }
}
