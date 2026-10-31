// SPDX-License-Identifier: MIT

use std::path::Path;

use serde_json::{Value, json};

use crate::{
    NOTED_PLACEHOLDER, RUNNER_COMMANDS, STEP_IF, STEP_MODES, STEP_UNLESS, STEP_VARIANTS, STEP_WHEN,
    fixture::manifest,
    fs_check::FsStep,
    fs_helpers::percent_decoded,
    peer_steps::{
        ForeignLabelStep, ForeignStep, HttpRequestsStep, HttpStep, LaunchStep, PortalOpenStep,
        PortalResponseStep, Relaunch, ZipCryptoStep,
    },
    protocol,
    runner_steps::{EachStep, LogStep, ProcStep},
    scenario::Scenario,
    variables::{Run, substitute, substitute_text},
};

pub(crate) fn run_steps(
    scenario: &Scenario,
    run: &Run,
    variables: &[(String, String)],
) -> Result<Vec<(usize, Value)>, String> {
    let mut steps = Vec::new();
    let mut notes: Vec<(String, Value)> = Vec::new();
    let mut flags: Vec<String> = Vec::new();
    for (line, step) in &scenario.steps {
        let mut step = substitute(step, variables);
        let Some(object) = step.as_object_mut() else {
            continue;
        };
        if let Some(modes) = object.remove(STEP_MODES)
            && !modes
                .as_array()
                .is_some_and(|modes| modes.iter().any(|mode| *mode == *run.mode))
        {
            continue;
        }
        if let Some(variants) = object.remove(STEP_VARIANTS)
            && !variants
                .as_array()
                .is_some_and(|variants| variants.iter().any(|id| *id == *run.variant.id))
        {
            continue;
        }
        let checked = substitute_notes(&step, &notes);
        check_step(&checked).map_err(|error| format!("line {line}: {error}"))?;
        let checked = match checked.get("each") {
            Some(source) => {
                let source = source.as_str().unwrap_or_default();
                if !notes
                    .iter()
                    .any(|(note, value)| note == source && value.is_array())
                {
                    return Err(format!(
                        "line {line}: each needs an earlier note of a list, {source:?}"
                    ));
                }
                let each: EachStep = serde_json::from_value(checked.clone())
                    .map_err(|error| format!("line {line}: invalid each step: {error}"))?;
                let mut checked = checked.clone();
                checked["step"] = each.expand(&json!(NOTED_PLACEHOLDER))?;
                checked
            }
            None => checked,
        };
        for key in [STEP_IF, STEP_UNLESS] {
            for flag in flag_names(checked.get(key)) {
                if !flags.iter().any(|kept| kept == flag) {
                    return Err(format!(
                        "line {line}: {key} names {flag:?}, which no earlier expect or flag keeps"
                    ));
                }
            }
        }
        let keeps_flag = checked.get("expect").is_some()
            || checked.get("flag").is_some()
            || checked
                .get("log")
                .is_some_and(|log| log.get("contains").is_some());
        if keeps_flag && let Some(flag) = checked.get("as").and_then(Value::as_str) {
            flags.push(flag.to_owned());
        }
        if let Some(token) = unknown_variable(&checked) {
            return Err(format!(
                "line {line}: ${token} is neither a variable nor an earlier note"
            ));
        }
        if let Some((name, placeholder)) = kept_note(&checked, &notes)? {
            if variables.iter().any(|(variable, _)| *variable == name) {
                return Err(format!("line {line}: the note {name:?} hides a variable"));
            }
            notes.retain(|(existing, _)| *existing != name);
            notes.push((name, placeholder));
        }
        steps.push((*line, step));
    }
    if let Some(line) = scenario.header.known_bug_step
        && !steps.iter().any(|(number, _)| *number == line)
    {
        return Err(format!(
            "known_bug_step {line} does not run in the {} run",
            run.label
        ));
    }
    Ok(steps)
}

fn kept_note(step: &Value, notes: &[(String, Value)]) -> Result<Option<(String, Value)>, String> {
    let name = || step["as"].as_str().unwrap_or_default().to_owned();
    if step.get("note").is_some() {
        let Ok(protocol::Command::Note { value, .. }) = protocol::parse_command(&step.to_string())
        else {
            return Err("invalid note step".to_owned());
        };
        let Ok(protocol::Command::Note { pick, .. }) = protocol::parse_command(&step.to_string())
        else {
            return Err("invalid note step".to_owned());
        };
        let placeholder = if value == protocol::Accessor::ScrollTarget {
            json!(0.0)
        } else if value == protocol::Accessor::EditableFields {
            json!(0)
        } else if value == protocol::Accessor::Number {
            json!(1)
        } else if value.is_list() && pick.is_none() {
            json!([NOTED_PLACEHOLDER])
        } else {
            json!(NOTED_PLACEHOLDER)
        };
        return Ok(Some((name(), placeholder)));
    }
    if let Some(log) = step.get("log")
        && step.get("as").is_some()
        && log.get("contains").is_none()
    {
        let placeholder = if log.get("matches").is_some() && log.get("pick").is_none() {
            json!([1])
        } else {
            json!(1)
        };
        return Ok(Some((name(), placeholder)));
    }
    if step.get("http").is_some() || step.get("zipcrypto").is_some() {
        return Ok(Some((name(), json!(NOTED_PLACEHOLDER))));
    }
    if let Some(kept) = step
        .get("portal_response")
        .and_then(|check| check.get("keep"))
        .and_then(Value::as_str)
    {
        return Ok(Some((kept.to_owned(), json!(NOTED_PLACEHOLDER))));
    }
    for key in ["stem", "parent"] {
        if let Some(source) = step.get(key) {
            let source = source.as_str().unwrap_or_default();
            if !notes
                .iter()
                .any(|(note, value)| note == source && value.is_string())
            {
                return Err(format!("{key} needs an earlier note of a name, {source:?}"));
            }
            return Ok(Some((name(), json!(NOTED_PLACEHOLDER))));
        }
    }
    if let Some(source) = step.get("increment") {
        let source = source.as_str().unwrap_or_default();
        if !notes
            .iter()
            .any(|(note, value)| note == source && value.is_string())
        {
            return Err(format!(
                "increment needs an earlier note of a name, {source:?}"
            ));
        }
        return Ok(Some((name(), json!(NOTED_PLACEHOLDER))));
    }
    if let Some(source) = step.get("slice") {
        let source = source.as_str().unwrap_or_default();
        if !notes
            .iter()
            .any(|(note, value)| note == source && value.is_array())
        {
            return Err(format!("slice needs an earlier note of a list, {source:?}"));
        }
        let placeholder = if step.get("pick").is_some() {
            json!(NOTED_PLACEHOLDER)
        } else {
            json!([NOTED_PLACEHOLDER])
        };
        return Ok(Some((name(), placeholder)));
    }
    Ok(None)
}

pub(crate) fn slice(
    names: &[Value],
    middle: Option<u64>,
    pick: Option<i64>,
) -> Result<Value, String> {
    let mut names = names.to_vec();
    if let Some(count) = middle.map(|count| usize::try_from(count).unwrap_or(usize::MAX)) {
        let start = names.len() / 2;
        names = names[start..start.saturating_add(count).min(names.len())].to_vec();
    }
    let Some(pick) = pick else {
        return Ok(Value::Array(names));
    };
    let length = i64::try_from(names.len()).map_err(|error| error.to_string())?;
    let index = if pick < 0 { length + pick } else { pick };
    usize::try_from(index)
        .ok()
        .and_then(|index| names.get(index).cloned())
        .ok_or_else(|| format!("index {pick} is outside {names:?}"))
}

pub(crate) fn unknown_variable(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let mut rest = text.as_str();
            while let Some(start) = rest.find('$') {
                let after = &rest[start + 1..];
                if let Some(escaped) = after.strip_prefix('$') {
                    rest = escaped;
                    continue;
                }
                let token: String = after
                    .chars()
                    .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .collect();
                if !token.is_empty() {
                    return Some(token);
                }
                rest = after;
            }
            None
        }
        Value::Array(items) => items.iter().find_map(unknown_variable),
        Value::Object(object) => object.values().find_map(unknown_variable),
        _ => None,
    }
}

pub(crate) fn substitute_notes(value: &Value, notes: &[(String, Value)]) -> Value {
    if notes.is_empty() {
        return value.clone();
    }
    match value {
        Value::String(text) => {
            if let Some((_, noted)) = text
                .strip_prefix('$')
                .and_then(|name| notes.iter().find(|(note, _)| note == name))
            {
                return noted.clone();
            }
            let texts: Vec<(String, String)> = notes
                .iter()
                .map(|(name, noted)| (name.clone(), python_text(noted)))
                .collect();
            Value::String(substitute_text(text, &texts))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute_notes(item, notes))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, item)| (key.clone(), substitute_notes(item, notes)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn python_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".to_owned(),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(|item| match item {
                    Value::String(text) => format!("'{text}'"),
                    other => python_text(other),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

pub(crate) fn increment(text: &str) -> Result<String, String> {
    let digits = text.chars().rev().take_while(char::is_ascii_digit).count();
    let (prefix, number) = text.split_at(text.len() - digits);
    let value: u64 = number
        .parse()
        .map_err(|_| format!("{text:?} does not end in a number"))?;
    Ok(format!("{prefix}{:0width$}", value + 1, width = digits))
}

pub(crate) fn parent(text: &str) -> Result<String, String> {
    Path::new(text)
        .parent()
        .map(|parent| parent.display().to_string())
        .ok_or_else(|| format!("{text:?} has no parent"))
}

pub(crate) fn stem(text: &str) -> Result<String, String> {
    Path::new(text)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .ok_or_else(|| format!("{text:?} has no stem"))
}

pub(crate) fn check_step(step: &Value) -> Result<(), String> {
    for key in ["increment", "stem", "parent"] {
        if step.get(key).is_some() {
            let object = step.as_object().ok_or("a step must be an object")?;
            let valid = object.len() == 2
                && step[key].is_string()
                && step["as"].as_str().is_some_and(|name| !name.is_empty());
            return if valid {
                Ok(())
            } else {
                Err(format!(
                    "{key} names a note and the name to keep the result as"
                ))
            };
        }
    }
    if step.get("holds").is_some() {
        return Err("holds is the runner's own check; a step uses when".to_owned());
    }
    for key in [STEP_IF, STEP_UNLESS] {
        if let Some(flag) = step.get(key) {
            let named = match flag {
                Value::Array(flags) => {
                    !flags.is_empty()
                        && flags
                            .iter()
                            .all(|flag| flag.as_str().is_some_and(|flag| !flag.is_empty()))
                }
                flag => flag.as_str().is_some_and(|flag| !flag.is_empty()),
            };
            if !named {
                return Err(format!(
                    "{key} names what an earlier expect or flag kept, or a list of such names"
                ));
            }
            if step.get("as").is_some()
                && step
                    .get("log")
                    .is_some_and(|log| log.get("contains").is_some())
            {
                return Err(format!(
                    "a log that keeps a flag always runs; it takes no {key}"
                ));
            }
            let mut step = step.clone();
            if let Some(object) = step.as_object_mut() {
                object.remove(key);
            }
            return check_step(&step);
        }
    }
    if let Some(condition) = step.get("flag") {
        let valid = step.as_object().is_some_and(|object| {
            object
                .keys()
                .all(|key| matches!(key.as_str(), "flag" | "as" | "timeout_ms"))
        }) && step["as"].as_str().is_some_and(|name| !name.is_empty())
            && step.get("timeout_ms").is_none_or(Value::is_u64);
        if !valid {
            return Err(
                "flag takes a predicate, the name to keep whether it held as, and an optional \
                 timeout_ms to wait for it"
                    .to_owned(),
            );
        }
        return protocol::parse_command(&json!({ "holds": condition }).to_string())
            .map(|_| ())
            .map_err(|error| format!("flag: {error}"));
    }
    if step.get("slice").is_some() {
        let object = step.as_object().ok_or("a step must be an object")?;
        let valid = step["slice"].is_string()
            && step["as"].as_str().is_some_and(|name| !name.is_empty())
            && object
                .keys()
                .all(|key| matches!(key.as_str(), "slice" | "as" | "middle" | "pick"))
            && object.get("middle").is_none_or(Value::is_u64)
            && object.get("pick").is_none_or(Value::is_i64)
            && (object.contains_key("middle") || object.contains_key("pick"));
        return if valid {
            Ok(())
        } else {
            Err(
                "slice names a list note, the name to keep the result as, and middle or pick"
                    .to_owned(),
            )
        };
    }
    if let Some(name) = step.get("screenshot") {
        let valid = step.as_object().is_some_and(|object| object.len() == 1)
            && name.as_str().is_some_and(|name| {
                !name.is_empty() && !name.contains('/') && name.ends_with(".png")
            });
        return if valid {
            Ok(())
        } else {
            Err(
                "screenshot names a .png file in the run's artifacts and takes nothing else"
                    .to_owned(),
            )
        };
    }
    if let Some(name) = step.get("baseline") {
        let valid = step.as_object().is_some_and(|object| {
            object
                .keys()
                .all(|key| key == "baseline" || (key == "message" && object[key].is_string()))
        }) && name
            .as_str()
            .is_some_and(|name| !name.is_empty() && !name.contains('/') && !name.contains('.'));
        return if valid {
            Ok(())
        } else {
            Err(
                "baseline names a committed image in tests/qa/baselines without its .png, and takes only a message"
                    .to_owned(),
            )
        };
    }
    let keeps_flag = step.get("expect").is_some()
        || step
            .get("log")
            .is_some_and(|log| log.get("contains").is_some());
    if keeps_flag && step.get("as").is_some() && step.get(STEP_WHEN).is_some() {
        return Err("an expect or log that keeps a flag in as may not have a when".to_owned());
    }
    if step.get("expect").is_some()
        && step
            .get("as")
            .is_some_and(|name| !name.as_str().is_some_and(|name| !name.is_empty()))
    {
        return Err("an expect keeps whether it held under a name in as".to_owned());
    }
    if let Some(condition) = step.get(STEP_WHEN) {
        if ["note", "increment", "slice", "stem"]
            .iter()
            .any(|key| step.get(*key).is_some())
            || (step.get("log").is_some() && step.get("as").is_some())
        {
            return Err(
                "a note, increment, slice, or stem always runs; it takes no when".to_owned(),
            );
        }
        protocol::parse_command(&json!({ "holds": condition }).to_string())
            .map_err(|error| format!("when: {error}"))?;
        let mut step = step.clone();
        if let Some(object) = step.as_object_mut() {
            object.remove(STEP_WHEN);
        }
        return check_step(&step);
    }
    if let Some(layout) = step.get("layout") {
        let name = layout.as_str().ok_or("layout names a manifest layout")?;
        if step.as_object().is_some_and(|object| object.len() != 1) {
            return Err("layout takes no other fields".to_owned());
        }
        return if manifest().layouts.contains_key(name) {
            Ok(())
        } else {
            Err(format!("no layout {name:?} in the manifest"))
        };
    }
    if step.get("each").is_some() {
        let object = step.as_object().ok_or("a step must be an object")?;
        let each: EachStep = serde_json::from_value(Value::Object(object.clone()))
            .map_err(|error| format!("invalid each step: {error}"))?;
        if each.name.is_empty() || each.derive.keys().any(|key| key.is_empty()) {
            return Err("each keeps each item under a name in as".to_owned());
        }
        let inner = each.expand(&json!(NOTED_PLACEHOLDER))?;
        if RUNNER_COMMANDS
            .iter()
            .any(|command| inner.get(*command).is_some())
        {
            return Err("each repeats a step the process runs".to_owned());
        }
        return check_step(&inner);
    }
    if let Some(restart) = step.get("restart") {
        if step.as_object().is_some_and(|object| object.len() != 1) {
            return Err("restart takes nothing else".to_owned());
        }
        if *restart == json!(true) {
            return Ok(());
        }
        let relaunch: Relaunch = serde_json::from_value(restart.clone())
            .map_err(|error| format!("invalid restart: {error}"))?;
        return relaunch.check();
    }
    if step.get("launch").is_some() {
        let parsed: LaunchStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("invalid launch step: {error}"))?;
        return if parsed.launch.is_empty() {
            Err("launch needs the arguments strata gets".to_owned())
        } else {
            parsed
                .launch
                .iter()
                .filter(|_| parsed.percent_path)
                .try_for_each(|argument| percent_decoded(argument).map(|_| ()))
        };
    }
    if step.get("portal_open").is_some() {
        let parsed: PortalOpenStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("invalid portal_open step: {error}"))?;
        return if parsed.portal_open.is_empty() {
            Err("portal_open needs the request's title".to_owned())
        } else {
            Ok(())
        };
    }
    if step.get("portal_response").is_some() {
        let parsed: PortalResponseStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("invalid portal_response step: {error}"))?;
        let check = &parsed.portal_response;
        let answers = check.status.is_some()
            || check.names.is_some()
            || check.paths.is_some()
            || check.keep.is_some()
            || check.png.is_some()
            || check.discard;
        return if !check.done && (answers || parsed.timeout_ms.is_some()) {
            Err(
                "a portal_response that is not done checks nothing else and does not wait"
                    .to_owned(),
            )
        } else {
            Ok(())
        };
    }
    if step.get("http").is_some() {
        let parsed: HttpStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("invalid http step: {error}"))?;
        return if parsed.name.is_empty() {
            Err("http keeps the URL under a name in as".to_owned())
        } else {
            Ok(())
        };
    }
    if step.get("http_requests").is_some() {
        return serde_json::from_value::<HttpRequestsStep>(step.clone())
            .map(|_| ())
            .map_err(|error| format!("invalid http_requests step: {error}"));
    }
    if step.get("foreign").is_some() {
        let parsed: ForeignStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("invalid foreign step: {error}"))?;
        return if parsed.name.is_empty() || parsed.foreign.paths.is_empty() {
            Err("foreign needs paths and a name in as for its button's box".to_owned())
        } else {
            Ok(())
        };
    }
    if step.get("foreign_label").is_some() {
        return serde_json::from_value::<ForeignLabelStep>(step.clone())
            .map(|_| ())
            .map_err(|error| format!("invalid foreign_label step: {error}"));
    }
    if step.get("foreign_stop").is_some() {
        return if *step == json!({ "foreign_stop": true }) {
            Ok(())
        } else {
            Err("foreign_stop is {\"foreign_stop\": true}".to_owned())
        };
    }
    if step.get("zipcrypto").is_some() {
        let parsed: ZipCryptoStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("invalid zipcrypto step: {error}"))?;
        return if parsed.name.is_empty() {
            Err("zipcrypto keeps the colliding password under a name in as".to_owned())
        } else {
            Ok(())
        };
    }
    if let Some(message) = step.get("fail") {
        return if step.as_object().is_some_and(|object| object.len() == 1)
            && message.as_str().is_some_and(|message| !message.is_empty())
        {
            Ok(())
        } else {
            Err("fail takes an assertion message and nothing else".to_owned())
        };
    }
    if let Some(arguments) = step.get("dbus_send") {
        let valid = step.as_object().is_some_and(|object| object.len() == 1)
            && arguments.as_array().is_some_and(|arguments| {
                !arguments.is_empty() && arguments.iter().all(Value::is_string)
            });
        return if valid {
            Ok(())
        } else {
            Err("dbus_send takes the arguments after dbus-send --session".to_owned())
        };
    }
    if step.get("log").is_some() {
        let parsed: LogStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("invalid log step: {error}"))?;
        return parsed.check();
    }
    if step.get("proc").is_some() {
        let parsed: ProcStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("invalid proc step: {error}"))?;
        return parsed.check();
    }
    if step.get("fs").is_some() {
        let step: FsStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("invalid fs step: {error}"))?;
        if step.fs.acts() && step.timeout_ms.is_some() {
            return Err(
                "an fs write or remember happens at once; it takes no timeout_ms".to_owned(),
            );
        }
        return step.fs.kind().map(|_| ());
    }
    protocol::parse_command(&step.to_string()).map(|_| ())
}

pub(crate) fn flag_names(value: Option<&Value>) -> Vec<&str> {
    match value {
        Some(Value::Array(flags)) => flags.iter().filter_map(Value::as_str).collect(),
        Some(flag) => flag.as_str().into_iter().collect(),
        None => Vec::new(),
    }
}
