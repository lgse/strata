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
//!
//! On a field whose value is a list, a rule may instead name the elements Rune's list adds:
//! `<field> <code> <issue> {"rune_adds":[<value>, …]}`. It leaves out a difference only when
//! GTK's list is Rune's with every element equal to one of them taken out, so the rest of the
//! list, its order included, still counts: a node Rune names and GTK does not, by a ruling
//! (`app.preview`'s "Volume", lgse/strata#2606).

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

/// A difference a rule excuses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Only {
    /// GTK's value `gtk` against one of Rune's `rune`.
    Values { gtk: Value, rune: Vec<Value> },
    /// A list of Rune's that is GTK's with these elements added, wherever they stand.
    RuneAdds(Vec<Value>),
}

impl Only {
    /// Whether GTK's value `gtk` against Rune's `rune` is what the rule excuses.
    fn is(&self, gtk: &Value, rune: &Value) -> bool {
        match self {
            Self::Values {
                gtk: ruled,
                rune: values,
            } => gtk == ruled && values.contains(rune),
            Self::RuneAdds(added) => {
                let (Value::Array(gtk), Value::Array(rune)) = (gtk, rune) else {
                    return false;
                };
                let kept: Vec<&Value> = rune.iter().filter(|item| !added.contains(item)).collect();
                kept.len() < rune.len() && kept.iter().copied().eq(gtk.iter())
            }
        }
    }

    /// Whether `difference` is what the rule excuses: `gtk_left` says which side GTK's stream
    /// is, when the comparison knows.
    fn excuses(&self, difference: &Difference, gtk_left: Option<bool>) -> bool {
        let is = |gtk: &Value, rune: &Value| self.is(gtk, rune);
        let (left, right) = (&difference.left, &difference.right);
        match gtk_left {
            Some(true) => is(left, right),
            Some(false) => is(right, left),
            None => is(left, right) || is(right, left),
        }
    }
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
    /// The elements the field's `rune_adds` rules name are taken out together, so a list that
    /// adds what two rulings allow is GTK's with both taken out. `gtk_left` says which side
    /// GTK's stream is, when the comparison knows.
    pub(crate) fn excuses(&self, difference: &Difference, gtk_left: Option<bool>) -> bool {
        let on_field = |entry: &&Entry| project::covers(&entry.field, &difference.field);
        let added: Vec<Value> = self
            .rules
            .iter()
            .filter(on_field)
            .filter_map(|entry| match &entry.only {
                Some(Only::RuneAdds(added)) => Some(added.iter().cloned()),
                _ => None,
            })
            .flatten()
            .collect();
        let together = (!added.is_empty()).then_some(Only::RuneAdds(added));
        self.covers(&difference.field)
            || together.is_some_and(|only| only.excuses(difference, gtk_left))
            || self.rules.iter().filter(on_field).any(|entry| {
                matches!(&entry.only, Some(only @ Only::Values { .. }) if only.excuses(difference, gtk_left))
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
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let entry = entry(line, codes).map_err(|error| format!("line {}: {error}", index + 1))?;
        if let Some(last) = entries.last()
            && sort_key(last) >= sort_key(&entry)
        {
            return Err(format!(
                "line {}: entries are sorted by field, code and issue and listed once; {} comes \
                 after {}",
                index + 1,
                entry.field,
                last.field
            ));
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// What orders the entries: a field may have a rule of one code for each ruling, each citing
/// its own issue (`app.dialog`'s, lgse/strata#2405 and lgse/strata#2572), in the issues' order.
fn sort_key(entry: &Entry) -> (&str, &str, u64) {
    let number = entry
        .issue
        .strip_prefix("lgse/strata#")
        .and_then(|number| number.parse().ok())
        .unwrap_or(u64::MAX);
    (&entry.field, &entry.code, number)
}

fn entry(line: &str, codes: &[&str]) -> Result<Entry, String> {
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
    Ok(Entry {
        field: (*field).to_owned(),
        code: (*code).to_owned(),
        issue: (*issue).to_owned(),
        only,
    })
}

/// The values after an entry's issue: `{"gtk":<value>,"rune":[<value>, …]}`, or
/// `{"rune_adds":[<value>, …]}`.
fn only(text: &str) -> Result<Only, String> {
    let wrong = || {
        format!(
            "the values an entry names are {{\"gtk\":<value>,\"rune\":[<value>, …]}} or \
             {{\"rune_adds\":[<value>, …]}}, not {text:?}"
        )
    };
    let value: Value = serde_json::from_str(text).map_err(|_| wrong())?;
    let object = value.as_object().ok_or_else(wrong)?;
    rune_adds(object)
        .or_else(|| values(object))
        .ok_or_else(wrong)
}

/// `{"rune_adds":[<value>, …]}`: one key, and at least one value.
fn rune_adds(object: &serde_json::Map<String, Value>) -> Option<Only> {
    let added = object.get("rune_adds")?.as_array()?;
    let alone = object.len() == 1;
    (alone && !added.is_empty()).then(|| Only::RuneAdds(added.clone()))
}

/// `{"gtk":<value>,"rune":[<value>, …]}`: both keys and no other, and at least one of Rune's.
fn values(object: &serde_json::Map<String, Value>) -> Option<Only> {
    let gtk = object.get("gtk")?;
    let rune = object.get("rune")?.as_array()?;
    let both = object.len() == 2;
    (both && !rune.is_empty()).then(|| Only::Values {
        gtk: gtk.clone(),
        rune: rune.clone(),
    })
}

pub(crate) fn is_issue(text: &str) -> bool {
    text.strip_prefix("lgse/strata#").is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}
