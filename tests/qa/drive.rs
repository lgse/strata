// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    path::Path,
    time::{Duration, Instant, SystemTime},
};

use serde_json::{Map, Value, json};

use crate::{
    Application, COMMAND_TIMEOUT, CONTROLLER_PANIC, DEFAULT_LAYOUT, DEFAULT_WAIT, Display,
    LAUNCH_TIMEOUT, Launch, POLL_INTERVAL, STEP_IF, STEP_UNLESS, STEP_WHEN, WAIT_MARGIN,
    baseline::compare_to_baseline,
    fixture::Manifest,
    fs_check::{FsState, FsStep},
    keyboard::switch,
    peer_steps::{PeerError, Peers, Relaunch, dbus_send, peer_step},
    run::Outcome,
    runner_steps::{EachStep, Kept, LogStep, ProcStep},
    steps::{flag_names, increment, parent, slice, stem, substitute_notes},
    variables::unescape,
};

pub(crate) struct Context<'a> {
    pub(crate) steps: &'a [(usize, Value)],
    pub(crate) initial_layout: &'a str,
    pub(crate) manifest: &'a Manifest,
    pub(crate) display: &'a Display,
    pub(crate) fixture: &'a Path,
    pub(crate) fixture_time: SystemTime,
    pub(crate) screens: &'a Path,
    pub(crate) artifacts: &'a Path,
    pub(crate) launch: &'a Launch<'a>,
}

pub(crate) fn drive(
    application: &mut Application,
    peers: &mut Peers,
    context: &Context,
    layout: &mut String,
    fs_state: &mut FsState,
) -> Outcome {
    let mut outcome = Outcome::default();
    let hello = application.receive(LAUNCH_TIMEOUT);
    if let Err(error) = hello.and_then(|hello| {
        record(
            &mut outcome,
            None,
            "launch",
            &json!({ "launch": true }),
            hello,
        )
    }) {
        outcome.errors.push(format!("launch: {error}"));
        return outcome;
    }
    // Only now: the process read the US keymap at startup, and key names
    // keep meaning US key positions under any layout.
    if context.initial_layout != DEFAULT_LAYOUT {
        match switch(context.manifest, context.display, context.initial_layout) {
            Ok(()) => context.initial_layout.clone_into(layout),
            Err(error) => {
                outcome.errors.push(format!("layout: {error}"));
                return outcome;
            }
        }
    }
    let mut notes: Vec<(String, Value)> = Vec::new();
    let mut flags: BTreeMap<String, bool> = BTreeMap::new();
    for (line, step) in context.steps {
        let step = &unescape(&substitute_notes(step, &notes));
        let label = format!("line {line} {step}");
        let mut object = step.as_object().cloned().unwrap_or_default();
        let condition = |key: &str, object: &mut Map<String, Value>, held: bool| {
            let names = object.remove(key);
            flag_names(names.as_ref())
                .into_iter()
                .any(|flag| flags.get(flag).copied() == Some(held))
        };
        if condition(STEP_IF, &mut object, false) | condition(STEP_UNLESS, &mut object, true) {
            if let Some(name) = object
                .get("expect")
                .or_else(|| object.get("flag"))
                .and(object.get("as"))
            {
                flags.insert(name.as_str().unwrap_or_default().to_owned(), false);
            }
            continue;
        }
        if let Some(condition) = object.remove(STEP_WHEN) {
            let probe = json!({ "holds": condition });
            match application.send(&probe, COMMAND_TIMEOUT) {
                Ok(reply) => {
                    let holds = reply["result"] == json!(true);
                    if let Err(error) = record(&mut outcome, Some(*line), &label, &probe, reply) {
                        file_failure(&mut outcome, *line, &label, &error);
                        return outcome;
                    }
                    if !holds {
                        continue;
                    }
                }
                Err(error) => {
                    outcome.errors.push(format!("{label}: {error}"));
                    return outcome;
                }
            }
        }
        let step = &Value::Object(object.clone());
        if let Some(name) = object.get("layout") {
            let name = name.as_str().unwrap_or_default().to_owned();
            match switch(context.manifest, context.display, &name) {
                Ok(()) => *layout = name,
                Err(error) => {
                    outcome.errors.push(format!("{label}: {error}"));
                    return outcome;
                }
            }
            continue;
        }
        if let Some(condition) = object.get("flag") {
            // Polls up to its timeout and never fails.
            let probe = json!({ "holds": condition });
            let deadline = Instant::now()
                + object
                    .get("timeout_ms")
                    .and_then(Value::as_u64)
                    .map_or(Duration::ZERO, Duration::from_millis);
            let holds = loop {
                match application.send(&probe, COMMAND_TIMEOUT) {
                    Ok(reply) => {
                        let holds = reply["result"] == json!(true);
                        if let Err(error) = record(&mut outcome, Some(*line), &label, &probe, reply)
                        {
                            file_failure(&mut outcome, *line, &label, &error);
                            return outcome;
                        }
                        if holds || Instant::now() >= deadline {
                            break holds;
                        }
                        std::thread::sleep(POLL_INTERVAL);
                    }
                    Err(error) => {
                        outcome.errors.push(format!("{label}: {error}"));
                        return outcome;
                    }
                }
            };
            flags.insert(object["as"].as_str().unwrap_or_default().to_owned(), holds);
            continue;
        }
        if let Some(name) = object.get("screenshot") {
            let destination = context.screens.join(name.as_str().unwrap_or_default());
            let taken = fs::create_dir_all(context.screens)
                .map_err(|error| error.to_string())
                .and_then(|()| context.display.screenshot(&destination));
            if let Err(error) = taken {
                outcome.errors.push(format!("{label}: {error}"));
                return outcome;
            }
            continue;
        }
        if let Some(name) = object.get("baseline") {
            let name = name.as_str().unwrap_or_default();
            let capture = context.screens.join(format!("{name}.capture.png"));
            let compared = fs::create_dir_all(context.screens)
                .map_err(|error| error.to_string())
                .and_then(|()| context.display.screenshot(&capture))
                .and_then(|()| compare_to_baseline(name, &capture, context.screens));
            match compared {
                Ok(comparison) if comparison.matched => {}
                Ok(comparison) => {
                    let failure = comparison.failure(context.artifacts);
                    file_failure(&mut outcome, *line, &label, &failure);
                    return outcome;
                }
                Err(error) => {
                    outcome.errors.push(format!("{label}: {error}"));
                    return outcome;
                }
            }
            continue;
        }
        if let Some(source) = object.get("each") {
            let each: EachStep = match serde_json::from_value(step.clone()) {
                Ok(each) => each,
                Err(error) => {
                    outcome
                        .errors
                        .push(format!("{label}: invalid each step: {error}"));
                    return outcome;
                }
            };
            let items = notes
                .iter()
                .find(|(note, _)| Some(note.as_str()) == source.as_str())
                .and_then(|(_, value)| value.as_array().cloned())
                .unwrap_or_default();
            for item in items {
                let inner = match each.expand(&item) {
                    Ok(inner) => unescape(&inner),
                    Err(error) => {
                        file_failure(&mut outcome, *line, &label, &error);
                        return outcome;
                    }
                };
                let timeout = match inner.get("timeout_ms").and_then(Value::as_u64) {
                    Some(ms) => (Duration::from_millis(ms) + WAIT_MARGIN).max(COMMAND_TIMEOUT),
                    None => COMMAND_TIMEOUT,
                };
                let inner_label = format!("line {line} {inner}");
                match application.send(&inner, timeout) {
                    Ok(reply) => {
                        if let Err(error) =
                            record(&mut outcome, Some(*line), &inner_label, &inner, reply)
                        {
                            file_failure(&mut outcome, *line, &inner_label, &error);
                            return outcome;
                        }
                    }
                    Err(error) => {
                        outcome.errors.push(format!("{inner_label}: {error}"));
                        return outcome;
                    }
                }
            }
            continue;
        }
        if let Some(restart) = object.get("restart") {
            if *layout != DEFAULT_LAYOUT {
                outcome.errors.push(format!(
                    "{label}: a restart reads the keymap again, so it needs the US layout"
                ));
                return outcome;
            }
            let relaunch: Relaunch = if *restart == json!(true) {
                Relaunch::default()
            } else {
                match serde_json::from_value(restart.clone()) {
                    Ok(relaunch) => relaunch,
                    Err(error) => {
                        outcome
                            .errors
                            .push(format!("{label}: invalid restart: {error}"));
                        return outcome;
                    }
                }
            };
            application.terminate_group();
            let started = Application::start(context.launch, &relaunch).and_then(|started| {
                *application = started;
                application.receive(LAUNCH_TIMEOUT)
            });
            if let Err(error) = started.and_then(|hello| {
                record(
                    &mut outcome,
                    Some(*line),
                    &label,
                    &json!({ "launch": true }),
                    hello,
                )
            }) {
                outcome.errors.push(format!("{label}: {error}"));
                return outcome;
            }
            continue;
        }
        if let Some(result) = peer_step(application, peers, context, &object, &mut notes) {
            match result {
                Ok(Some((request, reply))) => {
                    if let Err(error) = record(&mut outcome, Some(*line), &label, &request, reply) {
                        file_failure(&mut outcome, *line, &label, &error);
                        return outcome;
                    }
                }
                Ok(None) => {}
                Err(PeerError::Failed(error)) => {
                    file_failure(&mut outcome, *line, &label, &error);
                    return outcome;
                }
                Err(PeerError::Harness(error)) => {
                    outcome.errors.push(format!("{label}: {error}"));
                    return outcome;
                }
            }
            continue;
        }
        if let Some(arguments) = object.get("dbus_send") {
            let arguments: Vec<String> = arguments
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|argument| argument.as_str().map(str::to_owned))
                .collect();
            if let Err(error) = dbus_send(context.launch, &arguments) {
                file_failure(&mut outcome, *line, &label, &error);
                return outcome;
            }
            continue;
        }
        if object.contains_key("log") {
            let parsed: LogStep = match serde_json::from_value(step.clone()) {
                Ok(parsed) => parsed,
                Err(error) => {
                    outcome
                        .errors
                        .push(format!("{label}: invalid log step: {error}"));
                    return outcome;
                }
            };
            match parsed.run(&application.log) {
                Ok(Kept::Nothing) => {}
                Ok(Kept::Flag(name, holds)) => {
                    flags.insert(name, holds);
                }
                Ok(Kept::Note(name, value)) => {
                    notes.retain(|(note, _)| *note != name);
                    notes.push((name, value));
                }
                Err(error) => {
                    file_failure(&mut outcome, *line, &label, &error);
                    return outcome;
                }
            }
            continue;
        }
        if object.contains_key("proc") {
            let parsed: ProcStep = match serde_json::from_value(step.clone()) {
                Ok(parsed) => parsed,
                Err(error) => {
                    outcome
                        .errors
                        .push(format!("{label}: invalid proc step: {error}"));
                    return outcome;
                }
            };
            if let Err(error) = application.pid().and_then(|pid| parsed.run(pid)) {
                file_failure(&mut outcome, *line, &label, &error);
                return outcome;
            }
            continue;
        }
        if let Some(source) = object.get("slice") {
            let name = object["as"].as_str().unwrap_or_default().to_owned();
            let sliced = notes
                .iter()
                .find(|(note, _)| Some(note.as_str()) == source.as_str())
                .and_then(|(_, value)| value.as_array())
                .ok_or_else(|| format!("no note of a list called {source}"))
                .and_then(|names| {
                    slice(
                        names,
                        object.get("middle").and_then(Value::as_u64),
                        object.get("pick").and_then(Value::as_i64),
                    )
                });
            match sliced {
                Ok(value) => {
                    notes.retain(|(note, _)| *note != name);
                    notes.push((name, value));
                    continue;
                }
                Err(error) => {
                    file_failure(&mut outcome, *line, &label, &error);
                    return outcome;
                }
            }
        }
        if let Some((source, derive)) = object
            .get("increment")
            .map(|source| (source, increment as fn(&str) -> Result<String, String>))
            .or_else(|| object.get("stem").map(|source| (source, stem as _)))
            .or_else(|| object.get("parent").map(|source| (source, parent as _)))
        {
            let name = object["as"].as_str().unwrap_or_default().to_owned();
            let noted = notes
                .iter()
                .find(|(note, _)| Some(note.as_str()) == source.as_str())
                .and_then(|(_, value)| value.as_str())
                .ok_or_else(|| format!("no note of a name called {source}"))
                .and_then(derive);
            match noted {
                Ok(text) => {
                    notes.retain(|(note, _)| *note != name);
                    notes.push((name, json!(text)));
                    continue;
                }
                Err(error) => {
                    file_failure(&mut outcome, *line, &label, &error);
                    return outcome;
                }
            }
        }
        let result = if object.contains_key("fs") {
            wait_for_fs(step, context.fixture, context.fixture_time, fs_state)
        } else {
            let timeout = match object.get("timeout_ms").and_then(Value::as_u64) {
                Some(ms) => (Duration::from_millis(ms) + WAIT_MARGIN).max(COMMAND_TIMEOUT),
                None => COMMAND_TIMEOUT,
            };
            match application.send(step, timeout) {
                Ok(reply) => {
                    let result = reply["result"].clone();
                    if let Some(name) = object.get("expect").and(object.get("as")) {
                        flags.insert(
                            name.as_str().unwrap_or_default().to_owned(),
                            reply["ok"] == json!(true),
                        );
                    }
                    let recorded = record(&mut outcome, Some(*line), &label, step, reply);
                    if recorded.is_ok()
                        && let Some(name) = object.get("note").and(object.get("as"))
                    {
                        let name = name.as_str().unwrap_or_default().to_owned();
                        notes.retain(|(note, _)| *note != name);
                        notes.push((name, result));
                    }
                    recorded
                }
                Err(error) => {
                    outcome.errors.push(format!("{label}: {error}"));
                    return outcome;
                }
            }
        };
        if let Err(error) = result {
            file_failure(&mut outcome, *line, &label, &error);
            return outcome;
        }
    }
    outcome
}

/// A controller panic is the harness's error, which no `known_bug` excuses.
pub(crate) fn file_failure(outcome: &mut Outcome, line: usize, label: &str, error: &str) {
    let failure = format!("{label}: {error}");
    if error.contains(CONTROLLER_PANIC) {
        outcome.errors.push(failure);
    } else {
        outcome.failures.push((Some(line), failure));
    }
}

fn record(
    outcome: &mut Outcome,
    line: Option<usize>,
    label: &str,
    step: &Value,
    reply: Value,
) -> Result<(), String> {
    let _ = writeln!(
        outcome.transcript,
        "{}",
        json!({ "command": step, "reply": reply })
    );
    if let Some(tree) = reply["result"]["tree"].as_str() {
        outcome.last_tree = Some(tree.to_owned());
    }
    for critical in reply["criticals"].as_array().into_iter().flatten() {
        outcome.failures.push((
            line,
            format!(
                "{label}: GLib warning or worse: {}",
                critical.as_str().unwrap_or_default()
            ),
        ));
    }
    if reply["ok"].as_bool() == Some(true) {
        return Ok(());
    }
    let failures = reply["failures"].as_array().cloned().unwrap_or_default();
    let explanation = failures
        .iter()
        .map(|failure| {
            format!(
                "{} expected {}, actual {}",
                failure["message"].as_str().unwrap_or("predicate"),
                failure["predicate"],
                failure["actual"]
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    match (reply["error"].as_str(), step.get("expect")) {
        (None, Some(_)) => {
            outcome
                .failures
                .push((line, format!("{label}: {explanation}")));
            Ok(())
        }
        (error, _) => Err(format!(
            "{}{}{explanation}",
            error.unwrap_or_default(),
            if explanation.is_empty() { "" } else { ": " }
        )),
    }
}

pub(crate) fn wait_for_fs(
    step: &Value,
    fixture: &Path,
    fixture_time: SystemTime,
    state: &mut FsState,
) -> Result<(), String> {
    let step: FsStep = serde_json::from_value(step.clone())
        .map_err(|error| format!("invalid fs step: {error}"))?;
    if step.fs.acts() {
        return step
            .fs
            .observe(fixture, fixture_time, state)
            .map_err(|error| format!("{}: {error}", step.fs.path));
    }
    let deadline = Instant::now() + step.timeout_ms.map_or(DEFAULT_WAIT, Duration::from_millis);
    loop {
        match step.fs.observe(fixture, fixture_time, state) {
            Ok(()) => return Ok(()),
            Err(_) if state.fatal.is_some() => {
                return Err(format!(
                    "{} {}",
                    step.fs.path,
                    state.fatal.take().unwrap_or_default()
                ));
            }
            Err(actual) if Instant::now() >= deadline => {
                return Err(format!(
                    "timed out{}; {} {actual}",
                    step.message
                        .as_deref()
                        .map(|message| format!(" waiting for {message}"))
                        .unwrap_or_default(),
                    step.fs.path
                ));
            }
            Err(_) => std::thread::sleep(POLL_INTERVAL),
        }
    }
}
