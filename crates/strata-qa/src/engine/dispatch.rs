// SPDX-License-Identifier: MIT

//! Runs one step.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{
    LOCATE_TIMEOUT, POINTER_GAP, POLL_INTERVAL,
    fields::editable_fields,
    locate::screen_rect,
    lookup::result_rows,
    observe::preferences,
    pointer::{Aimed, DragEnds},
    scope::{Remembered, Scope},
    waits::{Turn, adopt},
};
use crate::{
    Accessor, Aim, Backend, Command, Failure, Outcome, Query, Rect, Shape, Target, number_json,
};

/// Runs a step against the window the backend names, and returns what it came to.
pub async fn execute<B: Backend>(backend: &B, kept: &Remembered<B>, command: Command) -> Outcome {
    if let Command::Adopt {
        title,
        timeout,
        message,
    } = &command
    {
        return adopt(backend, title, *timeout, message.as_deref()).await;
    }
    let Some(window) = backend.window() else {
        return Outcome::error("the Strata window is gone".to_owned());
    };
    if let Err(error) = backend.begin(&window) {
        return Outcome::error(error);
    }
    Scope {
        backend,
        window,
        kept,
    }
    .run(command)
    .await
}

/// What a scenario calls a step only a backend can run.
fn step_name(command: &Command) -> &'static str {
    match command {
        Command::RevealControl { .. } => "reveal_control",
        Command::Reveal { .. } => "reveal",
        Command::SetScrollbar { .. } => "set_scrollbar",
        Command::Capture { .. } => "capture",
        Command::MeasureText { .. } => "measure_text",
        Command::DropSamples { .. } => "drop_samples",
        Command::ClipboardSet(_) => "clipboard_set",
        Command::Clipboard { .. } => "clipboard",
        Command::Keymap(_) => "keymap",
        Command::Invariants => "invariants",
        _ => "the step",
    }
}

fn value(result: Value) -> Outcome {
    Outcome {
        result,
        ..Outcome::default()
    }
}

impl<B: Backend> Scope<'_, B> {
    async fn run(&self, command: Command) -> Outcome {
        let backend = self.backend;
        match command {
            Command::Key { chord, repeat } => {
                for _ in 0..repeat {
                    if let Err(error) = self.press(&chord).await {
                        return Outcome::error(error);
                    }
                }
                Outcome::default()
            }
            Command::Type(text) => {
                for character in text.chars() {
                    if let Err(error) = backend.text_input(&self.window, character).await {
                        return Outcome::error(error);
                    }
                }
                Outcome::default()
            }
            Command::SelectWithKeyboard { name, message } => {
                self.select_with_keyboard(&name, message.as_deref()).await
            }
            Command::OpenMenu {
                target,
                until,
                message,
                open_states,
            } => self.open_menu(&target, &until, message, &open_states).await,
            Command::Adopt { .. } => Outcome::error("adopt runs before a window".to_owned()),
            Command::Keep {
                rect: [x, y, width, height],
                name,
            } => {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "screen coordinates are far below 2^52"
                )]
                let rect = Rect {
                    x: x as f64,
                    y: y as f64,
                    width: width as f64,
                    height: height as f64,
                };
                self.kept.boxes.borrow_mut().insert(name, rect);
                Outcome::default()
            }
            Command::Activate { target, message } => {
                Outcome::from(self.activate(&target, message.as_deref()).await)
            }
            Command::Holds(predicate) => match self.check(&predicate) {
                Ok(verdict) => value(json!(verdict.is_ok())),
                Err(error) => Outcome::error(error),
            },
            Command::Remember { target, name } => self.remember(&target, name).await,
            Command::Note {
                value: accessor,
                dir,
                wait,
                message,
                from,
                through,
                unless_first,
                of,
                middle,
                plus,
                sorted,
                pick,
                expr,
                until,
                until_names,
            } => {
                let shape = Shape {
                    from: from.as_deref(),
                    through: through.as_deref(),
                    middle,
                    plus: &plus,
                    sorted,
                    pick,
                };
                let read = || match accessor {
                    Accessor::ScrollTarget => {
                        let Some(of) = of.as_ref() else {
                            return Err("scroll_target needs of".to_owned());
                        };
                        self.scroll_target(of, dir.as_deref())
                    }
                    Accessor::Bounds => {
                        let Some(of) = of.as_ref() else {
                            return Err("bounds needs of".to_owned());
                        };
                        let [x, y, width, height] = self
                            .locate_now(of, None, Aim::Point)
                            .map_err(|error| error.message().to_owned())?
                            .rect
                            .pixels();
                        Ok(json!(format!(
                            "Bounds(x={x}, y={y}, width={width}, height={height})"
                        )))
                    }
                    Accessor::EditableFields => Ok(json!(editable_fields(backend, &self.window))),
                    Accessor::Number => {
                        let Some(expr) = expr.as_ref() else {
                            return Err("number needs expr".to_owned());
                        };
                        let number = match self.evaluate(expr)? {
                            Ok(number) => number,
                            Err(_) if wait.is_some() => return Ok(Value::Null),
                            Err(missing) => {
                                return Err(format!("cannot work out the number: {missing}"));
                            }
                        };
                        for bound in &until {
                            match self.evaluate(&bound.than)? {
                                Ok(than) if bound.op.holds(number, than) => {}
                                _ => return Ok(Value::Null),
                            }
                        }
                        Ok(number_json(number))
                    }
                    Accessor::VisibleTop => shape.apply(self.visible_top()?),
                    Accessor::Visible => shape.apply(
                        self.visible_rows(dir.as_deref())?
                            .into_iter()
                            .map(|row| row.name)
                            .collect(),
                    ),
                    _ => self.note(accessor, dir.as_deref(), &shape, unless_first.as_deref()),
                };
                let read = || {
                    read().map(|value| match &until_names {
                        Some(names) => {
                            let listed: Vec<String> = value
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|name| name.as_str().map(str::to_owned))
                                .collect();
                            if value.is_array() && names.matches(&listed) {
                                value
                            } else {
                                Value::Null
                            }
                        }
                        None => value,
                    })
                };
                let Some(timeout) = wait else {
                    return read().map_or_else(Outcome::error, value);
                };
                let deadline = Instant::now() + timeout;
                loop {
                    match read() {
                        Err(error) => return Outcome::error(error),
                        Ok(Value::Null) if Instant::now() >= deadline => {
                            let failure = Failure {
                                predicate: json!({ "note": accessor }),
                                actual: Value::Null,
                                message,
                            };
                            return self.timed_out(failure, timeout);
                        }
                        Ok(Value::Null) => backend.sleep(POLL_INTERVAL).await,
                        Ok(result) => return value(result),
                    }
                }
            }
            Command::Click {
                target,
                button,
                count,
                modifiers,
                at,
                left_bound,
                release_modifiers_first,
            } => {
                let aim = Aimed {
                    target: &target,
                    at: at.as_ref(),
                    left_bound: left_bound.as_deref(),
                };
                Outcome::from(if release_modifiers_first {
                    self.click_releasing_modifiers_first(&aim, button, &modifiers)
                        .await
                } else {
                    self.click(&aim, button, count, &modifiers).await
                })
            }
            Command::ClickResultRows { states, modifiers } => {
                let rows = result_rows(backend, &self.window);
                for row in rows {
                    let now = backend.states(&row);
                    if !states.matches(now.selected, now.focused) {
                        continue;
                    }
                    if let Err(error) = self.click_node(&row, &modifiers).await {
                        return Outcome::error(error);
                    }
                }
                Outcome::default()
            }
            Command::Select(target) => self.select(&target).await,
            Command::Move {
                target,
                at,
                left_bound,
            } => Outcome::from(
                self.move_onto(&Aimed {
                    target: &target,
                    at: at.as_ref(),
                    left_bound: left_bound.as_deref(),
                })
                .await,
            ),
            Command::MoveBy { x, y } => Outcome::from(self.move_by(x, y).await),
            Command::Sleep(duration) => {
                backend.sleep(duration).await;
                Outcome::default()
            }
            Command::Wheel {
                target,
                at,
                clicks,
                down,
                modifiers,
                until: None,
            } => Outcome::from(
                self.wheel(&target, at.as_ref(), clicks, down, &modifiers)
                    .await,
            ),
            Command::Wheel {
                target,
                at,
                clicks,
                down,
                modifiers,
                until: Some(repeat),
            } => {
                let turn = Turn {
                    target: &target,
                    at: at.as_ref(),
                    clicks,
                    down,
                    modifiers: &modifiers,
                };
                self.wheel_until(&turn, &repeat).await
            }
            Command::KeysUntil {
                predicate,
                keys,
                times,
                check_first,
                await_focus_change,
                exhausted,
            } => {
                let mut last = None;
                for _ in 0..times {
                    if check_first {
                        match self.check(&predicate) {
                            Err(error) => return Outcome::error(error),
                            Ok(Ok(())) => return Outcome::default(),
                            Ok(Err(failure)) => last = Some(failure),
                        }
                    }
                    let before = self.focused();
                    for chord in &keys {
                        if let Err(error) = self.press(chord).await {
                            return Outcome::error(error);
                        }
                    }
                    if let Some((timeout, message)) = &await_focus_change {
                        let deadline = Instant::now() + *timeout;
                        while self.focused() == before {
                            if Instant::now() >= deadline {
                                let failure = Failure {
                                    predicate: json!({ "await_focus_change": true }),
                                    actual: json!(before.as_ref().map(|node| backend.name(node))),
                                    message: message.clone(),
                                };
                                return self.timed_out(failure, *timeout);
                            }
                            backend.sleep(POLL_INTERVAL).await;
                        }
                    }
                    if !check_first {
                        match self.check(&predicate) {
                            Err(error) => return Outcome::error(error),
                            Ok(Ok(())) => return Outcome::default(),
                            Ok(Err(failure)) => last = Some(failure),
                        }
                    }
                }
                match (exhausted, last) {
                    (Some(message), Some(mut failure)) => {
                        failure.message = Some(message);
                        Outcome {
                            result: json!({ "tree": backend.dump(&self.window) }),
                            failures: vec![failure],
                            error: None,
                        }
                    }
                    _ => Outcome::default(),
                }
            }
            Command::KeyDown(key) => Outcome::from(self.raw_key(&key, true).await),
            Command::KeyUp(key) => Outcome::from(self.raw_key(&key, false).await),
            Command::During {
                checks,
                duration,
                every,
            } => {
                let deadline = Instant::now() + duration;
                loop {
                    for (predicate, message) in &checks {
                        match self.check(predicate) {
                            Err(error) => return Outcome::error(error),
                            Ok(Err(mut failure)) => {
                                failure.message.clone_from(message);
                                return Outcome {
                                    failures: vec![failure],
                                    error: Some("the predicate stopped holding".to_owned()),
                                    ..Outcome::default()
                                };
                            }
                            Ok(Ok(())) => {}
                        }
                    }
                    if Instant::now() >= deadline {
                        return Outcome::default();
                    }
                    backend.sleep(every).await;
                }
            }
            Command::Drag {
                from,
                to,
                from_at,
                to_at,
                steps,
                release,
                modifiers,
                after_press,
            } => {
                let ends = DragEnds {
                    from: &from,
                    to: &to,
                    from_at: from_at.as_ref(),
                    to_at: to_at.as_ref(),
                };
                match self
                    .drag(&ends, steps, release, &modifiers, &after_press)
                    .await
                {
                    Ok(None) => Outcome::default(),
                    Ok(Some(failure)) => Outcome {
                        failures: vec![failure],
                        error: Some("the check after the press failed".to_owned()),
                        ..Outcome::default()
                    },
                    Err(error) => Outcome::error(error),
                }
            }
            Command::ButtonDown(number) => {
                Outcome::from(self.press_button(number, true, Duration::ZERO).await)
            }
            Command::ButtonUp(number) => {
                Outcome::from(self.press_button(number, false, POINTER_GAP).await)
            }
            Command::Resize { width, height } => {
                let resized = backend.resize(&self.window, width, height).await;
                backend.settle(&self.window).await;
                Outcome::from(resized)
            }
            Command::Settle {
                target,
                timeout,
                name,
            } => self.settle_target(&target, timeout, name).await,
            Command::Wait {
                predicate,
                timeout,
                message,
                samples,
                settle,
                never,
            } => {
                let deadline = Instant::now() + timeout;
                let mut steady = 0;
                let mut seen: Option<Failure> = None;
                loop {
                    if let Some(target) = &settle {
                        let left = deadline.saturating_duration_since(Instant::now());
                        let mut settled = self.settle_target(target, left, None).await;
                        if settled.error.is_some() {
                            if !settled.failures.is_empty() {
                                settled.error =
                                    Some(format!("timed out after {} ms", timeout.as_millis()));
                            }
                            return settled;
                        }
                    }
                    if let Some((forbidden, forbidden_message)) = never.as_deref()
                        && seen.is_none()
                    {
                        match self.check(forbidden) {
                            Err(error) => return Outcome::error(error),
                            Ok(Ok(())) => {
                                seen = Some(Failure {
                                    predicate: json!({ "never": forbidden }),
                                    actual: json!("held at a poll"),
                                    message: forbidden_message.clone(),
                                });
                            }
                            Ok(Err(_)) => {}
                        }
                    }
                    match self.check(&predicate) {
                        Err(error) => return Outcome::error(error),
                        Ok(Ok(())) => {
                            steady += 1;
                            if steady >= samples {
                                return match seen {
                                    None => Outcome::default(),
                                    Some(failure) => Outcome {
                                        failures: vec![failure],
                                        error: Some(
                                            "the forbidden predicate held while waiting".to_owned(),
                                        ),
                                        ..Outcome::default()
                                    },
                                };
                            }
                            if Instant::now() >= deadline {
                                let failure = Failure {
                                    predicate: serde_json::to_value(&predicate)
                                        .unwrap_or(Value::Null),
                                    actual: json!(format!("held for {steady} of {samples} polls")),
                                    message,
                                };
                                return self.timed_out(failure, timeout);
                            }
                        }
                        Ok(Err(mut failure)) if Instant::now() >= deadline => {
                            failure.message = message;
                            return self.timed_out(failure, timeout);
                        }
                        Ok(Err(_)) => steady = 0,
                    }
                    backend.sleep(POLL_INTERVAL).await;
                }
            }
            Command::Expect {
                predicates,
                message,
                timeout,
                hold,
            } => {
                let deadline = timeout.map(|timeout| Instant::now() + timeout);
                let mut since: Option<Instant> = None;
                loop {
                    let mut failures = Vec::new();
                    for predicate in &predicates {
                        match self.check(predicate) {
                            Ok(Ok(())) => {}
                            Ok(Err(mut failure)) => {
                                failure.message.clone_from(&message);
                                failures.push(failure);
                            }
                            Err(error) => return Outcome::error(error),
                        }
                    }
                    let held = match hold {
                        None => true,
                        Some(hold) if failures.is_empty() => {
                            since.get_or_insert_with(Instant::now).elapsed() >= hold
                        }
                        Some(_) => {
                            since = None;
                            false
                        }
                    };
                    if failures.is_empty() && held {
                        return Outcome::default();
                    }
                    // Like `_settles`: without a hold, one last look decides.
                    if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                        if failures.is_empty()
                            && let Some(hold) = hold
                        {
                            failures.push(Failure {
                                predicate: json!({ "hold_ms": hold.as_millis() }),
                                actual: json!(format!(
                                    "held for {} ms",
                                    since.map_or(0, |since| since.elapsed().as_millis())
                                )),
                                message,
                            });
                        }
                        return Outcome {
                            failures,
                            ..Outcome::default()
                        };
                    }
                    backend.sleep(POLL_INTERVAL).await;
                }
            }
            Command::Query(Query::State) => value(
                serde_json::to_value(self.scene().observation.app_state()).unwrap_or(Value::Null),
            ),
            Command::Query(Query::Tree) => value(Value::String(backend.dump(&self.window))),
            Command::Query(Query::Prefs) => preferences(backend).map_or_else(Outcome::error, value),
            command @ (Command::RevealControl { .. }
            | Command::Reveal { .. }
            | Command::SetScrollbar { .. }
            | Command::Capture { .. }
            | Command::MeasureText { .. }
            | Command::DropSamples { .. }
            | Command::ClipboardSet(_)
            | Command::Clipboard { .. }
            | Command::Keymap(_)
            | Command::Invariants) => backend.command(self, &command).await.unwrap_or_else(|| {
                Outcome::error(format!(
                    "{} is not supported on this target",
                    step_name(&command)
                ))
            }),
        }
    }

    /// One key event by name, then a painted frame.
    async fn raw_key(&self, name: &str, pressed: bool) -> Result<(), String> {
        self.backend.key(name, pressed).await?;
        self.pause(Duration::ZERO).await;
        Ok(())
    }

    /// Keeps a node under a name. A field is kept for `field` targets and checks; any other
    /// node is kept with its box. A target that need not be drawn is waited for among the
    /// presented nodes, and keeps a box only if it has one.
    async fn remember(&self, target: &Target, name: String) -> Outcome {
        let backend = self.backend;
        if let Target::Role {
            rendered: false, ..
        } = target
        {
            let deadline = Instant::now() + LOCATE_TIMEOUT;
            loop {
                match self.presented(target) {
                    Err(error) => return Outcome::error(error),
                    Ok(Some(node)) => {
                        self.kept.fields.borrow_mut().remove(&name);
                        if let Ok(rect) = screen_rect(backend, &node) {
                            self.kept.boxes.borrow_mut().insert(name.clone(), rect);
                        }
                        self.kept
                            .nodes
                            .borrow_mut()
                            .insert(name, backend.keep(&node));
                        return Outcome::default();
                    }
                    Ok(None) if Instant::now() >= deadline => {
                        return Outcome::error(format!(
                            "no {} presented after {} s",
                            target.to_json(),
                            LOCATE_TIMEOUT.as_secs()
                        ));
                    }
                    Ok(None) => backend.sleep(POLL_INTERVAL).await,
                }
            }
        }
        match self.find(target, None, Aim::Point).await {
            Ok(located) if matches!(target, Target::Field(None) | Target::FieldWithin(_)) => {
                self.kept.nodes.borrow_mut().remove(&name);
                self.kept
                    .fields
                    .borrow_mut()
                    .insert(name, backend.keep(&located.node));
                Outcome::default()
            }
            Ok(located) => {
                self.kept.fields.borrow_mut().remove(&name);
                self.kept
                    .nodes
                    .borrow_mut()
                    .insert(name.clone(), backend.keep(&located.node));
                self.kept.boxes.borrow_mut().insert(name, located.rect);
                Outcome::default()
            }
            Err(error) => Outcome::error(error),
        }
    }
}
