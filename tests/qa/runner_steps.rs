// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{DEFAULT_WAIT, NOTED_PLACEHOLDER, POLL_INTERVAL, steps::stem, variables::substitute};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EachStep {
    #[serde(rename = "each")]
    _source: String,
    #[serde(rename = "as")]
    pub(crate) name: String,
    step: Value,
    #[serde(default)]
    pub(crate) derive: BTreeMap<String, Derived>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Derived {
    number_plus: i64,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    suffix: String,
}

impl EachStep {
    pub(crate) fn expand(&self, item: &Value) -> Result<Value, String> {
        let text = match item {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        let mut variables = vec![(self.name.clone(), text.clone())];
        for (key, derived) in &self.derive {
            let value = if text == NOTED_PLACEHOLDER {
                NOTED_PLACEHOLDER.to_owned()
            } else {
                let stem = stem(&text)?;
                let digits: String = stem
                    .chars()
                    .rev()
                    .take_while(char::is_ascii_digit)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let number: i64 = digits
                    .parse()
                    .map_err(|_| format!("{text:?} has no number in its stem"))?;
                format!(
                    "{}{}{}",
                    derived.prefix,
                    number + derived.number_plus,
                    derived.suffix
                )
            };
            variables.push((key.clone(), value));
        }
        variables.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
        if !self.step.is_object() {
            return Err("each repeats one step object".to_owned());
        }
        Ok(substitute(&self.step, &variables))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogStep {
    log: LogQuery,
    timeout_ms: Option<u64>,
    message: Option<String>,
    #[serde(rename = "as")]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogQuery {
    count: Option<String>,
    matches: Option<String>,
    contains: Option<String>,
    pick: Option<i64>,
    equals: Option<Value>,
    at_least: Option<Value>,
    at_most: Option<Value>,
    #[serde(default)]
    plus: i64,
}

pub(crate) enum Kept {
    Nothing,
    Flag(String, bool),
    Note(String, Value),
}

impl LogStep {
    pub(crate) fn check(&self) -> Result<(), String> {
        let query = &self.log;
        let kinds = [&query.count, &query.matches, &query.contains]
            .into_iter()
            .filter(|kind| kind.is_some())
            .count();
        if kinds != 1 {
            return Err("log reads exactly one of count, matches, and contains".to_owned());
        }
        if let Some(pattern) = &query.matches {
            regex::Regex::new(pattern).map_err(|error| format!("log matches: {error}"))?;
        }
        let conditions = [&query.equals, &query.at_least, &query.at_most]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if conditions
            .iter()
            .any(|value| !value.is_u64() && !value.is_array())
        {
            return Err("a log bound is a number or a noted list".to_owned());
        }
        match &self.name {
            Some(name) if name.is_empty() => Err("log keeps under a name in as".to_owned()),
            Some(_) => {
                if !conditions.is_empty()
                    || query.plus != 0
                    || self.timeout_ms.is_some()
                    || self.message.is_some()
                {
                    return Err("a log that keeps a value checks nothing".to_owned());
                }
                if query.pick.is_some() && query.matches.is_none() {
                    return Err("pick belongs to matches".to_owned());
                }
                Ok(())
            }
            None => {
                if query.contains.is_some() || query.pick.is_some() {
                    return Err("contains and pick keep a flag or a note under as".to_owned());
                }
                if conditions.is_empty() {
                    return Err("log needs equals, at_least, or at_most".to_owned());
                }
                Ok(())
            }
        }
    }

    fn read(path: &Path) -> String {
        let text = String::from_utf8_lossy(&fs::read(path).unwrap_or_default()).into_owned();
        regex::Regex::new("\x1b\\[[0-9;]*m")
            .map(|colours| colours.replace_all(&text, "").into_owned())
            .unwrap_or(text)
    }

    fn numbers(pattern: &str, text: &str) -> Result<Vec<i64>, String> {
        let pattern = regex::Regex::new(pattern).map_err(|error| error.to_string())?;
        pattern
            .captures_iter(text)
            .map(|found| {
                found
                    .get(1)
                    .unwrap_or_else(|| found.get(0).expect("a match has its whole text"))
                    .as_str()
                    .parse::<i64>()
                    .map_err(|error| format!("log matches a non-number: {error}"))
            })
            .collect()
    }

    pub(crate) fn run(&self, path: &Path) -> Result<Kept, String> {
        self.check()?;
        let query = &self.log;
        if let Some(name) = &self.name {
            let text = Self::read(path);
            return Ok(match (&query.count, &query.matches, &query.contains) {
                (Some(part), _, _) => {
                    Kept::Note(name.clone(), json!(text.matches(part.as_str()).count()))
                }
                (_, Some(pattern), _) => {
                    let numbers = Self::numbers(pattern, &text)?;
                    let value = match query.pick {
                        None => json!(numbers),
                        Some(pick) => {
                            let length =
                                i64::try_from(numbers.len()).map_err(|error| error.to_string())?;
                            let index = if pick < 0 { length + pick } else { pick };
                            json!(
                                usize::try_from(index)
                                    .ok()
                                    .and_then(|index| numbers.get(index))
                                    .ok_or_else(|| format!(
                                        "index {pick} is outside {numbers:?}"
                                    ))?
                            )
                        }
                    };
                    Kept::Note(name.clone(), value)
                }
                (_, _, Some(part)) => Kept::Flag(name.clone(), text.contains(part.as_str())),
                _ => Kept::Nothing,
            });
        }
        let bound = |value: &Option<Value>| -> Option<i64> {
            value.as_ref().map(|value| {
                value.as_array().map_or_else(
                    || value.as_i64().unwrap_or_default(),
                    |items| items.len() as i64,
                ) + query.plus
            })
        };
        let (equals, at_least, at_most) = (
            bound(&query.equals),
            bound(&query.at_least),
            bound(&query.at_most),
        );
        let deadline = Instant::now() + self.timeout_ms.map_or(DEFAULT_WAIT, Duration::from_millis);
        loop {
            let text = Self::read(path);
            let observed = match (&query.count, &query.matches) {
                (Some(part), _) => text.matches(part.as_str()).count() as i64,
                (_, Some(pattern)) => Self::numbers(pattern, &text)?.len() as i64,
                _ => 0,
            };
            if equals.is_none_or(|equals| observed == equals)
                && at_least.is_none_or(|least| observed >= least)
                && at_most.is_none_or(|most| observed <= most)
            {
                return Ok(Kept::Nothing);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{}the log has {observed}, not {}{}{}",
                    self.message
                        .as_deref()
                        .map(|message| format!("timed out waiting for {message}; "))
                        .unwrap_or_default(),
                    equals.map(|n| format!("= {n} ")).unwrap_or_default(),
                    at_least.map(|n| format!(">= {n} ")).unwrap_or_default(),
                    at_most.map(|n| format!("<= {n}")).unwrap_or_default(),
                ));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcStep {
    proc: ProcQuery,
    timeout_ms: Option<u64>,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcQuery {
    gone: Option<Value>,
    parent_is_app: Option<Value>,
    kill: Option<Value>,
    app_cwd: Option<String>,
}

impl ProcStep {
    pub(crate) fn check(&self) -> Result<(), String> {
        let query = &self.proc;
        match (
            &query.gone,
            &query.parent_is_app,
            &query.kill,
            &query.app_cwd,
        ) {
            (Some(pids), None, None, None) if pids.is_array() => Ok(()),
            (None, Some(pid), None, None) if pid.is_u64() => Ok(()),
            (None, None, None, Some(_)) => Ok(()),
            (None, None, Some(pid), None) if pid.is_u64() => {
                if self.timeout_ms.is_some() || self.message.is_some() {
                    Err("a kill happens at once; it takes no timeout_ms or message".to_owned())
                } else {
                    Ok(())
                }
            }
            _ => Err(
                "proc takes one of gone (a noted list of pids), parent_is_app, kill (a pid), \
                 or app_cwd (a path)"
                    .to_owned(),
            ),
        }
    }

    pub(crate) fn run(&self, application: u32) -> Result<(), String> {
        let query = &self.proc;
        let pid = |value: &Value| {
            value
                .as_u64()
                .and_then(|pid| i32::try_from(pid).ok())
                .ok_or_else(|| format!("{value} is not a pid"))
        };
        let parent_is_app = |target: i32| -> Result<(), String> {
            let status = fs::read_to_string(format!("/proc/{target}/status"))
                .map_err(|error| format!("/proc/{target}/status: {error}"))?;
            if status.contains(&format!("PPid:\t{application}\n")) {
                Ok(())
            } else {
                Err(format!("{target} is not a child of {application}"))
            }
        };
        if let Some(target) = &query.kill {
            // Only the application's own children, never anything else.
            let raw = pid(target)?;
            let target = rustix::process::Pid::from_raw(raw)
                .ok_or_else(|| format!("{target} is not a pid"))?;
            // The pidfd pins the process before its parent is checked, so a
            // reused pid can never receive the signal.
            let pidfd = rustix::process::pidfd_open(target, rustix::process::PidfdFlags::empty())
                .map_err(|error| format!("cannot open {target:?}: {error}"))?;
            parent_is_app(raw).map_err(|error| format!("refusing to kill: {error}"))?;
            return rustix::process::pidfd_send_signal(&pidfd, rustix::process::Signal::KILL)
                .map_err(|error| format!("cannot kill {target:?}: {error}"));
        }
        let deadline = Instant::now() + self.timeout_ms.map_or(DEFAULT_WAIT, Duration::from_millis);
        loop {
            let verdict = if let Some(pids) = &query.gone {
                let running: Vec<i32> = pids
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(pid)
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter(|pid| Path::new(&format!("/proc/{pid}")).exists())
                    .collect();
                if running.is_empty() {
                    Ok(())
                } else {
                    Err(format!("{running:?} still run"))
                }
            } else if let Some(expected) = &query.app_cwd {
                let cwd = fs::read_link(format!("/proc/{application}/cwd"))
                    .map_err(|error| format!("/proc/{application}/cwd: {error}"))?;
                if cwd == Path::new(expected) {
                    Ok(())
                } else {
                    Err(format!(
                        "the application's working folder is {}",
                        cwd.display()
                    ))
                }
            } else {
                parent_is_app(pid(query.parent_is_app.as_ref().unwrap_or(&Value::Null))?)
            };
            match verdict {
                Ok(()) => return Ok(()),
                Err(actual) if Instant::now() >= deadline => {
                    return Err(format!(
                        "{}{actual}",
                        self.message
                            .as_deref()
                            .map(|message| format!("timed out waiting for {message}; "))
                            .unwrap_or_default()
                    ));
                }
                Err(_) => std::thread::sleep(POLL_INTERVAL),
            }
        }
    }
}
