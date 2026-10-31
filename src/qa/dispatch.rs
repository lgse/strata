// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use gtk::{glib, prelude::*};
use serde_json::{Value, json};

use super::{
    Controller, LOCATE_TIMEOUT, Outcome, POINTER_GAP, POLL_INTERVAL, Sample,
    clipboard::{abridged, private_display, read_clipboard},
    driver::Input,
    input::Event,
    note::{Shape, note},
    observe::{evaluate, number_json, preferences, scroll_target, visible_rows, visible_top},
    pointer::{AfterPress, Aimed, DragEnds},
    protocol::{self, Accessor, Chord, Command, Failure, Predicate, Query, Target},
    reveal::set_scrollbar,
    samples::grab,
    settle, state,
    targets::{self, Aim, Rect},
};

impl Controller {
    pub(super) async fn execute(&self, command: Command) -> Outcome {
        if let Command::Adopt {
            title,
            timeout,
            message,
        } = &command
        {
            return self.adopt(title, *timeout, message.as_deref()).await;
        }
        let Some(window) = self.window() else {
            return Outcome::error("the Strata window is gone".to_owned());
        };
        let Some(injector) = self.injector.as_ref() else {
            return Outcome::error("no input connection".to_owned());
        };
        if injector.logging() {
            match window
                .surface()
                .ok_or_else(|| "the window has no surface".to_owned())
                .and_then(|surface| injector.surface_origin(&surface))
            {
                Ok(origin) => injector.set_log_origin(origin),
                Err(error) => return Outcome::error(error),
            }
        }
        let input = Input {
            injector,
            window: &window,
            memory: &self.memory,
            widgets: &self.widgets,
            boxes: &self.settled,
        };
        match command {
            Command::Key { chord, repeat } => {
                for _ in 0..repeat {
                    if let Err(error) = input.press(&chord).await {
                        return Outcome::error(error);
                    }
                }
                Outcome::default()
            }
            Command::Type(text) => {
                for character in text.chars() {
                    let chord = Chord {
                        modifiers: Vec::new(),
                        key: character.to_string(),
                    };
                    if let Err(error) = input.press(&chord).await {
                        return Outcome::error(error);
                    }
                }
                Outcome::default()
            }
            Command::SelectWithKeyboard { name, message } => {
                input.select_with_keyboard(&name, message.as_deref()).await
            }
            Command::OpenMenu {
                target,
                until,
                message,
                open_states,
            } => {
                self.open_menu(&input, &target, &until, message, &open_states)
                    .await
            }
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
                self.settled.borrow_mut().insert(name, rect);
                Outcome::default()
            }
            Command::RevealControl { target } => self.reveal_control(&input, &target).await,
            Command::Reveal { target } => self.reveal(&input, &target).await,
            Command::Activate { target, message } => {
                Outcome::from(input.activate(&target, message.as_deref()).await)
            }
            Command::Holds(predicate) => match self.check(&input, &predicate) {
                Ok(verdict) => Outcome {
                    result: json!(verdict.is_ok()),
                    ..Outcome::default()
                },
                Err(error) => Outcome::error(error),
            },
            Command::Remember {
                target:
                    target @ Target::Role {
                        rendered: false, ..
                    },
                name,
            } => {
                let deadline = Instant::now() + LOCATE_TIMEOUT;
                loop {
                    match input.presented(&target) {
                        Err(error) => return Outcome::error(error),
                        Ok(Some(widget)) => {
                            self.memory.borrow_mut().remove(&name);
                            if let Ok(rect) = targets::screen_rect(&widget, injector) {
                                self.settled.borrow_mut().insert(name.clone(), rect);
                            }
                            self.widgets.borrow_mut().insert(name, widget.downgrade());
                            return Outcome::default();
                        }
                        Ok(None) if Instant::now() >= deadline => {
                            return Outcome::error(format!(
                                "no {} presented after {} s",
                                target.to_json(),
                                LOCATE_TIMEOUT.as_secs()
                            ));
                        }
                        Ok(None) => glib::timeout_future(POLL_INTERVAL).await,
                    }
                }
            }
            Command::Remember { target, name } => match input.find(&target, None, Aim::Point).await
            {
                Ok(located) if matches!(target, Target::Field(None) | Target::FieldWithin(_)) => {
                    self.widgets.borrow_mut().remove(&name);
                    self.memory
                        .borrow_mut()
                        .insert(name, located.widget.downgrade());
                    Outcome::default()
                }
                Ok(located) => {
                    self.memory.borrow_mut().remove(&name);
                    self.widgets
                        .borrow_mut()
                        .insert(name.clone(), located.widget.downgrade());
                    self.settled.borrow_mut().insert(name, located.rect);
                    Outcome::default()
                }
                Err(error) => Outcome::error(error),
            },
            Command::Note {
                value,
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
                let read = || match value {
                    Accessor::ScrollTarget => {
                        let Some(of) = of.as_ref() else {
                            return Err("scroll_target needs of".to_owned());
                        };
                        scroll_target(&input, of, dir.as_deref())
                    }
                    Accessor::Bounds => {
                        let Some(of) = of.as_ref() else {
                            return Err("bounds needs of".to_owned());
                        };
                        let [x, y, width, height] = input
                            .locate_now(of, None, Aim::Point)
                            .map_err(|error| error.message().to_owned())?
                            .rect
                            .pixels();
                        Ok(json!(format!(
                            "Bounds(x={x}, y={y}, width={width}, height={height})"
                        )))
                    }
                    Accessor::EditableFields => Ok(json!(targets::editable_fields(&window))),
                    Accessor::Number => {
                        let Some(expr) = expr.as_ref() else {
                            return Err("number needs expr".to_owned());
                        };
                        let number = match evaluate(&input, expr)? {
                            Ok(number) => number,
                            Err(_) if wait.is_some() => return Ok(Value::Null),
                            Err(missing) => {
                                return Err(format!("cannot work out the number: {missing}"));
                            }
                        };
                        for bound in &until {
                            match evaluate(&input, &bound.than)? {
                                Ok(than) if bound.op.holds(number, than) => {}
                                _ => return Ok(Value::Null),
                            }
                        }
                        Ok(number_json(number))
                    }
                    Accessor::VisibleTop => shape.apply(visible_top(&input)?),
                    Accessor::Visible => shape.apply(
                        visible_rows(&input, dir.as_deref())?
                            .into_iter()
                            .map(|row| row.name)
                            .collect(),
                    ),
                    _ => note(
                        &window,
                        value,
                        dir.as_deref(),
                        &shape,
                        unless_first.as_deref(),
                    ),
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
                    return read().map_or_else(Outcome::error, |result| Outcome {
                        result,
                        ..Outcome::default()
                    });
                };
                let deadline = Instant::now() + timeout;
                loop {
                    match read() {
                        Err(error) => return Outcome::error(error),
                        Ok(Value::Null) if Instant::now() >= deadline => {
                            let failure = Failure {
                                predicate: json!({ "note": value }),
                                actual: Value::Null,
                                message,
                            };
                            return Outcome::timed_out(&window, failure, timeout);
                        }
                        Ok(Value::Null) => glib::timeout_future(POLL_INTERVAL).await,
                        Ok(result) => {
                            return Outcome {
                                result,
                                ..Outcome::default()
                            };
                        }
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
                    input
                        .click_releasing_modifiers_first(&aim, button, &modifiers)
                        .await
                } else {
                    input.click(&aim, button, count, &modifiers).await
                })
            }
            Command::ClickResultRows { states, modifiers } => {
                let rows = targets::result_rows(window.upcast_ref());
                for row in rows {
                    let now = targets::states(&row);
                    if !states.matches(now.selected, now.focused) {
                        continue;
                    }
                    if let Err(error) = input.click_widget(&row, &modifiers).await {
                        return Outcome::error(error);
                    }
                }
                Outcome::default()
            }
            Command::Select(target) => input.select(&target).await,
            Command::Move {
                target,
                at,
                left_bound,
            } => Outcome::from(
                input
                    .move_onto(&Aimed {
                        target: &target,
                        at: at.as_ref(),
                        left_bound: left_bound.as_deref(),
                    })
                    .await,
            ),
            Command::MoveBy { x, y } => Outcome::from(input.move_by(x, y).await),
            Command::Sleep(duration) => {
                glib::timeout_future(duration).await;
                Outcome::default()
            }
            Command::SetScrollbar { orientation, value } => {
                Outcome::from(set_scrollbar(&window, orientation, value))
            }
            Command::Wheel {
                target,
                at,
                clicks,
                down,
                modifiers,
                until: None,
            } => Outcome::from(
                input
                    .wheel(&target, at.as_ref(), clicks, down, &modifiers)
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
                self.wheel_until(
                    &input,
                    &target,
                    at.as_ref(),
                    clicks,
                    down,
                    &modifiers,
                    &repeat,
                )
                .await
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
                        match self.check(&input, &predicate) {
                            Err(error) => return Outcome::error(error),
                            Ok(Ok(())) => return Outcome::default(),
                            Ok(Err(failure)) => last = Some(failure),
                        }
                    }
                    let before = input.focused_widget();
                    for chord in &keys {
                        if let Err(error) = input.press(chord).await {
                            return Outcome::error(error);
                        }
                    }
                    if let Some((timeout, message)) = &await_focus_change {
                        let deadline = Instant::now() + *timeout;
                        while input.focused_widget() == before {
                            if Instant::now() >= deadline {
                                let failure = Failure {
                                    predicate: json!({ "await_focus_change": true }),
                                    actual: json!(before.as_ref().map(targets::accessible_name)),
                                    message: message.clone(),
                                };
                                return Outcome::timed_out(input.window, failure, *timeout);
                            }
                            glib::timeout_future(POLL_INTERVAL).await;
                        }
                    }
                    if !check_first {
                        match self.check(&input, &predicate) {
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
                            result: json!({ "tree": state::dump(input.window) }),
                            failures: vec![failure],
                            error: None,
                        }
                    }
                    _ => Outcome::default(),
                }
            }
            Command::KeyDown(key) => Outcome::from(input.raw_key(&key, true).await),
            Command::KeyUp(key) => Outcome::from(input.raw_key(&key, false).await),
            Command::During {
                checks,
                duration,
                every,
            } => {
                let deadline = Instant::now() + duration;
                loop {
                    for (predicate, message) in &checks {
                        match self.check(&input, predicate) {
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
                    glib::timeout_future(every).await;
                }
            }
            Command::Capture {
                target,
                inset,
                name,
            } => match input
                .find(&target, None, Aim::Point)
                .await
                .and_then(|located| {
                    let [x, y, width, height] = located.rect.pixels();
                    let inset = i64::from(inset);
                    grab(
                        injector,
                        [x + inset, y + inset, width - 2 * inset, height - 2 * inset],
                    )
                }) {
                Ok(capture) => {
                    self.samples
                        .borrow_mut()
                        .insert(name, Sample::Pixels(capture));
                    Outcome::default()
                }
                Err(error) => Outcome::error(error),
            },
            Command::MeasureText { kept, name } => match self.measure_text(injector, &kept) {
                Ok(Ok((position, contrast))) => {
                    self.samples
                        .borrow_mut()
                        .insert(name, Sample::Text { position, contrast });
                    Outcome::default()
                }
                Ok(Err(failure)) => Outcome {
                    failures: vec![failure],
                    error: Some("the label has no ink".to_owned()),
                    ..Outcome::default()
                },
                Err(error) => Outcome::error(error),
            },
            Command::DropSamples {
                kept,
                resting,
                dragging,
                escape,
                name,
            } => {
                self.drop_samples(&input, &kept, &resting, &dragging, escape, name)
                    .await
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
                let check = |predicate: &Predicate| self.check(&input, predicate);
                let ends = DragEnds {
                    from: &from,
                    to: &to,
                    from_at: from_at.as_ref(),
                    to_at: to_at.as_ref(),
                };
                let pressed = AfterPress {
                    checks: &after_press,
                    check: &check,
                };
                match input
                    .drag(&ends, steps, release, &modifiers, &pressed)
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
            Command::ButtonDown(number) => Outcome::from(
                input
                    .emit(
                        Event::Button {
                            number,
                            pressed: true,
                        },
                        Duration::ZERO,
                    )
                    .await,
            ),
            Command::ButtonUp(number) => Outcome::from(
                input
                    .emit(
                        Event::Button {
                            number,
                            pressed: false,
                        },
                        POINTER_GAP,
                    )
                    .await,
            ),
            Command::Resize { width, height } => {
                let height = height.map_or_else(
                    || u16::try_from(window.height()).map_err(|error| error.to_string()),
                    Ok,
                );
                let resized = window
                    .surface()
                    .ok_or_else(|| "the window has no surface".to_owned())
                    .and_then(|surface| injector.resize(&surface, width, height?));
                settle(Some(&window)).await;
                Outcome::from(resized)
            }
            Command::Settle {
                target,
                timeout,
                name,
            } => self.settle_target(&input, &target, timeout, name).await,
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
                        let mut settled = self.settle_target(&input, target, left, None).await;
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
                        match self.check(&input, forbidden) {
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
                    match self.check(&input, &predicate) {
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
                                return Outcome::timed_out(&window, failure, timeout);
                            }
                        }
                        Ok(Err(mut failure)) if Instant::now() >= deadline => {
                            failure.message = message;
                            return Outcome::timed_out(&window, failure, timeout);
                        }
                        Ok(Err(_)) => steady = 0,
                    }
                    glib::timeout_future(POLL_INTERVAL).await;
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
                        match self.check(&input, predicate) {
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
                    glib::timeout_future(POLL_INTERVAL).await;
                }
            }
            Command::ClipboardSet(text) => match private_display() {
                Ok(display) => {
                    display.clipboard().set_text(&text);
                    display.flush();
                    if let Some(previous) = self.clipboard.replace(Some(display)) {
                        previous.close();
                    }
                    Outcome::default()
                }
                Err(error) => Outcome::error(error),
            },
            Command::Clipboard { text, message } => {
                let display = match self.clipboard.take().map_or_else(private_display, Ok) {
                    Ok(display) => display,
                    Err(error) => return Outcome::error(error),
                };
                let read = read_clipboard(&display).await;
                display.close();
                match read {
                    Some(read) if text.matches(&read) => Outcome::default(),
                    Some(read) => Outcome {
                        failures: vec![Failure {
                            predicate: json!({ "clipboard": text }),
                            actual: json!(abridged(&read)),
                            message,
                        }],
                        error: Some("the clipboard holds other text".to_owned()),
                        ..Outcome::default()
                    },
                    None => {
                        let failure = Failure {
                            predicate: json!({ "clipboard": text }),
                            actual: Value::Null,
                            message: Some(
                                "selected table text on the private display clipboard".to_owned(),
                            ),
                        };
                        Outcome::timed_out(&window, failure, protocol::DEFAULT_WAIT)
                    }
                }
            }
            Command::Query(Query::State) => Outcome {
                result: serde_json::to_value(state::scan(&window).observation.app_state())
                    .unwrap_or(Value::Null),
                ..Outcome::default()
            },
            Command::Query(Query::Tree) => Outcome {
                result: Value::String(state::dump(&window)),
                ..Outcome::default()
            },
            Command::Query(Query::Prefs) => match preferences() {
                Ok(preferences) => Outcome {
                    result: preferences,
                    ..Outcome::default()
                },
                Err(error) => Outcome::error(error),
            },
        }
    }
}
