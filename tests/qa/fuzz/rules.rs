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
//! Both are one entry to a line, `<field> <code> <issue>`, sorted by field, so two branches
//! that each add a line merge cleanly. A line that starts with `#` is a comment.
//!
//! A rule of the map may name the values it excuses, after the issue:
//! `<field> <code> <issue> {"gtk":<value>,"rune":[<value>, …]}`. It then leaves out only a
//! difference between GTK's value and one of Rune's on that field, every other difference on
//! it still counting: an owner's ruling on one deviation, kept narrow
//! (`app.dialog`'s, lgse/strata#2405: GTK's popovers are no dialogs, Rune's plain ones are).

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

/// A difference a rule excuses: GTK's value `gtk` against one of Rune's `rune`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Only {
    pub(crate) gtk: Value,
    pub(crate) rune: Vec<Value>,
}

impl Only {
    /// Whether `difference` is GTK's value against one of Rune's: `gtk_left` says which side
    /// GTK's stream is, when the comparison knows.
    fn excuses(&self, difference: &Difference, gtk_left: Option<bool>) -> bool {
        let is = |gtk: &Value, rune: &Value| *gtk == self.gtk && self.rune.contains(rune);
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
    /// `gtk_left` says which side GTK's stream is, when the comparison knows.
    pub(crate) fn excuses(&self, difference: &Difference, gtk_left: Option<bool>) -> bool {
        self.covers(&difference.field)
            || self.rules.iter().any(|entry| {
                entry.only.as_ref().is_some_and(|only| {
                    project::covers(&entry.field, &difference.field)
                        && only.excuses(difference, gtk_left)
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
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let entry = entry(line, codes).map_err(|error| format!("line {}: {error}", index + 1))?;
        if let Some(last) = entries.last()
            && (last.field.as_str(), last.code.as_str())
                >= (entry.field.as_str(), entry.code.as_str())
        {
            return Err(format!(
                "line {}: entries are sorted by field and listed once; {} comes after {}",
                index + 1,
                entry.field,
                last.field
            ));
        }
        entries.push(entry);
    }
    Ok(entries)
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

/// The values after an entry's issue: `{"gtk":<value>,"rune":[<value>, …]}`.
fn only(text: &str) -> Result<Only, String> {
    let wrong = || {
        format!(
            "the values an entry names are {{\"gtk\":<value>,\"rune\":[<value>, …]}}, not {text:?}"
        )
    };
    let value: Value = serde_json::from_str(text).map_err(|_| wrong())?;
    let object = value
        .as_object()
        .filter(|object| object.len() == 2)
        .ok_or_else(wrong)?;
    let (Some(gtk), Some(Value::Array(rune))) = (object.get("gtk"), object.get("rune")) else {
        return Err(wrong());
    };
    if rune.is_empty() {
        return Err(wrong());
    }
    Ok(Only {
        gtk: gtk.clone(),
        rune: rune.clone(),
    })
}

pub(crate) fn is_issue(text: &str) -> bool {
    text.strip_prefix("lgse/strata#").is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}
