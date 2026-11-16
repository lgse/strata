// SPDX-License-Identifier: MIT

//! MIME-keyed recent-application history (`strata/recent-apps.toml`), matching
//! `src/ui/recent_apps/mod.rs` on `strata-core`'s background storage writer.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
    path::{Path, PathBuf},
};

use strata_core::storage::{atomic_write, writer};

pub(super) const MAX_PER_TYPE: usize = 5;
const STATE_VERSION: u32 = 1;
const STATE_FILE: &str = "strata/recent-apps.toml";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    pub(super) version: u32,
    pub(super) types: HashMap<String, Vec<Entry>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) app: String,
    pub(super) seq: u64,
}

#[derive(Clone, Copy)]
struct TomlSlice<'a>(&'a str);

fn history_path() -> PathBuf {
    glib::user_data_dir().join(STATE_FILE)
}

pub(super) fn record(content_types: &[String], app_id: &str, known_ids: HashSet<String>) {
    if app_id.is_empty() || content_types.is_empty() {
        return;
    }
    let (content_types, app_id) = (content_types.to_vec(), app_id.to_owned());
    drop(writer::run(move || {
        let path = history_path();
        let mut state = load_from(&path);
        record_in(&mut state, &content_types, &app_id);
        prune_unknown(&mut state, &known_ids);
        save_to(&path, &mut state);
    }));
}

pub(super) async fn read() -> State {
    writer::run(|| load_from(&history_path()))
        .await
        .unwrap_or_default()
}

fn read_history_file(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "unable to read recent application history");
            None
        }
    }
}

pub(super) fn load_from(path: &Path) -> State {
    strata_core::util::ui_thread::assert_off();
    let Some(contents) = read_history_file(path) else {
        return State::default();
    };
    match parse_state(&contents) {
        Ok(state) if state.version == STATE_VERSION => state,
        Ok(_) => {
            tracing::warn!(path = %path.display(), "ignoring recent application history with an unsupported version");
            State::default()
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "ignoring malformed recent application history");
            State::default()
        }
    }
}

pub(super) fn save_to(path: &Path, state: &mut State) {
    state.version = STATE_VERSION;
    let contents = serialize_state(state);
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(path = %path.display(), %error, "unable to create recent application history directory");
        return;
    }
    if let Err(error) = atomic_write(path, contents.as_bytes()) {
        tracing::warn!(path = %path.display(), %error, "unable to save recent application history");
    }
}

pub(super) fn record_in(state: &mut State, content_types: &[String], app_id: &str) {
    let base = next_seq(state, content_types.len());
    for (offset, content_type) in content_types.iter().enumerate() {
        let entries = state.types.entry(content_type.clone()).or_default();
        entries.retain(|entry| entry.app != app_id);
        entries.insert(
            0,
            Entry {
                app: app_id.to_owned(),
                seq: base + offset as u64,
            },
        );
        entries.truncate(MAX_PER_TYPE);
    }
}

fn next_seq(state: &mut State, count: usize) -> u64 {
    let maximum = state
        .types
        .values()
        .flatten()
        .map(|entry| entry.seq)
        .max()
        .unwrap_or(0);
    if maximum.checked_add(count as u64).is_some() {
        return maximum + 1;
    }
    let mut sequences: Vec<u64> = state
        .types
        .values()
        .flatten()
        .map(|entry| entry.seq)
        .collect();
    sequences.sort_unstable();
    sequences.dedup();
    for entry in state.types.values_mut().flatten() {
        entry.seq = sequences
            .binary_search(&entry.seq)
            .expect("existing sequence") as u64
            + 1;
    }
    sequences.len() as u64 + 1
}

pub(super) fn prune_unknown(state: &mut State, known_ids: &HashSet<String>) {
    if known_ids.is_empty() {
        return;
    }
    state.types.retain(|_, entries| {
        entries.retain(|entry| known_ids.contains(entry.app.as_str()));
        !entries.is_empty()
    });
}

impl State {
    pub(super) fn recent_ids(&self, content_types: &[String]) -> Vec<String> {
        let mut ordered: Vec<(&str, u64)> = Vec::new();
        for content_type in content_types {
            if let Some(entries) = self.types.get(content_type) {
                ordered.extend(entries.iter().map(|entry| (entry.app.as_str(), entry.seq)));
            }
        }
        ordered.sort_by_key(|entry| Reverse(entry.1));
        let mut seen = HashSet::new();
        ordered
            .into_iter()
            .filter(|(app, _)| seen.insert(*app))
            .map(|(app, _)| app.to_owned())
            .collect()
    }
}

pub(super) fn select_recent(ordered_ids: &[String], eligible_ids: &HashSet<String>) -> Vec<String> {
    ordered_ids
        .iter()
        .filter(|id| eligible_ids.contains(id.as_str()))
        .cloned()
        .collect()
}

pub(super) fn serialize_state(state: &State) -> String {
    let mut out = format!("version = {}\n", state.version);
    let ordered: BTreeMap<&str, &Vec<Entry>> = state
        .types
        .iter()
        .filter(|(_, entries)| !entries.is_empty())
        .map(|(mime, entries)| (mime.as_str(), entries))
        .collect();
    for (mime, entries) in ordered {
        for entry in entries {
            let _ = writeln!(
                out,
                "\n[[types.{}]]\napp = {}\nseq = {}",
                quote_toml(TomlSlice(mime)),
                quote_toml(TomlSlice(&entry.app)),
                entry.seq
            );
        }
    }
    out
}

fn quote_toml(value: TomlSlice<'_>) -> String {
    let mut out = String::with_capacity(value.0.len() + 2);
    out.push('"');
    for ch in value.0.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[derive(Default)]
enum Section {
    #[default]
    Root,
    TypesTable,
    TypeEntry {
        mime: String,
        app: Option<String>,
        seq: Option<u64>,
    },
}

impl Section {
    fn flush(&mut self, state: &mut State) -> Result<(), &'static str> {
        if let Self::TypeEntry { mime, app, seq } = std::mem::take(self) {
            let entry = Entry {
                app: app.ok_or("missing app in [[types]] entry")?,
                seq: seq.ok_or("missing seq in [[types]] entry")?,
            };
            state.types.entry(mime).or_default().push(entry);
        }
        Ok(())
    }
}

pub(super) fn parse_state(contents: &str) -> Result<State, &'static str> {
    let mut state = State::default();
    let mut section = Section::Root;
    for raw in contents.lines() {
        let line = strip_comment(TomlSlice(raw)).0.trim();
        if line.is_empty() {
            continue;
        }
        if line == "[types]" {
            section.flush(&mut state)?;
            section = Section::TypesTable;
        } else if let Some(inner) = line
            .strip_prefix("[[types.")
            .and_then(|s| s.strip_suffix("]]"))
        {
            section.flush(&mut state)?;
            section = Section::TypeEntry {
                mime: parse_key(TomlSlice(inner.trim()))?,
                app: None,
                seq: None,
            };
        } else {
            apply_assignment(&mut state, &mut section, TomlSlice(line))?;
        }
    }
    section.flush(&mut state)?;
    Ok(state)
}

fn apply_assignment(
    state: &mut State,
    section: &mut Section,
    line: TomlSlice<'_>,
) -> Result<(), &'static str> {
    let (lhs, rhs) = split_assignment(line)?;
    match section {
        Section::Root if lhs.0 == "version" => {
            state.version = rhs.0.parse::<u32>().map_err(|_| "invalid version")?;
            Ok(())
        }
        Section::TypesTable => {
            let mime = parse_key(lhs)?;
            state.types.insert(mime, parse_inline_entries(rhs)?);
            Ok(())
        }
        Section::TypeEntry { app, seq, .. } => match lhs.0 {
            "app" => {
                *app = Some(parse_quoted(rhs)?);
                Ok(())
            }
            "seq" => {
                *seq = Some(rhs.0.parse::<u64>().map_err(|_| "invalid seq")?);
                Ok(())
            }
            _ => Err("unexpected key in [[types]] entry"),
        },
        Section::Root => Err("unexpected root key"),
    }
}

fn parse_inline_entries(rhs: TomlSlice<'_>) -> Result<Vec<Entry>, &'static str> {
    let inner = rhs
        .0
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or("expected inline entry array")?
        .trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    split_outside(TomlSlice(inner), b',')
        .into_iter()
        .filter(|part| !part.0.trim().is_empty())
        .map(parse_inline_entry)
        .collect()
}

fn parse_inline_entry(part: TomlSlice<'_>) -> Result<Entry, &'static str> {
    let body = part
        .0
        .trim()
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .ok_or("expected inline entry table")?;
    let mut app = None;
    let mut seq = None;
    for field in split_outside(TomlSlice(body), b',') {
        let trimmed = field.0.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (k, v) = split_assignment(TomlSlice(trimmed))?;
        match k.0 {
            "app" => app = Some(parse_quoted(v)?),
            "seq" => seq = Some(v.0.parse::<u64>().map_err(|_| "invalid inline seq")?),
            _ => return Err("unexpected inline entry field"),
        }
    }
    Ok(Entry {
        app: app.ok_or("missing inline app")?,
        seq: seq.ok_or("missing inline seq")?,
    })
}

struct ScanState {
    in_quotes: bool,
    depth: usize,
}

impl ScanState {
    fn step(&mut self, byte: u8) -> usize {
        if self.in_quotes {
            if byte == b'\\' {
                return 2;
            }
            if byte == b'"' {
                self.in_quotes = false;
            }
            return 1;
        }
        match byte {
            b'"' => self.in_quotes = true,
            b'{' | b'[' => self.depth += 1,
            b'}' | b']' => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        1
    }

    fn at_top(&self) -> bool {
        !self.in_quotes && self.depth == 0
    }
}

fn split_outside(input: TomlSlice<'_>, delimiter: u8) -> Vec<TomlSlice<'_>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut scan = ScanState {
        in_quotes: false,
        depth: 0,
    };
    let bytes = input.0.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        if scan.at_top() && bytes[idx] == delimiter {
            parts.push(TomlSlice(&input.0[start..idx]));
            start = idx + 1;
            idx += 1;
        } else {
            idx += scan.step(bytes[idx]);
        }
    }
    parts.push(TomlSlice(&input.0[start..]));
    parts
}

fn strip_comment(line: TomlSlice<'_>) -> TomlSlice<'_> {
    let mut scan = ScanState {
        in_quotes: false,
        depth: 0,
    };
    let bytes = line.0.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        if !scan.in_quotes && bytes[idx] == b'#' {
            return TomlSlice(&line.0[..idx]);
        }
        idx += scan.step(bytes[idx]);
    }
    line
}

fn split_assignment(line: TomlSlice<'_>) -> Result<(TomlSlice<'_>, TomlSlice<'_>), &'static str> {
    let mut scan = ScanState {
        in_quotes: false,
        depth: 0,
    };
    let bytes = line.0.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        if !scan.in_quotes && bytes[idx] == b'=' {
            return Ok((
                TomlSlice(line.0[..idx].trim()),
                TomlSlice(line.0[idx + 1..].trim()),
            ));
        }
        idx += scan.step(bytes[idx]);
    }
    Err("missing '=' in assignment")
}

fn parse_key(raw: TomlSlice<'_>) -> Result<String, &'static str> {
    if raw.0.starts_with('"') {
        return parse_quoted(raw);
    }
    let valid = !raw.0.is_empty()
        && raw
            .0
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    valid.then(|| raw.0.to_owned()).ok_or("invalid bare key")
}

fn parse_quoted(raw: TomlSlice<'_>) -> Result<String, &'static str> {
    let body = raw
        .0
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or("expected quoted string")?;
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next().ok_or("unterminated escape")? {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            _ => return Err("unsupported escape"),
        }
    }
    Ok(out)
}
