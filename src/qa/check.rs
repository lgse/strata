// SPDX-License-Identifier: MIT

use std::{path::Path, time::Duration};

use gtk::prelude::*;
use serde_json::{Value, json};

use super::{
    Controller, Sample,
    driver::Input,
    observe::{
        self, child_processes, evaluate, footer_fill, hit_path, list_sections, number_json,
        preferences, scroll_value, theme_accent, visible_rows, visible_top,
    },
    pixels,
    protocol::{Check, Direction, Failure, FieldRef, PointAxis, Predicate, Target},
    samples::grab,
    state::{self, FieldState},
    targets::{self, Aim, Kept, LookupError},
};

impl Controller {
    /// Outer error: a broken lookup, which waiting cannot fix; inner: the predicate not holding.
    pub(super) fn check(
        &self,
        input: &Input<'_>,
        predicate: &Predicate,
    ) -> Result<Result<(), Failure>, String> {
        if !predicate.check.is_live() {
            return Ok(state::scan(input.window).observation.evaluate(predicate));
        }
        let mismatch = |actual: Value| Failure {
            predicate: serde_json::to_value(predicate).unwrap_or(Value::Null),
            actual,
            message: None,
        };
        let verdict = |holds: bool, actual: Value| {
            if holds { Ok(()) } else { Err(mismatch(actual)) }
        };
        let locate = |target: &Target| match input.locate_now(target, None, Aim::Point) {
            Ok(located) => Ok(Some(located)),
            Err(LookupError::Missing(_)) => Ok(None),
            Err(LookupError::Broken(error)) => Err(error),
        };
        Ok(match &predicate.check {
            Check::Any(predicates) => {
                let mut actual = Vec::new();
                for predicate in predicates {
                    match self.check(input, predicate)? {
                        Ok(()) => return Ok(Ok(())),
                        Err(failure) => actual.push(failure.actual),
                    }
                }
                Err(mismatch(json!(actual)))
            }
            Check::Field(check) => {
                let field = input
                    .field(&check.field())?
                    .and_then(|field| FieldState::read(&field));
                verdict(state::field_matches(check, field.as_ref()), json!(field))
            }
            Check::Pref(check) => {
                let actual = preferences()?
                    .get(&check.key)
                    .cloned()
                    .unwrap_or(Value::Null);
                let holds = match (&check.value, &check.contains, &check.one_of) {
                    (Some(value), _, _) => actual == *value,
                    (None, Some(part), _) => match &actual {
                        Value::Array(items) => items.iter().any(|item| item.as_str() == Some(part)),
                        Value::String(text) => text.contains(part.as_str()),
                        _ => false,
                    },
                    (None, None, Some(values)) => values.contains(&actual),
                    (None, None, None) => false,
                };
                verdict(holds, actual)
            }
            Check::EditorWithinViewport(field) => {
                let Some(editor) = input.field(field)? else {
                    return Ok(Err(mismatch(json!("no field"))));
                };
                let window = input.window.upcast_ref::<gtk::Widget>();
                let scene = state::scan(input.window);
                let pane = scene
                    .pane_widgets(None)
                    .and_then(|pane| pane.pane.compute_bounds(window));
                let editor = editor.compute_bounds(window);
                match (editor, pane) {
                    (Some(editor), Some(pane)) => {
                        let editor = [editor.x(), editor.y(), editor.width(), editor.height()]
                            .map(|value| f64::from(value).floor() as i64);
                        let pane_x = f64::from(pane.x()).floor() as i64;
                        let width = i64::from(input.window.width());
                        verdict(
                            state::within_viewport(editor, pane_x, width),
                            json!({ "editor": editor, "pane_x": pane_x, "window_width": width }),
                        )
                    }
                    _ => Err(mismatch(json!("the field or the pane has no bounds"))),
                }
            }
            Check::State(check) => match input.resolve(&check.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(widget) => {
                    let states = targets::states(&widget);
                    let wanted = [
                        (check.selected, states.selected),
                        (check.focused, states.focused),
                        (check.checked, states.checked),
                        (check.pressed, states.pressed),
                        (check.sensitive, states.sensitive),
                        (check.selectable, states.selectable),
                        (check.editable, states.editable),
                        (check.focusable, states.focusable),
                        (check.busy, states.busy),
                        (check.marked, states.marked),
                    ];
                    verdict(
                        wanted
                            .iter()
                            .all(|(wanted, actual)| wanted.is_none_or(|wanted| wanted == *actual)),
                        json!({
                            "selected": states.selected,
                            "focused": states.focused,
                            "checked": states.checked,
                            "pressed": states.pressed,
                            "sensitive": states.sensitive,
                            "selectable": states.selectable,
                            "editable": states.editable,
                            "focusable": states.focusable,
                            "busy": states.busy,
                            "marked": states.marked,
                        }),
                    )
                }
            },
            Check::Present(target) => match locate(target)? {
                Some(_) => Ok(()),
                None => Err(mismatch(json!(false))),
            },
            Check::Absent(target) => match locate(target)? {
                Some(located) => Err(mismatch(json!(located.rect.pixels()))),
                None => Ok(()),
            },
            Check::Moved(movement) => {
                let key = movement
                    .since
                    .clone()
                    .unwrap_or_else(|| movement.target.to_json().to_string());
                let Some(before) = self.settled.borrow().get(&key).copied() else {
                    return Err(format!(
                        "moved needs an earlier settle or remember on {key}"
                    ));
                };
                match locate(&movement.target)? {
                    None => Err(mismatch(json!("not on screen"))),
                    Some(now) => {
                        let (settled, now) = (before.pixels(), now.rect.pixels());
                        let moved = match (movement.direction, movement.y_within) {
                            (Direction::Up, _) => now[1] < settled[1],
                            (Direction::Down, _) => now[1] > settled[1],
                            (Direction::Left, _) => now[0] < settled[0],
                            (Direction::Right, _) => now[0] > settled[0],
                            (Direction::None, None) if movement.within.is_some() => {
                                let within = i64::from(movement.within.unwrap_or_default());
                                (now[0] - settled[0]).abs() <= within
                                    && (now[1] - settled[1]).abs() <= within
                                    && now[2..] == settled[2..]
                            }
                            (Direction::None, None) => now == settled,
                            (Direction::None, Some(within)) => {
                                (now[1] - settled[1]).abs() <= i64::from(within)
                            }
                        };
                        verdict(moved, json!({ "settled": settled, "now": now }))
                    }
                }
            }
            Check::OnScreen(target) => match locate(target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let window = targets::screen_rect(input.window.upcast_ref(), input.injector)
                        .map_err(|error| error.message().to_owned())?
                        .pixels();
                    let bounds = located.rect.pixels();
                    verdict(
                        state::on_screen(bounds, window),
                        json!({ "bounds": bounds, "window": window }),
                    )
                }
            },
            Check::Blank(blank) => match locate(&blank.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let point = located.rect.point(blank.at.as_ref());
                    let point = (point.0.floor() as i64, point.1.floor() as i64);
                    let mut covering = Vec::new();
                    for widget in targets::presented(&located.widget).into_iter().skip(1) {
                        let role = targets::role_name(&widget);
                        if matches!(role, "panel" | "filler" | "group")
                            || state::rendered_bounds(&widget).is_none()
                        {
                            continue;
                        }
                        let Ok(rect) = targets::screen_rect(&widget, input.injector) else {
                            continue;
                        };
                        if state::contains(rect.pixels(), point) {
                            covering.push(json!({
                                "role": role,
                                "name": targets::accessible_name(&widget),
                            }));
                        }
                    }
                    verdict(covering.is_empty(), json!(covering))
                }
            },
            Check::Not(inner) => match self.check(input, inner)? {
                Ok(()) => Err(mismatch(json!("the negated predicate holds"))),
                Err(_) => Ok(()),
            },
            Check::Visible(names) => {
                let visible: Vec<String> = visible_rows(input, predicate.dir.as_deref())?
                    .into_iter()
                    .map(|row| row.name)
                    .collect();
                verdict(names.matches(&visible), json!(visible))
            }
            Check::ScrollValue(check) => {
                let value = scroll_value(input, predicate.dir.as_deref())?;
                let wanted = check.at_least.as_f64().unwrap_or(f64::INFINITY);
                verdict(value.is_some_and(|value| value >= wanted), json!(value))
            }
            Check::BandSelected(band) => {
                let (from, to) = (self.kept_box(&band.from)?, self.kept_box(&band.to)?);
                let [from_x, ..] = from.pixels();
                let [to_x, to_y, ..] = to.pixels();
                let inside: Vec<(String, bool)> = visible_rows(input, predicate.dir.as_deref())?
                    .into_iter()
                    .filter(|row| {
                        let [x, y, width, height] = row.name_box;
                        y + height <= to_y && x < to_x && x + width > from_x
                    })
                    .map(|row| (row.name, row.selected))
                    .collect();
                verdict(
                    !inside.is_empty() && inside.iter().all(|(_, selected)| *selected),
                    json!(inside),
                )
            }
            Check::FocusWithin(target) => match locate(target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let focus = gtk::prelude::GtkWindowExt::focus(input.window)
                        .filter(|focus| focus.has_focus() && focus.is_mapped());
                    let within = focus.as_ref().is_some_and(|focus| {
                        *focus == located.widget || focus.is_ancestor(&located.widget)
                    });
                    verdict(
                        within,
                        json!(focus.map(|focus| targets::accessible_name(&focus))),
                    )
                }
            },
            Check::Resized(resized) => {
                let before = self.kept_box(&resized.since)?.pixels();
                match locate(&resized.target)? {
                    None => Err(mismatch(json!("not on screen"))),
                    Some(now) => {
                        let now = now.rect.pixels();
                        verdict(
                            resized.width.matches(now[2] - before[2]),
                            json!({ "kept": before, "now": now }),
                        )
                    }
                }
            }
            Check::ContainsPoint(contains) => {
                match (locate(&contains.target)?, locate(&contains.point)?) {
                    (Some(target), Some(point)) => {
                        let [x, y, width, height] = target.rect.pixels();
                        let (point_x, point_y) = point.point;
                        let (start, extent, at) = match contains.axis {
                            PointAxis::X => (x, width, point_x.floor() as i64),
                            PointAxis::Y => (y, height, point_y.floor() as i64),
                        };
                        verdict(
                            start <= at && at < start + extent,
                            json!({ "bounds": target.rect.pixels(), "point": [point_x, point_y] }),
                        )
                    }
                    _ => Err(mismatch(json!("not on screen"))),
                }
            }
            Check::FooterFill(fill) => match footer_fill(input, fill)? {
                Ok(()) => Ok(()),
                Err(actual) => Err(mismatch(actual)),
            },
            Check::Repainted(repainted) => {
                let Some(Sample::Pixels(before)) =
                    self.samples.borrow().get(&repainted.since).cloned()
                else {
                    return Err(format!("no capture was kept as {:?}", repainted.since));
                };
                let width = i64::try_from(before.width).map_err(|error| error.to_string())?;
                let height = i64::try_from(before.height).map_err(|error| error.to_string())?;
                let after = grab(input.injector, [before.x, before.y, width, height])?;
                let pointer = (before.x + width / 2, before.y + height / 2);
                verdict(
                    pixels::differs_beside_pointer(&before, &after, pointer),
                    json!("no pixel changed beside the pointer"),
                )
            }
            Check::ContrastBelow(below) => {
                let (_, of) = self.text_sample(&below.of)?;
                let (_, to) = self.text_sample(&below.to)?;
                let share = below.share.as_f64().unwrap_or_default();
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "pixel sums stay far below 2^52, as Python's float arithmetic assumes"
                )]
                let holds = (of as f64) < to as f64 * share;
                verdict(holds, json!({ "of": of, "to": to }))
            }
            Check::Elapsed(elapsed) => match self.samples.borrow().get(&elapsed.since) {
                Some(Sample::Instant(since)) => {
                    let passed = since.elapsed();
                    verdict(
                        passed >= Duration::from_millis(elapsed.ms),
                        json!(passed.as_millis()),
                    )
                }
                _ => return Err(format!("no instant was kept as {:?}", elapsed.since)),
            },
            Check::DiagnosticsMention(words) => {
                let words: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
                let mentioned = self.windows().iter().any(|window| {
                    targets::presented(window.upcast_ref())
                        .iter()
                        .any(|widget| {
                            let name = targets::accessible_name(widget).to_lowercase();
                            let description = crate::ui::accessibility_qa::naming(widget)
                                .and_then(|naming| naming.description)
                                .unwrap_or_default()
                                .to_lowercase();
                            words
                                .iter()
                                .any(|word| name.contains(word) || description.contains(word))
                        })
                });
                verdict(mentioned, json!(false))
            }
            Check::All(predicates) => {
                let mut actual = Vec::new();
                let mut holds = true;
                for predicate in predicates {
                    match self.check(input, predicate)? {
                        Ok(()) => actual.push(json!(true)),
                        Err(failure) => {
                            holds = false;
                            actual.push(failure.actual);
                        }
                    }
                }
                verdict(holds, json!(actual))
            }
            Check::EditableFields(expected) => {
                let count = targets::editable_fields(input.window);
                verdict(count == *expected, json!(count))
            }
            Check::Description(check) => match input.resolve(&check.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(widget) => {
                    let description = targets::accessible_description(&widget);
                    verdict(check.text.matches(&description), json!(description))
                }
            },
            Check::Mentions(mention) => match locate(&mention.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let mut widgets = targets::presented(&located.widget);
                    if let Some(role) = &mention.role {
                        widgets.retain(|widget| {
                            targets::role_name(widget) == role
                                && state::rendered_bounds(widget).is_some()
                        });
                    }
                    let found = match (&mention.text, &mention.words) {
                        _ if mention.equals.is_some() => {
                            let equals = mention.equals.as_deref().unwrap_or_default();
                            widgets
                                .iter()
                                .filter(|widget| state::rendered_bounds(widget).is_some())
                                .any(|widget| {
                                    state::collapse_whitespace(&targets::accessible_name(widget))
                                        == equals
                                })
                        }
                        (Some(text), _) => widgets.iter().any(|widget| {
                            targets::accessible_name(widget).contains(text.as_str())
                                || targets::accessible_description(widget).contains(text.as_str())
                        }),
                        (None, Some(words)) => {
                            let words = state::collapse_whitespace(words);
                            widgets
                                .iter()
                                .filter(|widget| state::rendered_bounds(widget).is_some())
                                .any(|widget| {
                                    let shown = format!(
                                        "{} {}",
                                        targets::accessible_name(widget),
                                        targets::text_content(widget).unwrap_or_default()
                                    );
                                    state::collapse_whitespace(&shown).contains(&words)
                                })
                        }
                        (None, None) => false,
                    };
                    verdict(found, json!(false))
                }
            },
            Check::Presented(target) => {
                let found = targets::presented_target(
                    input.window,
                    input.injector,
                    Kept {
                        memory: &input.memory.borrow(),
                        widgets: &input.widgets.borrow(),
                        boxes: &input.boxes.borrow(),
                    },
                    target,
                )
                .map_err(|error| error.message().to_owned())?;
                verdict(found.is_some(), json!(false))
            }
            Check::File(file) => match (&file.exists, &file.mode_bits) {
                (_, Some(bits)) => {
                    use std::os::unix::fs::PermissionsExt as _;
                    let mask =
                        u32::from_str_radix(&bits.mask, 8).map_err(|error| error.to_string())?;
                    match observe::off_ui_thread(|| std::fs::metadata(&file.path)) {
                        Ok(metadata) => {
                            let mode = metadata.permissions().mode();
                            verdict((mode & mask != 0) == bits.set, json!(format!("{mode:o}")))
                        }
                        Err(error) => Err(mismatch(json!(error.to_string()))),
                    }
                }
                (exists, None) => {
                    let found =
                        observe::off_ui_thread(|| Path::new(&file.path).symlink_metadata().is_ok());
                    verdict(Some(found) == *exists, json!(found))
                }
            },
            Check::FocusedWidget(expected) => {
                let focused = targets::presented(input.window.upcast_ref())
                    .into_iter()
                    .find(|widget| {
                        targets::states(widget).focused && state::rendered_bounds(widget).is_some()
                    })
                    .map(|widget| targets::accessible_name(&widget));
                let holds = match (expected, &focused) {
                    (None, None) => true,
                    (Some(expected), Some(focused)) => expected.matches(focused),
                    _ => false,
                };
                verdict(holds, json!(focused))
            }
            Check::Inside(inside) => match (locate(&inside.target)?, locate(&inside.of)?) {
                (Some(target), Some(of)) => {
                    let (inner, outer) = (target.rect.pixels(), of.rect.pixels());
                    verdict(
                        state::inside(inner, outer, inside.axis),
                        json!({ "target": inner, "of": outer }),
                    )
                }
                _ => Err(mismatch(json!("not on screen"))),
            },
            Check::EditorWithinWindow(check) => {
                let Some(editor) = input.field(&FieldRef {
                    of: Some(check.of.clone()),
                    name: None,
                })?
                else {
                    return Ok(Err(mismatch(json!("no field"))));
                };
                let window = input.window.upcast_ref::<gtk::Widget>();
                let scene = state::scan(input.window);
                let pane = scene
                    .pane_widgets(check.pane.as_deref())
                    .and_then(|pane| pane.pane.compute_bounds(window));
                match (editor.compute_bounds(window), pane) {
                    (Some(editor), Some(pane)) => {
                        let editor = [editor.x(), editor.y(), editor.width(), editor.height()]
                            .map(|value| f64::from(value).floor() as i64);
                        let pane_x = f64::from(pane.x()).floor() as i64;
                        let width = i64::from(input.window.width());
                        verdict(
                            state::within_window(editor, pane_x, width),
                            json!({ "editor": editor, "pane_x": pane_x, "window_width": width }),
                        )
                    }
                    _ => Err(mismatch(json!("the field or the pane has no bounds"))),
                }
            }
            Check::Replaced(replaced) => {
                let Some(kept) = input.widgets.borrow().get(&replaced.since).cloned() else {
                    return Err(if input.memory.borrow().contains_key(&replaced.since) {
                        format!(
                            "{:?} was remembered as a field, not a widget",
                            replaced.since
                        )
                    } else {
                        format!("no widget was remembered as {:?}", replaced.since)
                    });
                };
                match locate(&replaced.target)? {
                    None => Err(mismatch(json!("not on screen"))),
                    Some(now) => {
                        verdict(kept.upgrade().as_ref() != Some(&now.widget), json!(false))
                    }
                }
            }
            Check::Pixel(pixel) => match locate(&pixel.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let [x, y, width, height] = located.rect.pixels();
                    let (x, y) = (x + width.div_euclid(2), y + height.div_euclid(2));
                    let colour = grab(input.injector, [x, y, 1, 1])?.pixels[0];
                    verdict(colour == pixel.rgb, json!(colour))
                }
            },
            Check::AccentPixels(accent) => {
                let wanted = theme_accent()?;
                match locate(&accent.target)? {
                    None => Err(mismatch(json!("not on screen"))),
                    Some(located) => {
                        let [x, y, width, height] = located.rect.pixels();
                        let inset = i64::from(accent.inset);
                        let capture = grab(
                            input.injector,
                            [x + inset, y + inset, width - 2 * inset, height - 2 * inset],
                        )?;
                        let count = capture
                            .pixels
                            .iter()
                            .filter(|colour| **colour == wanted)
                            .count();
                        verdict(
                            count > accent.more_than,
                            json!({ "accent": wanted, "count": count }),
                        )
                    }
                }
            }
            Check::AnyResultRow(wanted) => {
                let rows: Vec<Value> = targets::result_rows(input.window.upcast_ref())
                    .iter()
                    .map(|row| {
                        let states = targets::states(row);
                        json!([
                            targets::accessible_name(row),
                            states.selected,
                            states.focused
                        ])
                    })
                    .collect();
                let holds = targets::result_rows(input.window.upcast_ref())
                    .iter()
                    .any(|row| {
                        let states = targets::states(row);
                        wanted.matches(states.selected, states.focused)
                    });
                verdict(holds, json!(rows))
            }
            Check::HitPath(hit) => {
                if !hit.hits.contains_key(&hit.name) {
                    return Err(format!("{:?} is not one of the hits", hit.name));
                }
                let shown = hit_path(input.window, hit);
                verdict(
                    shown.as_deref() == hit.hits.get(&hit.name).map(String::as_str),
                    json!(shown),
                )
            }
            Check::Compare(comparison) => {
                let left = evaluate(input, &comparison.left)?;
                let right = evaluate(input, &comparison.right)?;
                match (left, right) {
                    (Ok(left), Ok(right)) => verdict(
                        comparison.op.holds(left, right),
                        json!({ "left": number_json(left), "right": number_json(right) }),
                    ),
                    (Err(missing), _) | (_, Err(missing)) => Err(mismatch(json!(missing))),
                }
            }
            Check::PanesWith(panes) => {
                let wanted = targets::Wanted::of(&panes.target)
                    .map_err(|error| error.message().to_owned())?;
                let scene = state::scan(input.window);
                let mut owners = Vec::new();
                for (index, pane) in scene.observation.panes.iter().enumerate() {
                    let Some(widgets) = scene.pane_widgets_at(index) else {
                        continue;
                    };
                    match wanted.find(&widgets.pane) {
                        Ok(_) => owners.push(pane.name.clone()),
                        Err(LookupError::Missing(_)) => {}
                        Err(LookupError::Broken(error)) => return Err(error),
                    }
                }
                verdict(panes.names.matches(&owners), json!(owners))
            }
            Check::Name(check) | Check::Text(check) => {
                let found = if check.rendered {
                    locate(&check.target)?.map(|located| located.widget)
                } else {
                    targets::presented_target(
                        input.window,
                        input.injector,
                        Kept {
                            memory: &input.memory.borrow(),
                            widgets: &input.widgets.borrow(),
                            boxes: &input.boxes.borrow(),
                        },
                        &check.target,
                    )
                    .map_err(|error| error.message().to_owned())?
                };
                match found {
                    None => Err(mismatch(json!("not found"))),
                    Some(widget) => {
                        let shown = if matches!(predicate.check, Check::Text(_)) {
                            targets::text_content(&widget).unwrap_or_default()
                        } else {
                            targets::accessible_name(&widget)
                        };
                        verdict(check.text.matches(&shown), json!(shown))
                    }
                }
            }
            Check::LargestImage(largest) => match locate(&largest.within)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let mut boxes = Vec::new();
                    for widget in targets::presented(&located.widget) {
                        if targets::role_name(&widget) != "image"
                            || state::rendered_bounds(&widget).is_none()
                        {
                            continue;
                        }
                        let [_, _, width, height] = targets::screen_rect(&widget, input.injector)
                            .map_err(|error| error.message().to_owned())?
                            .pixels();
                        if width > largest.wider_than {
                            boxes.push([width, height]);
                        }
                    }
                    let biggest = boxes.iter().copied().reduce(|best, next| {
                        if next[0] * next[1] > best[0] * best[1] {
                            next
                        } else {
                            best
                        }
                    });
                    let holds = match (biggest, largest.size, largest.exceeds) {
                        (None, _, _) => false,
                        (Some(found), Some(size), _) => found == size,
                        (Some([width, height]), None, Some([most_width, most_height])) => {
                            width > most_width || height > most_height
                        }
                        (Some(_), None, None) => true,
                    };
                    verdict(holds, json!(boxes))
                }
            },
            Check::Children(children) => {
                let found = child_processes(&children.comm_prefix)?;
                verdict(found.len() == children.count, json!(found))
            }
            Check::ListSections(sections) => match locate(&sections.within)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let grouped = list_sections(&located.widget);
                    let rows = grouped
                        .iter()
                        .find(|(heading, _)| *heading == sections.section)
                        .map(|(_, rows)| rows.clone())
                        .unwrap_or_default();
                    verdict(sections.names.matches(&rows), json!(grouped))
                }
            },
            Check::EditableText(text) => {
                let shown = targets::editable_texts(input.window);
                verdict(shown.iter().any(|shown| shown == text), json!(shown))
            }
            Check::VisibleTop(names) => {
                let visible = visible_top(input)?;
                verdict(names.matches(&visible), json!(visible))
            }
            Check::SomeWindow(some) => {
                let mut actual = Vec::new();
                for window in targets::windows_titled(&some.title) {
                    let inside = Input {
                        window: &window,
                        ..*input
                    };
                    match self.check(&inside, &some.holds)? {
                        Ok(()) => return Ok(Ok(())),
                        Err(failure) => actual.push(failure.actual),
                    }
                }
                Err(mismatch(json!(actual)))
            }
            Check::Windows(count) => {
                let found = targets::windows_titled(&count.title).len();
                verdict(found == count.count, json!(found))
            }
            Check::ChildNames(check) => match input.resolve(&check.of)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(parent) => {
                    let mut names = Vec::new();
                    let mut child = parent.first_child();
                    while let Some(current) = child {
                        child = current.next_sibling();
                        if current.is_visible() && targets::role_name(&current) == check.role {
                            names.push(targets::accessible_name(&current));
                        }
                    }
                    verdict(check.names.matches(&names), json!(names))
                }
            },
            Check::FocusOutside(target) => {
                let surface = input.resolve(target)?;
                let focused = input.focused_widget();
                let outside = focused.as_ref().is_some_and(|node| {
                    !targets::accessible_name(node).is_empty()
                        && surface
                            .as_ref()
                            .is_none_or(|surface| node != surface && !node.is_ancestor(surface))
                });
                verdict(
                    outside,
                    json!(focused.as_ref().map(targets::accessible_name)),
                )
            }
            Check::FocusedWidgetIs(target) => {
                let wanted = input.resolve(target)?;
                let focused = input.focused_widget();
                verdict(
                    focused.is_some() && focused == wanted,
                    json!(focused.as_ref().map(targets::accessible_name)),
                )
            }
            Check::Alive(name) => {
                let widget = input.widgets.borrow().get(name).cloned();
                let field = input.memory.borrow().get(name).cloned();
                let kept = widget
                    .or(field)
                    .ok_or_else(|| format!("no widget was remembered as {name:?}"))?;
                let alive = kept.upgrade().is_some_and(|widget| widget.root().is_some());
                verdict(alive, json!(alive))
            }
            Check::SubtreeText(check) => match input.resolve(&check.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(root) => {
                    let mut text = targets::presented(&root)
                        .iter()
                        .filter_map(targets::text_content)
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if check.strip_newlines {
                        text = text.replace('\n', "");
                    }
                    verdict(text.contains(&check.contains), json!(text))
                }
            },
            _ => state::scan(input.window).observation.evaluate(predicate),
        })
    }
}
