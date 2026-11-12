// SPDX-License-Identifier: MIT

//! An area's alphabet, `tests/qa/fuzz/alphabets/<area>.toml`: the actions a seed picks from,
//! and the scenario header every sequence of the area starts with.
//!
//! An action is one or more scenario steps written exactly as a scenario file writes them,
//! so a step can be copied either way. A step names its target by role and name, never by a
//! screen point, and takes a `when` wherever it would otherwise wait for a target that is
//! not there. `<name>` in a step stands for one of the action's `choose` values.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::Value;

use super::{DATA, sequence::Sequence};
use crate::REPOSITORY;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Alphabet {
    /// The file's stem: lowercase words joined by hyphens.
    pub(crate) area: String,
    /// Raised by every change that makes a seed write another sequence.
    pub(crate) version: u32,
    /// One line on what the alphabet reaches and what it leaves out.
    pub(crate) about: String,
    /// The scenario header of every sequence, as JSON, without its `scenario` key.
    pub(crate) header: String,
    #[serde(rename = "action")]
    pub(crate) actions: Vec<Action>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Action {
    pub(crate) name: String,
    /// How often the action is picked, against the sum of all weights.
    pub(crate) weight: u32,
    /// Scenario steps, each a line of JSON.
    pub(crate) steps: Vec<String>,
    /// What each `<name>` in the steps may stand for.
    #[serde(default)]
    pub(crate) choose: BTreeMap<String, Vec<String>>,
}

pub(crate) fn directory() -> PathBuf {
    Path::new(REPOSITORY).join(DATA).join("alphabets")
}

/// The areas that have an alphabet, in name order.
pub(crate) fn areas() -> Vec<String> {
    let mut areas: Vec<String> = fs::read_dir(directory())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .filter_map(|path| Some(path.file_stem()?.to_str()?.to_owned()))
        .collect();
    areas.sort();
    areas
}

pub(crate) fn load(area: &str) -> Result<Alphabet, String> {
    let path = directory().join(format!("{area}.toml"));
    let text = fs::read_to_string(&path).map_err(|_| {
        format!(
            "no alphabet {area:?} in {DATA}/alphabets; there is {}",
            areas().join(", ")
        )
    })?;
    let alphabet = parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    if alphabet.area != area {
        return Err(format!(
            "{}: the alphabet names the area {:?}",
            path.display(),
            alphabet.area
        ));
    }
    Ok(alphabet)
}

pub(crate) fn parse(text: &str) -> Result<Alphabet, String> {
    let alphabet: Alphabet = toml::from_str(text).map_err(|error| error.to_string())?;
    alphabet.check()?;
    Ok(alphabet)
}

fn is_area(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        })
}

impl Alphabet {
    pub(crate) fn weight(&self) -> u64 {
        self.actions
            .iter()
            .map(|action| u64::from(action.weight))
            .sum()
    }

    /// The header line of a sequence called `name`.
    pub(crate) fn header_line(&self, name: &str) -> String {
        let rest = self.header.trim().trim_start_matches('{').trim_start();
        let separator = if rest.starts_with('}') { "" } else { "," };
        format!(r#"{{"scenario":"{name}"{separator}{rest}"#)
    }

    fn check(&self) -> Result<(), String> {
        self.check_head()?;
        if self.actions.is_empty() {
            return Err("an alphabet has at least one action".to_owned());
        }
        let mut names: Vec<&str> = self
            .actions
            .iter()
            .map(|action| action.name.as_str())
            .collect();
        names.sort_unstable();
        if let Some(pair) = names.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(format!("two actions are named {:?}", pair[0]));
        }
        for action in &self.actions {
            action
                .check()
                .map_err(|error| format!("action {:?}: {error}", action.name))?;
        }
        // Every step of every action, with every value it may take, as one sequence: what
        // the loader and the replay would refuse of a generated sequence is refused here.
        let specimen = format!("fuzz-{}-specimen", self.area);
        Sequence {
            header: self.header_line(&specimen),
            name: specimen,
            steps: self.actions.iter().flat_map(Action::every_step).collect(),
        }
        .load()
        .map(|_| ())
    }

    /// The keys that say what the alphabet is, before its actions.
    fn check_head(&self) -> Result<(), String> {
        if !is_area(&self.area) {
            return Err(format!(
                "an area is lowercase words joined by hyphens, not {:?}",
                self.area
            ));
        }
        if self.version == 0 {
            return Err("version starts at 1".to_owned());
        }
        if self.about.trim().is_empty() || self.about.contains('\n') {
            return Err("about is one non-empty line".to_owned());
        }
        let header: Value = serde_json::from_str(&self.header)
            .map_err(|error| format!("header is a JSON object: {error}"))?;
        if header.is_object() && header.get("scenario").is_none() {
            Ok(())
        } else {
            Err("header is a JSON object without a scenario key".to_owned())
        }
    }
}

impl Action {
    fn check(&self) -> Result<(), String> {
        if !is_area(&self.name) {
            return Err("a name is lowercase words joined by hyphens".to_owned());
        }
        if self.weight == 0 {
            return Err("weight is at least 1".to_owned());
        }
        if self.steps.is_empty() {
            return Err("an action has at least one step".to_owned());
        }
        for (name, values) in &self.choose {
            if values.is_empty() {
                return Err(format!("choose.{name} lists nothing to choose from"));
            }
            if !self.steps.iter().any(|step| step.contains(&slot(name))) {
                return Err(format!("no step uses <{name}>"));
            }
        }
        self.steps.iter().try_for_each(|step| self.check_step(step))
    }

    /// A step is one line of JSON once every `<name>` in it has a value.
    fn check_step(&self, step: &str) -> Result<(), String> {
        if step.contains('\n') {
            return Err(format!("a step is one line: {step:?}"));
        }
        let filled = self.fill(step, &|_, values| values[0].as_str());
        if let Some(name) = unfilled(&filled) {
            return Err(format!("{step:?} has <{name}> with nothing to choose from"));
        }
        serde_json::from_str::<Value>(&filled)
            .map(|_| ())
            .map_err(|error| format!("{step:?} is not JSON: {error}"))
    }

    /// The step with each `<name>` replaced by the value `pick` takes from its list.
    pub(crate) fn fill<'a>(
        &'a self,
        step: &str,
        pick: &dyn Fn(&str, &'a [String]) -> &'a str,
    ) -> String {
        let mut filled = step.to_owned();
        for (name, values) in &self.choose {
            filled = filled.replace(&slot(name), pick(name, values));
        }
        filled
    }

    /// Each step once for every value of each name, the other names at their first value.
    fn every_step(&self) -> Vec<String> {
        let mut steps = Vec::new();
        for step in &self.steps {
            steps.push(self.fill(step, &|_, values| values[0].as_str()));
            for (name, values) in &self.choose {
                for value in values.iter().skip(1) {
                    let partly = step.replace(&slot(name), value);
                    steps.push(self.fill(&partly, &|_, values| values[0].as_str()));
                }
            }
        }
        steps
    }
}

fn slot(name: &str) -> String {
    format!("<{name}>")
}

/// The first `<word>` left in a step: a slot nothing filled, as against a `<` some text holds.
fn unfilled(step: &str) -> Option<&str> {
    step.match_indices('<').find_map(|(start, _)| {
        let rest = &step[start + 1..];
        let inner = &rest[..rest.find('>')?];
        let word = !inner.is_empty()
            && inner
                .chars()
                .all(|character| character.is_ascii_lowercase() || character == '_');
        word.then_some(inner)
    })
}
