// SPDX-License-Identifier: MIT

//! The corpus, `tests/qa/fuzz/corpus/<area>/`: the sequences an area keeps, each a scenario
//! file named for where it came from, `fuzz-<area>-v<version>-<seed>`, and the ones it has
//! parked against an issue.
//!
//! A kept sequence is one that showed a divergence. While its issue is open it is parked:
//! `parked` lists it as `v<version>-<seed> <code> <issue>`, sorted, and neither a run of
//! that seed on that version of the alphabet nor a replay of the corpus reports it again.
//! Once the issue is fixed its line goes, and a replay holds the sequence to agreeing.
//!
//! A seed means nothing across versions of an alphabet, so a parked line stops a run only
//! of the version it names. For the alphabet's own version the kept file has to be made of
//! steps the seed still writes, in their order: what is skipped is what was parked.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    DATA,
    alphabet::Alphabet,
    generate::{Origin, generate},
    rules::{exceptions, is_issue},
    sequence::Sequence,
};
use crate::REPOSITORY;

pub(crate) const PARKED: &str = "parked";
/// A parked sequence whose issue is a `divergence` still being worked on, with no
/// exception code yet.
const DIVERGENCE: &str = "divergence";
/// How many actions of a seed a kept file of the alphabet's own version is looked for in:
/// twice the long sequences calibration ran. A longer sequence of a seed starts with the
/// shorter one, so this is every run up to that length.
const HORIZON: usize = 240;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Parked {
    pub(crate) version: u32,
    pub(crate) seed: u64,
    pub(crate) code: String,
    pub(crate) issue: String,
}

pub(crate) fn directory(area: &str) -> PathBuf {
    Path::new(REPOSITORY).join(DATA).join("corpus").join(area)
}

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

fn files(area: &str) -> Vec<PathBuf> {
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

/// The area's sequences with where each came from, which its file's name has to say.
pub(crate) fn sequences(area: &str) -> Result<Vec<(Origin, Sequence)>, String> {
    files(area)
        .iter()
        .map(|path| {
            let sequence = Sequence::read(path)?;
            match Origin::of(&sequence.name) {
                Some(origin) if origin.area == area => Ok((origin, sequence)),
                _ => Err(format!(
                    "{}: a corpus file is named fuzz-{area}-v<version>-<seed>, the seed as \
                     sixteen hex digits",
                    path.display()
                )),
            }
        })
        .collect()
}

/// The sequences the area has parked; none when it has no `parked` file.
pub(crate) fn parked(area: &str) -> Result<Vec<Parked>, String> {
    let path = directory(area).join(PARKED);
    match fs::read_to_string(&path) {
        Ok(text) => parse_parked(&text).map_err(|error| format!("{}: {error}", path.display())),
        Err(_) => Ok(Vec::new()),
    }
}

/// The parked line that names this seed on this version of the alphabet, if one does.
pub(crate) fn parked_as(parked: &[Parked], version: u32, seed: u64) -> Option<&Parked> {
    parked
        .iter()
        .find(|parked| (parked.version, parked.seed) == (version, seed))
}

/// Holds an area's corpus to its alphabet: every file comes from a version the alphabet
/// has had and loads as a sequence, every parked line has its file, and a file parked on
/// the alphabet's own version is made of steps its seed still writes. Returns how many
/// sequences are kept and how many of them are parked.
pub(crate) fn check(alphabet: &Alphabet) -> Result<(usize, usize), String> {
    let area = &alphabet.area;
    let sequences = sequences(area)?;
    for (origin, sequence) in &sequences {
        if origin.version > alphabet.version {
            return Err(format!(
                "{}: the alphabet {area} is at version {}",
                sequence.name, alphabet.version
            ));
        }
        sequence.load()?;
    }
    let parked = parked(area)?;
    for entry in &parked {
        let name = Origin {
            area: area.clone(),
            version: entry.version,
            seed: entry.seed,
        }
        .name();
        let (_, kept) = sequences
            .iter()
            .find(|(_, sequence)| sequence.name == name)
            .ok_or_else(|| format!("{area}: {name} is parked and has no file in the corpus"))?;
        if entry.version == alphabet.version && !still_written(alphabet, entry.seed, kept) {
            return Err(format!(
                "{name}: seed {:#x} of {area} v{} no longer writes these steps, so the \
                 parked line would stop a run of some other sequence",
                entry.seed, alphabet.version
            ));
        }
    }
    Ok((sequences.len(), parked.len()))
}

/// Whether the kept steps are steps the seed writes today, in their order: a sequence cut
/// short or shrunk is, and one kept under another alphabet's steps is not. It catches a
/// file kept under the wrong version; it does not prove where a short file came from.
pub(crate) fn still_written(alphabet: &Alphabet, seed: u64, kept: &Sequence) -> bool {
    let written = generate(alphabet, seed, HORIZON).steps;
    let mut written = written.iter();
    kept.steps
        .iter()
        .all(|step| written.any(|candidate| candidate == step))
}

pub(crate) fn parse_parked(text: &str) -> Result<Vec<Parked>, String> {
    let mut parked: Vec<Parked> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let entry = parked_entry(line).map_err(|error| format!("line {}: {error}", index + 1))?;
        if parked
            .last()
            .is_some_and(|last| (last.version, last.seed) >= (entry.version, entry.seed))
        {
            return Err(format!(
                "line {}: entries are sorted by version and seed, and parked once",
                index + 1
            ));
        }
        parked.push(entry);
    }
    Ok(parked)
}

fn parked_entry(line: &str) -> Result<Parked, String> {
    let columns: Vec<&str> = line.split_whitespace().collect();
    let [origin, code, issue] = columns.as_slice() else {
        return Err("an entry is v<version>-<seed> <code> <issue>".to_owned());
    };
    // The end of a sequence's name, so the line reads as the file does.
    let named = Origin::of(&format!("fuzz-parked-{origin}")).ok_or_else(|| {
        format!(
            "a parked sequence is written v<version>-<seed>, the seed as sixteen hex digits, \
             not {origin:?}"
        )
    })?;
    let codes = exceptions();
    if !codes.contains(code) && *code != DIVERGENCE {
        return Err(format!(
            "the code is {DIVERGENCE} or one of {}, not {code:?}",
            codes.join(", ")
        ));
    }
    if !is_issue(issue) {
        return Err(format!(
            "the issue is written lgse/strata#<number>, not {issue:?}"
        ));
    }
    Ok(Parked {
        version: named.version,
        seed: named.seed,
        code: (*code).to_owned(),
        issue: (*issue).to_owned(),
    })
}
