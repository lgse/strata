// SPDX-License-Identifier: MIT

//! The two lists of fields a comparison leaves out, and why they are two.
//!
//! `normalisation.map` holds the exceptions: a field on which the two targets may differ,
//! under one of the plan's exception codes and an issue. It applies only when the streams
//! come from different targets.
//!
//! `calibration.mask` holds the noise: a field on which two replays of one sequence on one
//! target differ. It applies to every comparison, since a field that does not agree with
//! itself cannot be compared with anything. A mask is not an exception and is counted apart.
//!
//! Both are one entry to a line, `<field> <code> <issue>`, sorted by field, code and issue, so
//! two branches that each add a line merge cleanly. A line that starts with `#` is a comment.
//!
//! A rule of the map may name the values it excuses, after the issue:
//! `<field> <code> <issue> {"gtk":<value>,"rune":[<value>, …]}`. It then leaves out only a
//! difference between GTK's value and one of Rune's on that field, every other difference on
//! it still counting: an owner's ruling on one deviation, kept narrow
//! (`app.dialog`'s, lgse/strata#2405: GTK's popovers are no dialogs, Rune's plain ones are).
//! Without `"rune"` it leaves out GTK's value against any of Rune's, and without `"gtk"` any of
//! GTK's against one of Rune's. With `"while":{"gtk":{<field>:<value>, …},"rune":{…}}` it
//! applies only to a step at which each side's observation holds those values: a field is a
//! key of `app` or `context.toplevels`, and a value is one value or `{"one_of":[<value>, …]}`
//! (lgse/strata#2595: focus while one side's browser has no focused pane). A rule may then name
//! neither, and leaves out every difference on its field there. Rules of one field, code and
//! issue are sorted by what follows the issue.

use std::{fs, path::Path};

use serde_json::Value;

use super::{DATA, project, project::Difference};
use crate::{
    REPOSITORY,
    twins::{PLATFORM, TWIN_BEARING},
};

pub(crate) const MAP: &str = "normalisation.map";
pub(crate) const MASK: &str = "calibration.mask";
/// The plan's exception codes (`docs/rune/parity/README.md`, "Exceptions to total parity"),
/// as the runner's twins spell them: one list for a twin's label and for a rule of the map.
pub(crate) fn exceptions() -> Vec<&'static str> {
    TWIN_BEARING.iter().copied().chain([PLATFORM]).collect()
}

/// Why two replays on one target can differ on a field (`docs/testing.md`, "Calibration").
/// `unsettled`: the application is still working when the reply is written and shows
/// nothing to wait on. `clock`: the value follows the time of day or how long the run took.
/// `environment`: the value follows the machine, such as a path or a size outside the fixture.
pub(crate) const CAUSES: [&str; 3] = ["unsettled", "clock", "environment"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) field: String,
    pub(crate) code: String,
    pub(crate) issue: String,
    /// The values a rule of the map excuses on its field, when it names them.
    pub(crate) only: Option<Only>,
}

/// A difference a rule excuses: GTK's value `gtk` against one of Rune's `rune`, at a step at
/// which each `when` holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Only {
    /// GTK's value; any, when the rule names none.
    pub(crate) gtk: Option<Value>,
    /// Rune's values; any, when the rule names none.
    pub(crate) rune: Option<Vec<Value>>,
    /// What the observations must hold for the rule to apply.
    pub(crate) when: Vec<Condition>,
}

/// One side's field, `app.<key>` or `context.toplevels`, and the values it may hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Condition {
    pub(crate) rune: bool,
    pub(crate) field: String,
    pub(crate) one_of: Vec<Value>,
}

/// Which stream is GTK's, when the comparison knows, and each target's observation at the
/// step.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sides<'a> {
    pub(crate) gtk_left: Option<bool>,
    pub(crate) gtk: Option<&'a Value>,
    pub(crate) rune: Option<&'a Value>,
}

impl Only {
    /// Whether `difference` is GTK's value against one of Rune's, at a step at which every
    /// condition holds. A condition needs the observations, so it excuses nothing where the
    /// comparison does not know which stream is whose.
    fn excuses(&self, difference: &Difference, sides: Sides<'_>) -> bool {
        let holds = |condition: &Condition| {
            let observation = if condition.rune {
                sides.rune
            } else {
                sides.gtk
            };
            observation
                .and_then(|observation| field_of(observation, &condition.field))
                .is_some_and(|value| condition.one_of.contains(value))
        };
        if !self.when.iter().all(holds) {
            return false;
        }
        let is = |gtk: &Value, rune: &Value| {
            self.gtk.as_ref().is_none_or(|value| gtk == value)
                && self
                    .rune
                    .as_ref()
                    .is_none_or(|values| values.contains(rune))
        };
        let (left, right) = (&difference.left, &difference.right);
        match sides.gtk_left {
            Some(true) => is(left, right),
            Some(false) => is(right, left),
            None => is(left, right) || is(right, left),
        }
    }
}

/// The value of `field`, `<root>.<key>`, in an observation.
fn field_of<'a>(observation: &'a Value, field: &str) -> Option<&'a Value> {
    let (root, key) = field.split_once('.')?;
    observation.get(root)?.get(key)
}

/// The fields one comparison leaves out.
#[derive(Clone, Debug, Default)]
pub(crate) struct Ignored {
    pub(crate) rules: Vec<Entry>,
    pub(crate) masks: Vec<Entry>,
}

impl Ignored {
    /// What the tree holds. The rules count only between two targets, so a comparison of
    /// one target with itself is given `same_target` and leaves them out.
    pub(crate) fn load(same_target: bool) -> Result<Self, String> {
        let masks = read(MASK, &CAUSES)?;
        if let Some(entry) = masks.iter().find(|entry| entry.only.is_some()) {
            return Err(format!(
                "{DATA}/{MASK}: {} names values: only a rule of the map may",
                entry.field
            ));
        }
        let mut ignored = Self {
            rules: read(MAP, &exceptions())?,
            masks,
        };
        if same_target {
            ignored.rules.clear();
        }
        Ok(ignored)
    }

    /// Whether every difference on `field` is left out: a mask, or a rule that names no
    /// values.
    pub(crate) fn covers(&self, field: &str) -> bool {
        self.rules
            .iter()
            .chain(&self.masks)
            .filter(|entry| entry.only.is_none())
            .any(|entry| project::covers(&entry.field, field))
    }

    /// Whether `difference` is left out: its field is covered, or a rule names its values.
    /// `sides` says which stream is GTK's, and its observation, when the comparison knows.
    pub(crate) fn excuses(&self, difference: &Difference, sides: Sides<'_>) -> bool {
        self.covers(&difference.field)
            || self.rules.iter().any(|entry| {
                entry.only.as_ref().is_some_and(|only| {
                    project::covers(&entry.field, &difference.field)
                        && only.excuses(difference, sides)
                })
            })
    }
}

/// How many entries each code has, in the order the codes are listed.
pub(crate) fn counts(entries: &[Entry], codes: &[&str]) -> String {
    let by_code: Vec<String> = codes
        .iter()
        .filter_map(|code| {
            let count = entries.iter().filter(|entry| entry.code == *code).count();
            (count > 0).then(|| format!("{code} {count}"))
        })
        .collect();
    if by_code.is_empty() {
        entries.len().to_string()
    } else {
        format!("{} ({})", entries.len(), by_code.join(", "))
    }
}

pub(crate) fn read(file: &str, codes: &[&str]) -> Result<Vec<Entry>, String> {
    let path = Path::new(REPOSITORY).join(DATA).join(file);
    let text = fs::read_to_string(&path).map_err(|error| format!("{DATA}/{file}: {error}"))?;
    parse(&text, codes).map_err(|error| format!("{DATA}/{file}: {error}"))
}

pub(crate) fn parse(text: &str, codes: &[&str]) -> Result<Vec<Entry>, String> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut last_key: Option<(String, String, u64, String)> = None;
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (entry, values) =
            entry(line, codes).map_err(|error| format!("line {}: {error}", index + 1))?;
        let key = sort_key(&entry, values);
        if let Some(last) = entries.last()
            && last_key.as_ref().is_some_and(|last_key| *last_key >= key)
        {
            return Err(format!(
                "line {}: entries are sorted by field, code and issue, then by what follows the \
                 issue, and listed once; {} comes after {}",
                index + 1,
                entry.field,
                last.field
            ));
        }
        last_key = Some(key);
        entries.push(entry);
    }
    Ok(entries)
}

/// What orders the entries: a field may have a rule of one code for each ruling, each citing
/// its own issue (`app.dialog`'s, lgse/strata#2405 and lgse/strata#2572), in the issues' order,
/// and one ruling more than one rule, ordered by what follows the issue (lgse/strata#2595's).
fn sort_key(entry: &Entry, values: &str) -> (String, String, u64, String) {
    let number = entry
        .issue
        .strip_prefix("lgse/strata#")
        .and_then(|number| number.parse().ok())
        .unwrap_or(u64::MAX);
    (
        entry.field.clone(),
        entry.code.clone(),
        number,
        values.to_owned(),
    )
}

/// An entry, and the text after its issue.
fn entry<'a>(line: &'a str, codes: &[&str]) -> Result<(Entry, &'a str), String> {
    let mut rest = line.trim();
    let mut columns: Vec<&str> = Vec::new();
    while columns.len() < 3 && !rest.is_empty() {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        columns.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    let [field, code, issue] = columns.as_slice() else {
        return Err("an entry is <field> <code> <issue>".to_owned());
    };
    let only = (!rest.is_empty()).then(|| only(rest)).transpose()?;
    if !project::names_a_field(field) {
        return Err(format!(
            "{field:?} names no field an entry may leave out: a key of app, a path of fs or \
             a key of settings, with a list written []. A whole root, result, criticals and \
             invariants are not left out"
        ));
    }
    if !codes.contains(code) {
        return Err(format!(
            "the code is one of {}, not {code:?}",
            codes.join(", ")
        ));
    }
    if !is_issue(issue) {
        return Err(format!(
            "the issue is written lgse/strata#<number>, not {issue:?}"
        ));
    }
    let entry = Entry {
        field: (*field).to_owned(),
        code: (*code).to_owned(),
        issue: (*issue).to_owned(),
        only,
    };
    Ok((entry, rest))
}

/// The values after an entry's issue: `{"gtk":<value>,"rune":[<value>, …],"while":{"gtk":
/// {<field>:<value>, …},"rune":{…}}}`, with one of the three at least.
fn only(text: &str) -> Result<Only, String> {
    let wrong = || {
        format!(
            "the values an entry names are {{\"gtk\":<value>,\"rune\":[<value>, …],\"while\":\
             {{\"gtk\":{{<field>:<value>, …}},\"rune\":{{…}}}}}}, with one of the three at least, a \
             field app.<key> or context.toplevels and a value one value or \
             {{\"one_of\":[<value>, …]}}, not {text:?}"
        )
    };
    let value: Value = serde_json::from_str(text).map_err(|_| wrong())?;
    let object = value.as_object().ok_or_else(wrong)?;
    let known = ["gtk", "rune", "while"];
    if object.is_empty() || object.keys().any(|key| !known.contains(&key.as_str())) {
        return Err(wrong());
    }
    let rune = match object.get("rune") {
        None => None,
        Some(Value::Array(rune)) if !rune.is_empty() => Some(rune.clone()),
        Some(_) => return Err(wrong()),
    };
    let when = match object.get("while") {
        None => Vec::new(),
        Some(condition) => conditions(condition).ok_or_else(wrong)?,
    };
    Ok(Only {
        gtk: object.get("gtk").cloned(),
        rune,
        when,
    })
}

/// A `while`: for `gtk`, `rune` or both, the fields of that side's observation and their
/// values.
fn conditions(condition: &Value) -> Option<Vec<Condition>> {
    let sides = condition.as_object().filter(|sides| !sides.is_empty())?;
    let mut when = Vec::new();
    for (side, fields) in sides {
        let rune = match side.as_str() {
            "gtk" => false,
            "rune" => true,
            _ => return None,
        };
        when.extend(side_conditions(rune, fields)?);
    }
    Some(when)
}

/// One side's fields and values in a `while`.
fn side_conditions(rune: bool, fields: &Value) -> Option<Vec<Condition>> {
    let fields = fields.as_object().filter(|fields| !fields.is_empty())?;
    let condition = |(field, value): (&String, &Value)| {
        let one_of = alternatives(value).filter(|_| project::names_a_condition(field))?;
        Some(Condition {
            rune,
            field: field.clone(),
            one_of,
        })
    };
    fields.iter().map(condition).collect()
}

/// One value, or the values of `{"one_of":[<value>, …]}`.
fn alternatives(value: &Value) -> Option<Vec<Value>> {
    match value.as_object() {
        Some(object) if object.len() == 1 && object.contains_key("one_of") => {
            let one_of = object["one_of"]
                .as_array()
                .filter(|one_of| !one_of.is_empty())?;
            Some(one_of.clone())
        }
        _ => Some(vec![value.clone()]),
    }
}

pub(crate) fn is_issue(text: &str) -> bool {
    text.strip_prefix("lgse/strata#").is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}
