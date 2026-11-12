// SPDX-License-Identifier: MIT

//! The corpus, `tests/qa/fuzz/corpus/<area>/`: the shrunk sequences an area keeps, each a
//! scenario file named for its seed, and the seeds it has parked against an issue.
//!
//! A kept sequence is one that diverged and was fixed: the smoke tier replays it and it
//! must agree. A parked seed is one whose divergence has an open issue: `parked` lists it
//! as `<seed> <code> <issue>`, sorted by seed, and a run names it and does not report it
//! again.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    DATA,
    generate::seed_of,
    rules::{EXCEPTIONS, is_issue},
    sequence::Sequence,
};
use crate::REPOSITORY;

pub(crate) const PARKED: &str = "parked";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Parked {
    pub(crate) seed: u64,
    pub(crate) code: String,
    pub(crate) issue: String,
}

pub(crate) fn directory(area: &str) -> PathBuf {
    Path::new(REPOSITORY).join(DATA).join("corpus").join(area)
}

/// The areas that keep a corpus, in name order.
pub(crate) fn areas() -> Vec<String> {
    let mut areas: Vec<String> = fs::read_dir(Path::new(REPOSITORY).join(DATA).join("corpus"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    areas.sort();
    areas
}

/// The area's sequence files, in name order.
pub(crate) fn files(area: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(directory(area))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect();
    files.sort();
    files
}

/// The area's sequences. Each is named `fuzz-<area>-<seed>`, as its file is.
pub(crate) fn sequences(area: &str) -> Result<Vec<Sequence>, String> {
    files(area)
        .iter()
        .map(|path| {
            let sequence = Sequence::read(path)?;
            match seed_of(&sequence.name) {
                Some(seed) if sequence.name == super::generate::name(area, seed) => Ok(sequence),
                _ => Err(format!(
                    "{}: a corpus file is named fuzz-{area}-<seed>, the seed as sixteen hex digits",
                    path.display()
                )),
            }
        })
        .collect()
}

/// The seeds the area has parked; none when it has no `parked` file.
pub(crate) fn parked(area: &str) -> Result<Vec<Parked>, String> {
    let path = directory(area).join(PARKED);
    match fs::read_to_string(&path) {
        Ok(text) => parse_parked(&text).map_err(|error| format!("{}: {error}", path.display())),
        Err(_) => Ok(Vec::new()),
    }
}

pub(crate) fn parse_parked(text: &str) -> Result<Vec<Parked>, String> {
    let mut parked: Vec<Parked> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let entry = parked_entry(line).map_err(|error| format!("line {}: {error}", index + 1))?;
        if parked.last().is_some_and(|last| last.seed >= entry.seed) {
            return Err(format!(
                "line {}: seeds are sorted and parked once",
                index + 1
            ));
        }
        parked.push(entry);
    }
    Ok(parked)
}

fn parked_entry(line: &str) -> Result<Parked, String> {
    let columns: Vec<&str> = line.split_whitespace().collect();
    let [seed, code, issue] = columns.as_slice() else {
        return Err("an entry is <seed> <code> <issue>".to_owned());
    };
    let seed = (seed.len() == 16)
        .then(|| u64::from_str_radix(seed, 16).ok())
        .flatten()
        .ok_or_else(|| format!("the seed is sixteen hex digits, not {seed:?}"))?;
    if !EXCEPTIONS.contains(code) && *code != DIVERGENCE {
        return Err(format!(
            "the code is {DIVERGENCE} or one of {}, not {code:?}",
            EXCEPTIONS.join(", ")
        ));
    }
    if !is_issue(issue) {
        return Err(format!(
            "the issue is written lgse/strata#<number>, not {issue:?}"
        ));
    }
    Ok(Parked {
        seed,
        code: (*code).to_owned(),
        issue: (*issue).to_owned(),
    })
}

/// A parked seed whose issue is a `divergence` still being worked on, with no exception
/// code yet.
const DIVERGENCE: &str = "divergence";
