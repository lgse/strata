// SPDX-License-Identifier: MIT

use std::time::Duration;

use serde_json::Value;

use super::super::{Chord, DEFAULT_WAIT, Modifier, Predicate, StateName, Target};
use super::{
    Accessor, ActivateFields, AdoptFields, ButtonDownFields, ButtonUpFields, COMMANDS,
    CaptureFields, ClickFields, ClickResultRowsFields, ClipboardFields, ClipboardSetFields,
    Command, DragFields, DropSamplesFields, DuringCheck, DuringFields, ExpectFields, HoldsFields,
    KeepFields, KeyDownFields, KeyFields, KeyUpFields, KeysUntilFields, MeasureTextFields,
    MoveByFields, MoveFields, NoteFields, OpenMenuFields, PressCheck, PressStepFields, QueryFields,
    RememberFields, Repeat, ResizeFields, RevealControlFields, RevealFields, RowStates,
    SelectFields, SelectWithKeyboardFields, SetScrollbarFields, SettleFields, SleepFields,
    TypeFields, WaitFields, WheelDirection, WheelFields,
};

fn modifiers(names: &[String]) -> Result<Vec<Modifier>, String> {
    names.iter().map(|name| Modifier::parse(name)).collect()
}

pub(crate) fn parse_command(line: &str) -> Result<Command, String> {
    let value: Value =
        serde_json::from_str(line).map_err(|error| format!("invalid JSON: {error}"))?;
    let Value::Object(object) = value else {
        return Err("a command must be a JSON object".to_owned());
    };
    let names: Vec<&str> = COMMANDS
        .into_iter()
        .filter(|name| object.contains_key(*name))
        .collect();
    let [name] = names.as_slice() else {
        return Err(format!(
            "a command needs exactly one of {COMMANDS:?}, found {names:?}"
        ));
    };
    let fields = Value::Object(object.clone());
    let invalid = |error: serde_json::Error| format!("invalid {name} command: {error}");
    match *name {
        "key_down" | "key_up" => {
            let key = if *name == "key_down" {
                serde_json::from_value::<KeyDownFields>(fields)
                    .map_err(invalid)?
                    .key_down
            } else {
                serde_json::from_value::<KeyUpFields>(fields)
                    .map_err(invalid)?
                    .key_up
            };
            if key.is_empty() || key.contains('+') && key != "+" {
                return Err(format!("{name} takes one key name, not {key:?}"));
            }
            Ok(if *name == "key_down" {
                Command::KeyDown(key)
            } else {
                Command::KeyUp(key)
            })
        }
        "clipboard_set" => {
            let fields: ClipboardSetFields = serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::ClipboardSet(fields.clipboard_set))
        }
        "clipboard" => {
            let fields: ClipboardFields = serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::Clipboard {
                text: fields.clipboard,
                message: fields.message,
            })
        }
        "keys_until" => {
            let fields: KeysUntilFields = serde_json::from_value(fields).map_err(invalid)?;
            let times = match &fields.times {
                Value::Array(items) => u32::try_from(items.len()).ok(),
                other => other.as_u64().and_then(|times| u32::try_from(times).ok()),
            }
            .ok_or("keys_until times is a number of rounds or a noted list")?;
            if fields.keys.is_empty() || times == 0 {
                return Err("keys_until needs keys and at least one round".to_owned());
            }
            Ok(Command::KeysUntil {
                predicate: fields.keys_until,
                keys: fields
                    .keys
                    .iter()
                    .map(|key| Chord::parse(key))
                    .collect::<Result<_, _>>()?,
                times,
                check_first: fields.check_first,
                await_focus_change: fields
                    .await_focus_change
                    .map(|change| (Duration::from_millis(change.timeout_ms), change.message)),
                exhausted: fields.exhausted,
            })
        }
        "adopt" => {
            let fields: AdoptFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.adopt.is_empty() {
                return Err("adopt needs a window title".to_owned());
            }
            Ok(Command::Adopt {
                title: fields.adopt,
                timeout: fields
                    .timeout_ms
                    .map_or(DEFAULT_WAIT, Duration::from_millis),
                message: fields.message,
            })
        }
        "keep" => {
            let fields: KeepFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.name.is_empty() {
                return Err("keep needs a name in as".to_owned());
            }
            Ok(Command::Keep {
                rect: fields.keep,
                name: fields.name,
            })
        }
        "reveal_control" | "reveal" => {
            let target = if *name == "reveal" {
                serde_json::from_value::<RevealFields>(fields)
                    .map_err(invalid)?
                    .reveal
            } else {
                serde_json::from_value::<RevealControlFields>(fields)
                    .map_err(invalid)?
                    .reveal_control
            };
            let Target::Role {
                name: Some(_),
                within: None,
                ..
            } = &target
            else {
                return Err(format!("{name} takes a named role target without within"));
            };
            Ok(if *name == "reveal" {
                Command::Reveal { target }
            } else {
                Command::RevealControl { target }
            })
        }
        "during" => {
            let fields: DuringFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.ms == 0 || fields.every_ms == 0 {
                return Err("during needs a positive ms and every_ms".to_owned());
            }
            let checks = match fields.during {
                Value::Array(items) => {
                    if items.is_empty() || fields.message.is_some() {
                        return Err(
                            "a during list needs checks, each with its own message".to_owned()
                        );
                    }
                    items
                        .into_iter()
                        .map(|item| {
                            serde_json::from_value::<DuringCheck>(item)
                                .map(|check| (check.check, check.message))
                                .map_err(invalid)
                        })
                        .collect::<Result<_, _>>()?
                }
                one => vec![(
                    serde_json::from_value(one).map_err(invalid)?,
                    fields.message,
                )],
            };
            Ok(Command::During {
                checks,
                duration: Duration::from_millis(fields.ms),
                every: Duration::from_millis(fields.every_ms),
            })
        }
        "capture" => {
            let fields: CaptureFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.name.is_empty() {
                return Err("capture needs a name in as".to_owned());
            }
            Ok(Command::Capture {
                target: fields.capture,
                inset: fields.inset,
                name: fields.name,
            })
        }
        "measure_text" => {
            let fields: MeasureTextFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.name.is_empty() {
                return Err("measure_text needs a name in as".to_owned());
            }
            Ok(Command::MeasureText {
                kept: fields.measure_text,
                name: fields.name,
            })
        }
        "drop_samples" => {
            let fields: DropSamplesFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.name.is_empty() {
                return Err("drop_samples needs a name in as".to_owned());
            }
            Ok(Command::DropSamples {
                kept: fields.drop_samples,
                resting: fields.resting,
                dragging: fields.dragging,
                escape: fields.escape,
                name: fields.name,
            })
        }
        "key" => {
            let fields: KeyFields = serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::Key {
                chord: Chord::parse(&fields.key)?,
                repeat: fields.repeat,
            })
        }
        "type" => {
            let fields: TypeFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.text.is_empty() {
                return Err("type needs text".to_owned());
            }
            Ok(Command::Type(fields.text))
        }
        "select_with_keyboard" => {
            let fields: SelectWithKeyboardFields =
                serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::SelectWithKeyboard {
                name: fields.select_with_keyboard,
                message: fields.message,
            })
        }
        "open_menu" => {
            let fields: OpenMenuFields = serde_json::from_value(fields).map_err(invalid)?;
            fields.open_menu.accepts(None)?;
            let open_states = fields
                .open_states
                .unwrap_or_else(|| vec![StateName::Checked]);
            if open_states.is_empty() {
                return Err("open_states needs a state".to_owned());
            }
            Ok(Command::OpenMenu {
                target: fields.open_menu,
                until: fields.until,
                message: fields.message,
                open_states,
            })
        }
        "activate" => {
            let fields: ActivateFields = serde_json::from_value(fields).map_err(invalid)?;
            if !matches!(
                fields.activate,
                Target::Role { .. } | Target::DialogButton(_)
            ) {
                return Err("activate takes a role target or a dialog_button".to_owned());
            }
            Ok(Command::Activate {
                target: fields.activate,
                message: fields.message,
            })
        }
        "holds" => {
            let fields: HoldsFields = serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::Holds(fields.holds))
        }
        "remember" => {
            let fields: RememberFields = serde_json::from_value(fields).map_err(invalid)?;
            if matches!(fields.remember, Target::Field(Some(_))) {
                return Err(
                    "remember keeps the focused field, {\"field\":true} with an optional \
                     within, or a box"
                        .to_owned(),
                );
            }
            if fields.name.is_empty() {
                return Err("remember needs a name in as".to_owned());
            }
            Ok(Command::Remember {
                target: fields.remember,
                name: fields.name,
            })
        }
        "note" => {
            let fields: NoteFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.name.is_empty() {
                return Err("note needs a name in as".to_owned());
            }
            if fields.dir.is_some() && !fields.note.takes_directory() {
                return Err(format!(
                    "dir does not apply to note {:?}",
                    fields.note.name()
                ));
            }
            if (fields.from.is_some() || fields.through.is_some()) && !fields.note.is_list() {
                return Err("from and through slice a list".to_owned());
            }
            if fields.unless_first.is_some() != (fields.note == Accessor::MiddleVisible) {
                return Err("unless_first belongs to, and is needed by, middle_visible".to_owned());
            }
            if !fields.until_found && (fields.timeout_ms.is_some() || fields.message.is_some()) {
                return Err("timeout_ms and message apply to a note with until_found".to_owned());
            }
            if fields.of.is_some()
                != matches!(fields.note, Accessor::ScrollTarget | Accessor::Bounds)
            {
                return Err("of belongs to, and is needed by, scroll_target and bounds".to_owned());
            }
            if (fields.middle.is_some()
                || !fields.plus.is_empty()
                || fields.sorted
                || fields.pick.is_some())
                && !fields.note.is_list()
            {
                return Err("middle, plus, sorted, and pick take a list".to_owned());
            }
            if fields.expr.is_some() != (fields.note == Accessor::Number) {
                return Err("expr belongs to, and is needed by, number".to_owned());
            }
            if !fields.until.is_empty() && !(fields.note == Accessor::Number && fields.until_found)
            {
                return Err("until bounds a number noted with until_found".to_owned());
            }
            if fields.until_names.is_some() && !(fields.note.is_list() && fields.until_found) {
                return Err("until_names bounds a list noted with until_found".to_owned());
            }
            Ok(Command::Note {
                value: fields.note,
                dir: fields.dir,
                wait: fields.until_found.then(|| {
                    fields
                        .timeout_ms
                        .map_or(DEFAULT_WAIT, Duration::from_millis)
                }),
                message: fields.message,
                from: fields.from,
                through: fields.through,
                unless_first: fields.unless_first,
                of: fields.of,
                middle: fields.middle,
                plus: fields.plus,
                sorted: fields.sorted,
                pick: fields.pick,
                expr: fields.expr,
                until: fields.until,
                until_names: fields.until_names,
            })
        }
        "click" => {
            let fields: ClickFields = serde_json::from_value(fields).map_err(invalid)?;
            if !matches!(fields.button, 1 | 3) || !matches!(fields.count, 1 | 2) {
                return Err("click takes button 1 or 3 and count 1 or 2".to_owned());
            }
            if fields.release_mods_first && (fields.count != 1 || fields.mods.is_empty()) {
                return Err("release_mods_first takes one click with mods".to_owned());
            }
            fields.click.accepts(fields.at.as_ref())?;
            Ok(Command::Click {
                target: fields.click,
                button: fields.button,
                count: fields.count,
                modifiers: modifiers(&fields.mods)?,
                at: fields.at,
                left_bound: fields.left_bound,
                release_modifiers_first: fields.release_mods_first,
            })
        }
        "click_result_rows" => {
            let fields: ClickResultRowsFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.click_result_rows == RowStates::default() {
                return Err("click_result_rows needs a state to match".to_owned());
            }
            Ok(Command::ClickResultRows {
                states: fields.click_result_rows,
                modifiers: modifiers(&fields.mods)?,
            })
        }
        "select" => {
            let fields: SelectFields = serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::Select(fields.select))
        }
        "move" => {
            let fields: MoveFields = serde_json::from_value(fields).map_err(invalid)?;
            fields.target.accepts(fields.at.as_ref())?;
            Ok(Command::Move {
                target: fields.target,
                at: fields.at,
                left_bound: fields.left_bound,
            })
        }
        "move_by" => {
            let fields: MoveByFields = serde_json::from_value(fields).map_err(invalid)?;
            let [x, y] = fields.move_by;
            Ok(Command::MoveBy { x, y })
        }
        "button_down" => {
            let fields: ButtonDownFields = serde_json::from_value(fields).map_err(invalid)?;
            if !matches!(fields.button_down, 1..=3) {
                return Err("button_down takes button 1, 2, or 3".to_owned());
            }
            Ok(Command::ButtonDown(fields.button_down))
        }
        "set_scrollbar" => {
            let fields: SetScrollbarFields = serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::SetScrollbar {
                orientation: fields.set_scrollbar,
                value: fields.value,
            })
        }
        "sleep" => {
            let fields: SleepFields = serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::Sleep(Duration::from_millis(fields.sleep)))
        }
        "wheel" => {
            let fields: WheelFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.clicks == 0 {
                return Err("wheel needs at least one click".to_owned());
            }
            fields.wheel.accepts(fields.at.as_ref())?;
            if fields.until.is_none()
                && (fields.tries.is_some()
                    || fields.timeout_ms.is_some()
                    || fields.message.is_some())
            {
                return Err("tries, timeout_ms, and message belong to a wheel's until".to_owned());
            }
            if fields.tries.is_some() && (fields.timeout_ms.is_some() || fields.message.is_some()) {
                return Err(
                    "a wheel repeats up to tries times or until a timeout, not both".to_owned(),
                );
            }
            if fields.tries == Some(0) {
                return Err("tries must be at least one".to_owned());
            }
            Ok(Command::Wheel {
                target: fields.wheel,
                at: fields.at,
                clicks: fields.clicks,
                down: matches!(fields.direction, WheelDirection::Down),
                modifiers: modifiers(&fields.mods)?,
                until: fields.until.map(|predicate| Repeat {
                    predicate,
                    tries: fields.tries,
                    timeout: fields
                        .timeout_ms
                        .map_or(DEFAULT_WAIT, Duration::from_millis),
                    message: fields.message,
                }),
            })
        }
        "drag" => {
            let fields: DragFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.steps == 0 {
                return Err("drag needs at least one step".to_owned());
            }
            fields.drag.from.accepts(fields.drag.from_at.as_ref())?;
            fields.drag.to.accepts(fields.drag.to_at.as_ref())?;
            Ok(Command::Drag {
                from: fields.drag.from,
                to: fields.drag.to,
                from_at: fields.drag.from_at,
                to_at: fields.drag.to_at,
                steps: fields.steps,
                release: fields.release,
                modifiers: modifiers(&fields.mods)?,
                after_press: press_checks(fields.after_press, fields.message)?,
            })
        }
        "button_up" => {
            let fields: ButtonUpFields = serde_json::from_value(fields).map_err(invalid)?;
            if !matches!(fields.button_up, 1..=3) {
                return Err("button_up takes button 1, 2, or 3".to_owned());
            }
            Ok(Command::ButtonUp(fields.button_up))
        }
        "resize" => {
            let fields: ResizeFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.resize.width == 0 || fields.resize.height == Some(0) {
                return Err("resize needs a positive width and height".to_owned());
            }
            Ok(Command::Resize {
                width: fields.resize.width,
                height: fields.resize.height,
            })
        }
        "settle" => {
            let fields: SettleFields = serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::Settle {
                target: fields.settle,
                timeout: fields
                    .timeout_ms
                    .map_or(DEFAULT_WAIT, Duration::from_millis),
                name: fields.name,
            })
        }
        "wait" => {
            let fields: WaitFields = serde_json::from_value(fields).map_err(invalid)?;
            if fields.samples == 0 {
                return Err("a wait needs at least one sample".to_owned());
            }
            if fields.never.is_none() && fields.never_message.is_some() {
                return Err("never_message belongs to never".to_owned());
            }
            Ok(Command::Wait {
                predicate: fields.wait,
                timeout: fields
                    .timeout_ms
                    .map_or(DEFAULT_WAIT, Duration::from_millis),
                message: fields.message,
                samples: fields.samples,
                settle: fields.settling,
                never: fields
                    .never
                    .map(|predicate| Box::new((predicate, fields.never_message))),
            })
        }
        "expect" => {
            let fields: ExpectFields = serde_json::from_value(fields).map_err(invalid)?;
            let predicates = match fields.expect {
                Value::Array(items) if items.is_empty() => {
                    return Err("expect needs a predicate".to_owned());
                }
                Value::Array(items) => items
                    .into_iter()
                    .map(serde_json::from_value)
                    .collect::<Result<Vec<Predicate>, _>>()
                    .map_err(invalid)?,
                one => vec![serde_json::from_value(one).map_err(invalid)?],
            };
            if fields.hold_ms.is_some() && fields.timeout_ms.is_none() {
                return Err("hold_ms belongs to an expect with timeout_ms".to_owned());
            }
            Ok(Command::Expect {
                predicates,
                message: fields.message,
                timeout: fields.timeout_ms.map(Duration::from_millis),
                hold: fields.hold_ms.map(Duration::from_millis),
            })
        }
        _ => {
            let fields: QueryFields = serde_json::from_value(fields).map_err(invalid)?;
            Ok(Command::Query(fields.query))
        }
    }
}

fn press_checks(
    after_press: Option<Value>,
    message: Option<String>,
) -> Result<Vec<PressCheck>, String> {
    let invalid = |error: serde_json::Error| format!("invalid after_press: {error}");
    match after_press {
        None if message.is_some() => Err("a drag's message names its after_press check".to_owned()),
        None => Ok(Vec::new()),
        Some(Value::Array(steps)) => {
            if message.is_some() {
                return Err("each after_press step carries its own message".to_owned());
            }
            if steps.is_empty() {
                return Err("after_press needs a step".to_owned());
            }
            steps
                .into_iter()
                .map(|step| {
                    let fields: PressStepFields = serde_json::from_value(step).map_err(invalid)?;
                    match (fields.wait, fields.expect) {
                        (Some(predicate), None) => Ok(PressCheck {
                            predicate,
                            wait: Some(fields.timeout_ms.map_or(DEFAULT_WAIT, Duration::from_millis)),
                            message: fields.message,
                        }),
                        (None, Some(predicate)) if fields.timeout_ms.is_none() => Ok(PressCheck {
                            predicate,
                            wait: None,
                            message: fields.message,
                        }),
                        _ => Err(
                            "an after_press step is a wait (with an optional timeout_ms) or an expect"
                                .to_owned(),
                        ),
                    }
                })
                .collect()
        }
        Some(predicate) => Ok(vec![PressCheck {
            predicate: serde_json::from_value(predicate).map_err(invalid)?,
            wait: None,
            message,
        }]),
    }
}
