// SPDX-License-Identifier: MIT

//! A fuzz sequence: a scenario file whose steps the fuzz replay can run and observe.
//!
//! The file is the ordinary format and goes through the ordinary loader. The replay runs
//! fewer kinds of step than `drive` does, so a sequence is held to those: a process step
//! that acts or waits, or an `fs` action, each optionally under a `when`.

use std::{fs, path::Path};

use serde_json::Value;

use crate::{
    DEFAULT_LAYOUT, STEP_IF, STEP_MODES, STEP_UNLESS, STEP_VARIANTS, STEP_WHEN,
    fs_check::FsStep,
    protocol,
    scenario::{Scenario, load_scenario},
    steps::run_steps,
    variables::{Run, placeholders, runs, variables},
};

/// Every sequence is named `fuzz-<area>-<seed>`, so its artifacts never meet a scenario's.
pub(crate) const PREFIX: &str = "fuzz";
/// Steps that ask the application something and neither change it nor wait for it: a
/// sequence has no use for them, since every step is followed by an observation. `wait`
/// and `settle` stay: they are how an action waits on what the application shows, where an
/// area knows what that is.
const ASKING: [&str; 5] = ["holds", "query", "expect", "during", "note"];
/// Keys that tie a step to another step or to a run, which a shrunk sequence cannot keep.
const TYING: [&str; 5] = [STEP_IF, STEP_UNLESS, STEP_MODES, STEP_VARIANTS, "as"];

/// A sequence as text: the header line and one line for each step, exactly as an alphabet
/// spells them, so the same seed writes the same bytes whatever serialises JSON here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sequence {
    pub(crate) name: String,
    pub(crate) header: String,
    pub(crate) steps: Vec<String>,
}

/// A sequence the loader accepted, ready to replay.
pub(crate) struct Loaded {
    pub(crate) scenario: Scenario,
    pub(crate) run: Run,
}

impl Sequence {
    pub(crate) fn text(&self) -> String {
        let mut text = self.header.clone();
        text.push('\n');
        for step in &self.steps {
            text.push_str(step);
            text.push('\n');
        }
        text
    }

    /// Reads a sequence from a scenario file's text. The name is the header's.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let header = lines.next().ok_or("the sequence is empty")?.to_owned();
        let parsed: Value =
            serde_json::from_str(&header).map_err(|error| format!("the header: {error}"))?;
        let name = parsed["scenario"]
            .as_str()
            .ok_or("the header names no scenario")?
            .to_owned();
        Ok(Self {
            name,
            header,
            steps: lines.map(str::to_owned).collect(),
        })
    }

    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let sequence =
            Self::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        let stem = path.file_stem().and_then(|stem| stem.to_str());
        if stem != Some(sequence.name.as_str()) {
            return Err(format!(
                "{}: the header names {:?}",
                path.display(),
                sequence.name
            ));
        }
        Ok(sequence)
    }

    /// The same sequence with other steps, as the shrinker tries them.
    pub(crate) fn with_steps(&self, steps: Vec<String>) -> Self {
        Self {
            name: self.name.clone(),
            header: self.header.clone(),
            steps,
        }
    }

    /// Loads the sequence as the scenario it is and holds it to what the replay can run.
    /// The loader reads a file named for its scenario, so the text goes through one.
    pub(crate) fn load(&self) -> Result<Loaded, String> {
        if !self.name.starts_with(&format!("{PREFIX}-")) {
            return Err(format!(
                "a sequence is named {PREFIX}-<area>-<seed>, not {:?}",
                self.name
            ));
        }
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let path = directory.path().join(format!("{}.jsonl", self.name));
        fs::write(&path, self.text()).map_err(|error| error.to_string())?;
        let scenario = load_scenario(&path).map_err(|error| format!("{}: {error}", self.name))?;
        check_header(&scenario).map_err(|error| format!("{}: {error}", self.name))?;
        let mut runs = runs(&scenario.header);
        let run = match runs.len() {
            1 => runs.remove(0),
            count => {
                return Err(format!(
                    "{}: a sequence is one run in one mode, not {count}",
                    self.name
                ));
            }
        };
        for (line, step) in &scenario.steps {
            check_step(step).map_err(|error| format!("{}: line {line}: {error}", self.name))?;
        }
        run_steps(&scenario, &run, &variables(&run, &placeholders()))
            .map_err(|error| format!("{}: {error}", self.name))?;
        Ok(Loaded { scenario, run })
    }
}

/// What the replay does not start: a session bus, a portal, an input method, a fixed
/// fixture path, another keyboard layout; and what belongs to a scenario and not to a
/// sequence. A header that asks for one is refused, not replayed without it.
fn check_header(scenario: &Scenario) -> Result<(), String> {
    let header = &scenario.header;
    let unsupported = [
        ("session_bus", header.session_bus),
        ("portal", header.portal),
        ("input_method", header.input_method.is_some()),
        ("fixture_root", header.fixture_root.is_some()),
        ("variants", !header.variants.is_empty()),
        ("layout", header.layout != DEFAULT_LAYOUT),
        ("mutation", header.mutation.is_some()),
        ("known_bug", header.known_bug.is_some()),
        ("skip", header.skip.is_some()),
        ("teardown", !header.teardown.is_empty()),
        ("images", !header.images.is_empty()),
        // A sequence is replayed on every target and stands under no exception of its own.
        ("parity", header.parity.is_some()),
        ("targets", header.targets.is_some()),
    ];
    match unsupported.iter().find(|(_, set)| *set) {
        Some((key, _)) => Err(format!("the fuzz replay does not take the header's {key}")),
        None => Ok(()),
    }
}

/// A step the replay can run: one process command that acts or waits, or one `fs` action.
pub(crate) fn check_step(step: &Value) -> Result<(), String> {
    let object = step.as_object().ok_or("a step must be an object")?;
    if let Some(key) = TYING.iter().find(|key| object.contains_key(**key)) {
        return Err(format!(
            "a fuzz step stands alone, so it takes no {key}; {STEP_WHEN} is its one condition"
        ));
    }
    if object.contains_key("fs") {
        let parsed: FsStep = serde_json::from_value(strip(step, STEP_WHEN))
            .map_err(|error| format!("invalid fs step: {error}"))?;
        return if parsed.fs.acts() {
            Ok(())
        } else {
            Err("a fuzz fs step acts on the fixture; the observation does the checking".to_owned())
        };
    }
    let command = protocol::COMMANDS
        .iter()
        .find(|name| object.contains_key(**name))
        .ok_or("the fuzz replay runs process steps and fs actions only")?;
    if ASKING.contains(command) {
        return Err(format!(
            "{command} only asks; every fuzz step is observed already"
        ));
    }
    Ok(())
}

/// The step without one of its keys.
pub(crate) fn strip(step: &Value, key: &str) -> Value {
    let mut object = step.as_object().cloned().unwrap_or_default();
    object.remove(key);
    Value::Object(object)
}
