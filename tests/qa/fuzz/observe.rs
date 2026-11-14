// SPDX-License-Identifier: MIT

//! What is observed after a step, and when: the projected `app` snapshot, the fixture tree
//! and `settings.toml`, read again and again until they stop changing.
//!
//! A reply is written while the application may still be working, so one read is not an
//! observation (`docs/rune/slice/V3-qa-backend-trait.md`, "Replies"). Rest is what the runner
//! can see from outside: the same sample several times running. The wait is a budget and
//! never an assertion: a step that does not come to rest is recorded as unsettled and its
//! state is not compared.

use std::{
    fmt::Write as _,
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::project::{APP, CRITICALS, FS, INVARIANTS, RESULT, SETTINGS};

/// How rest is told. Calibration set these (`docs/testing.md`, "Calibration"): GTK goes on
/// changing for up to about 220 ms after a reply with nothing to see in between, and the
/// quiet window of 350 ms is that with a margin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rest {
    /// Equal samples in a row that count as rest.
    pub(crate) samples: u32,
    pub(crate) poll: Duration,
    /// How long a step may take to come to rest before it is recorded as unsettled.
    pub(crate) budget: Duration,
}

impl Default for Rest {
    fn default() -> Self {
        REST
    }
}

pub(crate) const REST: Rest = Rest {
    samples: 8,
    poll: Duration::from_millis(50),
    budget: Duration::from_secs(10),
};

/// How a step ended, as far as two targets can be held to the same word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Ok,
    /// The step's `when` did not hold, so it was not sent.
    Skipped,
    /// The target answered with an error: a target that is not there, a step it cannot run.
    Refused,
    /// A check in the step did not hold.
    Failed,
    /// The target panicked, exited with an error or stopped answering. The stream ends here.
    Crashed,
    /// The target exited as an application does when its window is closed. The stream ends
    /// here, and that is no finding: a step may close the window.
    Closed,
    /// The step took the application to a folder outside the fixture. The stream ends here:
    /// what is listed out there is the machine's, not the sequence's.
    Left,
}

impl Outcome {
    pub(crate) const ALL: [Self; 7] = [
        Self::Ok,
        Self::Skipped,
        Self::Refused,
        Self::Failed,
        Self::Crashed,
        Self::Closed,
        Self::Left,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Skipped => "skipped",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::Crashed => "crashed",
            Self::Closed => "closed",
            Self::Left => "left",
        }
    }

    /// Whether no step is taken after one that ended so.
    pub(crate) fn ends_the_replay(self) -> bool {
        matches!(self, Self::Crashed | Self::Closed | Self::Left)
    }
}

/// One read of everything an observation compares but the step's own outcome.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Sample {
    pub(crate) app: Value,
    pub(crate) fs: Value,
    pub(crate) settings: Value,
}

/// A sample taken at rest, or the last one when the budget ran out first.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rested {
    pub(crate) sample: Sample,
    pub(crate) settled: bool,
    pub(crate) polls: u32,
    pub(crate) waited: Duration,
    /// When, in milliseconds into the wait, a sample differed from the one before it. What
    /// calibration reads to see how long the application goes on changing after a step.
    pub(crate) changes: Vec<u64>,
}

pub(crate) fn milliseconds(time: Duration) -> u64 {
    u64::try_from(time.as_millis()).unwrap_or(u64::MAX)
}

/// Samples until `rest.samples` in a row are equal. An error from `sample` is the target
/// going away, and ends the wait with what was last read.
pub(crate) fn at_rest(
    sample: &mut dyn FnMut() -> Result<Sample, String>,
    rest: &Rest,
) -> (Rested, Option<String>) {
    let started = Instant::now();
    let mut last = Sample::default();
    let mut equal = 0;
    let mut polls = 0;
    let mut changes = Vec::new();
    let rested = |sample, settled, polls, changes| Rested {
        sample,
        settled,
        polls,
        waited: started.elapsed(),
        changes,
    };
    loop {
        let taken = match sample() {
            Ok(taken) => taken,
            Err(error) => return (rested(last, false, polls, changes), Some(error)),
        };
        polls += 1;
        if polls > 1 && taken == last {
            equal += 1;
        } else {
            equal = 1;
            if polls > 1 {
                changes.push(milliseconds(started.elapsed()));
            }
        }
        last = taken;
        if equal >= rest.samples {
            return (rested(last, true, polls, changes), None);
        }
        if started.elapsed() >= rest.budget {
            return (rested(last, false, polls, changes), None);
        }
        std::thread::sleep(rest.poll);
    }
}

/// What went wrong at a step whether or not the replays agree on it: what GLib logged since
/// the observation before, and the invariants that failed at rest (`docs/testing.md`,
/// "Invariants"), which only a target that checks them reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Faults {
    pub(crate) criticals: Vec<String>,
    pub(crate) invariants: Vec<String>,
}

/// One step's observation: what the comparator reads is `value`, and the rest says how it
/// was come by.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Observation {
    /// The step's place in the sequence, from 1; 0 is the launch.
    pub(crate) step: usize,
    /// The step as it was sent, or null for the launch.
    pub(crate) action: Value,
    /// `result`, `criticals`, `invariants`, `app`, `fs` and `settings`.
    pub(crate) value: Value,
    pub(crate) settled: bool,
    pub(crate) polls: u32,
    pub(crate) waited_ms: u64,
    /// `Rested::changes`.
    pub(crate) changes: Vec<u64>,
}

impl Observation {
    pub(crate) fn new(
        (step, action): (usize, Value),
        outcome: Outcome,
        faults: Faults,
        rested: Rested,
    ) -> Self {
        Self {
            step,
            action,
            value: json!({
                RESULT: outcome.name(),
                CRITICALS: faults.criticals,
                INVARIANTS: faults.invariants,
                APP: rested.sample.app,
                FS: rested.sample.fs,
                SETTINGS: rested.sample.settings,
            }),
            settled: rested.settled,
            polls: rested.polls,
            waited_ms: milliseconds(rested.waited),
            changes: rested.changes,
        }
    }

    pub(crate) fn outcome(&self) -> &str {
        self.value[RESULT].as_str().unwrap_or_default()
    }

    pub(crate) fn ends_the_replay(&self) -> bool {
        Outcome::ALL
            .into_iter()
            .any(|outcome| outcome.name() == self.outcome() && outcome.ends_the_replay())
    }

    pub(crate) fn criticals(&self) -> usize {
        self.value[CRITICALS].as_array().map_or(0, Vec::len)
    }

    /// The invariants that failed at this step.
    pub(crate) fn broken(&self) -> usize {
        self.value[INVARIANTS].as_array().map_or(0, Vec::len)
    }

    /// One line of a stream file. The digests are what the plan calls the observation's
    /// digest of the tree and of the settings; the listings beside them say what differs.
    pub(crate) fn line(&self) -> Value {
        json!({
            "step": self.step,
            "action": self.action,
            "settled": self.settled,
            "polls": self.polls,
            "waited_ms": self.waited_ms,
            "changed_ms": self.changes,
            "fs_digest": digest(&self.value[FS]),
            "settings_digest": digest(&self.value[SETTINGS]),
            "observed": self.value,
        })
    }

    pub(crate) fn from_line(line: &Value) -> Option<Self> {
        Some(Self {
            step: usize::try_from(line["step"].as_u64()?).ok()?,
            action: line["action"].clone(),
            value: line.get("observed")?.clone(),
            settled: line["settled"].as_bool()?,
            polls: u32::try_from(line["polls"].as_u64()?).ok()?,
            waited_ms: line["waited_ms"].as_u64()?,
            changes: line["changed_ms"]
                .as_array()?
                .iter()
                .filter_map(Value::as_u64)
                .collect(),
        })
    }
}

/// The first sixteen hex digits of the SHA-256 of a value's JSON. Keys are written in name
/// order, so equal values have equal digests.
pub(crate) fn digest(value: &Value) -> String {
    hex(&Sha256::digest(canonical(value).as_bytes()))
}

/// JSON with every object's keys in name order, whatever order the map keeps them in.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort_unstable();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|key| format!("{}:{}", Value::String(key.clone()), canonical(&object[key])))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(16);
    for byte in bytes.iter().take(8) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The fixture tree as `path: what it is`. A directory is `dir <mode>`, a file
/// `file <mode> <size> <content digest>` and a link `link <target>`. Times are left out:
/// they follow the clock, not the sequence.
pub(crate) fn tree(root: &Path) -> Value {
    let mut entries = Map::new();
    list(root, root, &mut entries);
    Value::Object(entries)
}

fn list(root: &Path, directory: &Path, entries: &mut Map<String, Value>) {
    let Ok(read) = fs::read_dir(directory) else {
        return;
    };
    let mut paths: Vec<_> = read.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let mode = metadata.permissions().mode() & 0o7777;
        if metadata.is_symlink() {
            let target = fs::read_link(&path).unwrap_or_default();
            entries.insert(relative, json!(format!("link {}", target.display())));
        } else if metadata.is_dir() {
            entries.insert(relative, json!(format!("dir {mode:o}")));
            list(root, &path, entries);
        } else {
            let content = fs::read(&path).map_or_else(
                |_| "unreadable".to_owned(),
                |bytes| hex(&Sha256::digest(&bytes)),
            );
            entries.insert(
                relative,
                json!(format!("file {mode:o} {} {content}", metadata.len())),
            );
        }
    }
}

/// `settings.toml` as the values it holds, so two files that say the same in another order
/// are equal. Null when there is none; a file that does not parse is its own text.
pub(crate) fn settings(path: &Path) -> Value {
    let Ok(text) = fs::read_to_string(path) else {
        return Value::Null;
    };
    toml::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "unreadable": text }))
}
