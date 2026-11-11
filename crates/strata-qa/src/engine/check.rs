// SPDX-License-Identifier: MIT

//! Decides whether a predicate holds against the live window.

use std::path::Path;

use serde_json::{Value, json};

use super::{
    fields::{editable_fields, editable_texts},
    locate::screen_rect,
    lookup::{is_ancestor, presented, rendered, result_rows, windows_titled},
    observe::{child_processes, hit_path, list_sections, off_ui_thread, preferences},
    scope::Scope,
    wanted::Wanted,
};
use crate::{
    Aim, Backend, Check, Direction, Failure, FieldRef, LookupError, PointAxis, Predicate, Target,
    collapse_whitespace, contains, field_matches, inside, number_json, on_screen, within_viewport,
    within_window,
};

/// Outer error: a broken lookup, which waiting cannot fix; inner: the predicate not holding.
pub type Verdict = Result<Result<(), Failure>, String>;

/// The failure of a predicate that does not hold, with what was found instead.
pub fn mismatch(predicate: &Predicate, actual: Value) -> Failure {
    Failure {
        predicate: serde_json::to_value(predicate).unwrap_or(Value::Null),
        actual,
        message: None,
    }
}

/// What a predicate is called in a scenario: the key that is not its `dir`.
fn predicate_name(predicate: &Predicate) -> String {
    serde_json::to_value(predicate)
        .ok()
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|fields| fields.keys().find(|key| *key != "dir").cloned())
        .unwrap_or_else(|| "the predicate".to_owned())
}

impl<B: Backend> Scope<'_, B> {
    pub fn check(&self, predicate: &Predicate) -> Verdict {
        if !predicate.check.is_live() {
            return Ok(self.scene().observation.evaluate(predicate));
        }
        let backend = self.backend;
        let mismatch = |actual: Value| mismatch(predicate, actual);
        let verdict = |holds: bool, actual: Value| {
            if holds { Ok(()) } else { Err(mismatch(actual)) }
        };
        let locate = |target: &Target| match self.locate_now(target, None, Aim::Point) {
            Ok(located) => Ok(Some(located)),
            Err(LookupError::Missing(_)) => Ok(None),
            Err(LookupError::Broken(error)) => Err(error),
        };
        Ok(match &predicate.check {
            Check::Any(predicates) => {
                let mut actual = Vec::new();
                for predicate in predicates {
                    match self.check(predicate)? {
                        Ok(()) => return Ok(Ok(())),
                        Err(failure) => actual.push(failure.actual),
                    }
                }
                Err(mismatch(json!(actual)))
            }
            Check::Field(check) => {
                let field = self
                    .field(&check.field())?
                    .and_then(|field| backend.field(&field));
                verdict(field_matches(check, field.as_ref()), json!(field))
            }
            Check::Pref(check) => {
                let actual = preferences(backend)?
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
                let Some(editor) = self.field(field)? else {
                    return Ok(Err(mismatch(json!("no field"))));
                };
                match self.editor_and_pane(&editor, None) {
                    Some((editor, pane_x, width)) => verdict(
                        within_viewport(editor, pane_x, width),
                        json!({ "editor": editor, "pane_x": pane_x, "window_width": width }),
                    ),
                    None => Err(mismatch(json!("the field or the pane has no bounds"))),
                }
            }
            Check::State(check) => match self.resolve(&check.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(node) => {
                    let states = backend.states(&node);
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
                let Some(before) = self.kept.boxes.borrow().get(&key).copied() else {
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
                    let window = screen_rect(backend, &self.window)
                        .map_err(|error| error.message().to_owned())?
                        .pixels();
                    let bounds = located.rect.pixels();
                    verdict(
                        on_screen(bounds, window),
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
                    for node in presented(backend, &located.node).into_iter().skip(1) {
                        let role = backend.role(&node);
                        if matches!(role, "panel" | "filler" | "group") || !rendered(backend, &node)
                        {
                            continue;
                        }
                        let Ok(rect) = screen_rect(backend, &node) else {
                            continue;
                        };
                        if contains(rect.pixels(), point) {
                            covering.push(json!({
                                "role": role,
                                "name": backend.name(&node),
                            }));
                        }
                    }
                    verdict(covering.is_empty(), json!(covering))
                }
            },
            Check::Not(inner) => match self.check(inner)? {
                Ok(()) => Err(mismatch(json!("the negated predicate holds"))),
                Err(_) => Ok(()),
            },
            Check::Visible(names) => {
                let visible: Vec<String> = self
                    .visible_rows(predicate.dir.as_deref())?
                    .into_iter()
                    .map(|row| row.name)
                    .collect();
                verdict(names.matches(&visible), json!(visible))
            }
            Check::ScrollValue(check) => {
                let value = self.scroll_value(predicate.dir.as_deref());
                let wanted = check.at_least.as_f64().unwrap_or(f64::INFINITY);
                verdict(value.is_some_and(|value| value >= wanted), json!(value))
            }
            Check::BandSelected(band) => {
                let (from, to) = (self.kept_box(&band.from)?, self.kept_box(&band.to)?);
                let [from_x, ..] = from.pixels();
                let [to_x, to_y, ..] = to.pixels();
                let inside: Vec<(String, bool)> = self
                    .visible_rows(predicate.dir.as_deref())?
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
                    let focus = backend.focus(&self.window);
                    let within = focus.as_ref().is_some_and(|focus| {
                        *focus == located.node || is_ancestor(backend, &located.node, focus)
                    });
                    verdict(within, json!(focus.map(|focus| backend.name(&focus))))
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
            Check::All(predicates) => {
                let mut actual = Vec::new();
                let mut holds = true;
                for predicate in predicates {
                    match self.check(predicate)? {
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
                let count = editable_fields(backend, &self.window);
                verdict(count == *expected, json!(count))
            }
            Check::Description(check) => match self.resolve(&check.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(node) => {
                    let description = backend.description(&node);
                    verdict(check.text.matches(&description), json!(description))
                }
            },
            Check::Mentions(mention) => match locate(&mention.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let mut nodes = presented(backend, &located.node);
                    if let Some(role) = &mention.role {
                        nodes.retain(|node| backend.role(node) == role && rendered(backend, node));
                    }
                    let found = match (&mention.text, &mention.words) {
                        _ if mention.equals.is_some() => {
                            let equals = mention.equals.as_deref().unwrap_or_default();
                            nodes
                                .iter()
                                .filter(|node| rendered(backend, node))
                                .any(|node| collapse_whitespace(&backend.name(node)) == equals)
                        }
                        (Some(text), _) => nodes.iter().any(|node| {
                            backend.name(node).contains(text.as_str())
                                || backend.description(node).contains(text.as_str())
                        }),
                        (None, Some(words)) => {
                            let words = collapse_whitespace(words);
                            nodes
                                .iter()
                                .filter(|node| rendered(backend, node))
                                .any(|node| {
                                    let shown = format!(
                                        "{} {}",
                                        backend.name(node),
                                        backend.text(node).unwrap_or_default()
                                    );
                                    collapse_whitespace(&shown).contains(&words)
                                })
                        }
                        (None, None) => false,
                    };
                    verdict(found, json!(false))
                }
            },
            Check::Presented(target) => {
                let found = self.presented(target)?;
                verdict(found.is_some(), json!(false))
            }
            Check::File(file) => match (&file.exists, &file.mode_bits) {
                (_, Some(bits)) => {
                    use std::os::unix::fs::PermissionsExt as _;
                    let mask =
                        u32::from_str_radix(&bits.mask, 8).map_err(|error| error.to_string())?;
                    match off_ui_thread(|| std::fs::metadata(&file.path)) {
                        Ok(metadata) => {
                            let mode = metadata.permissions().mode();
                            verdict((mode & mask != 0) == bits.set, json!(format!("{mode:o}")))
                        }
                        Err(error) => Err(mismatch(json!(error.to_string()))),
                    }
                }
                (exists, None) => {
                    let found = off_ui_thread(|| Path::new(&file.path).symlink_metadata().is_ok());
                    verdict(Some(found) == *exists, json!(found))
                }
            },
            Check::FocusedWidget(expected) => {
                let focused = self.focused().map(|node| backend.name(&node));
                let holds = match (expected, &focused) {
                    (None, None) => true,
                    (Some(expected), Some(focused)) => expected.matches(focused),
                    _ => false,
                };
                verdict(holds, json!(focused))
            }
            Check::Inside(check) => match (locate(&check.target)?, locate(&check.of)?) {
                (Some(target), Some(of)) => {
                    let (inner, outer) = (target.rect.pixels(), of.rect.pixels());
                    verdict(
                        inside(inner, outer, check.axis),
                        json!({ "target": inner, "of": outer }),
                    )
                }
                _ => Err(mismatch(json!("not on screen"))),
            },
            Check::EditorWithinWindow(check) => {
                let Some(editor) = self.field(&FieldRef {
                    of: Some(check.of.clone()),
                    name: None,
                })?
                else {
                    return Ok(Err(mismatch(json!("no field"))));
                };
                match self.editor_and_pane(&editor, check.pane.as_deref()) {
                    Some((editor, pane_x, width)) => verdict(
                        within_window(editor, pane_x, width),
                        json!({ "editor": editor, "pane_x": pane_x, "window_width": width }),
                    ),
                    None => Err(mismatch(json!("the field or the pane has no bounds"))),
                }
            }
            Check::Replaced(replaced) => {
                let Some(kept) = self.kept.nodes.borrow().get(&replaced.since).cloned() else {
                    return Err(if self.kept.fields.borrow().contains_key(&replaced.since) {
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
                    Some(now) => verdict(
                        backend.recall(&kept).as_ref() != Some(&now.node),
                        json!(false),
                    ),
                }
            }
            Check::AnyResultRow(wanted) => {
                let rows: Vec<Value> = result_rows(backend, &self.window)
                    .iter()
                    .map(|row| {
                        let states = backend.states(row);
                        json!([backend.name(row), states.selected, states.focused])
                    })
                    .collect();
                let holds = result_rows(backend, &self.window).iter().any(|row| {
                    let states = backend.states(row);
                    wanted.matches(states.selected, states.focused)
                });
                verdict(holds, json!(rows))
            }
            Check::HitPath(hit) => {
                if !hit.hits.contains_key(&hit.name) {
                    return Err(format!("{:?} is not one of the hits", hit.name));
                }
                let shown = hit_path(backend, &self.window, hit);
                verdict(
                    shown.as_deref() == hit.hits.get(&hit.name).map(String::as_str),
                    json!(shown),
                )
            }
            Check::Compare(comparison) => {
                let left = self.evaluate(&comparison.left)?;
                let right = self.evaluate(&comparison.right)?;
                match (left, right) {
                    (Ok(left), Ok(right)) => verdict(
                        comparison.op.holds(left, right),
                        json!({ "left": number_json(left), "right": number_json(right) }),
                    ),
                    (Err(missing), _) | (_, Err(missing)) => Err(mismatch(json!(missing))),
                }
            }
            Check::PanesWith(panes) => {
                let wanted =
                    Wanted::of(&panes.target).map_err(|error| error.message().to_owned())?;
                let scene = self.scene();
                let mut owners = Vec::new();
                for (index, pane) in scene.observation.panes.iter().enumerate() {
                    let Some(nodes) = scene.pane_at(index) else {
                        continue;
                    };
                    match wanted.find(backend, &nodes.pane) {
                        Ok(_) => owners.push(pane.name.clone()),
                        Err(LookupError::Missing(_)) => {}
                        Err(LookupError::Broken(error)) => return Err(error),
                    }
                }
                verdict(panes.names.matches(&owners), json!(owners))
            }
            Check::Name(check) | Check::Text(check) => {
                let found = if check.rendered {
                    locate(&check.target)?.map(|located| located.node)
                } else {
                    self.presented(&check.target)?
                };
                match found {
                    None => Err(mismatch(json!("not found"))),
                    Some(node) => {
                        let shown = if matches!(predicate.check, Check::Text(_)) {
                            backend.text(&node).unwrap_or_default()
                        } else {
                            backend.name(&node)
                        };
                        verdict(check.text.matches(&shown), json!(shown))
                    }
                }
            }
            Check::LargestImage(largest) => match locate(&largest.within)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let mut boxes = Vec::new();
                    for node in presented(backend, &located.node) {
                        if backend.role(&node) != "image" || !rendered(backend, &node) {
                            continue;
                        }
                        let [_, _, width, height] = screen_rect(backend, &node)
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
                    let grouped = list_sections(backend, &located.node);
                    let rows = grouped
                        .iter()
                        .find(|(heading, _)| *heading == sections.section)
                        .map(|(_, rows)| rows.clone())
                        .unwrap_or_default();
                    verdict(sections.names.matches(&rows), json!(grouped))
                }
            },
            Check::EditableText(text) => {
                let shown = editable_texts(backend, &self.window);
                verdict(shown.iter().any(|shown| shown == text), json!(shown))
            }
            Check::VisibleTop(names) => {
                let visible = self.visible_top()?;
                verdict(names.matches(&visible), json!(visible))
            }
            Check::SomeWindow(some) => {
                let mut actual = Vec::new();
                for window in windows_titled(backend, &some.title) {
                    let inside = Scope {
                        backend,
                        window,
                        kept: self.kept,
                    };
                    match inside.check(&some.holds)? {
                        Ok(()) => return Ok(Ok(())),
                        Err(failure) => actual.push(failure.actual),
                    }
                }
                Err(mismatch(json!(actual)))
            }
            Check::Windows(count) => {
                let found = windows_titled(backend, &count.title).len();
                verdict(found == count.count, json!(found))
            }
            Check::ChildNames(check) => match self.resolve(&check.of)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(parent) => {
                    let names: Vec<String> = backend
                        .children(&parent)
                        .iter()
                        .filter(|child| backend.visible(child) && backend.role(child) == check.role)
                        .map(|child| backend.name(child))
                        .collect();
                    verdict(check.names.matches(&names), json!(names))
                }
            },
            Check::FocusOutside(target) => {
                let surface = self.resolve(target)?;
                let focused = self.focused();
                let outside = focused.as_ref().is_some_and(|node| {
                    !backend.name(node).is_empty()
                        && surface.as_ref().is_none_or(|surface| {
                            node != surface && !is_ancestor(backend, surface, node)
                        })
                });
                verdict(
                    outside,
                    json!(focused.as_ref().map(|node| backend.name(node))),
                )
            }
            Check::FocusedWidgetIs(target) => {
                let wanted = self.resolve(target)?;
                let focused = self.focused();
                verdict(
                    focused.is_some() && focused == wanted,
                    json!(focused.as_ref().map(|node| backend.name(node))),
                )
            }
            Check::Alive(name) => {
                let node = self.kept.nodes.borrow().get(name).cloned();
                let field = self.kept.fields.borrow().get(name).cloned();
                let kept = node
                    .or(field)
                    .ok_or_else(|| format!("no widget was remembered as {name:?}"))?;
                let alive = backend
                    .recall(&kept)
                    .is_some_and(|node| backend.attached(&node));
                verdict(alive, json!(alive))
            }
            Check::SubtreeText(check) => match self.resolve(&check.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(root) => {
                    let mut text = presented(backend, &root)
                        .iter()
                        .filter_map(|node| backend.text(node))
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if check.strip_newlines {
                        text = text.replace('\n', "");
                    }
                    verdict(text.contains(&check.contains), json!(text))
                }
            },
            // What only the toolkit can read: screen pixels, samples it took, and what it
            // records beside the accessible names.
            Check::Pixel(_)
            | Check::AccentPixels(_)
            | Check::Repainted(_)
            | Check::ContrastBelow(_)
            | Check::Elapsed(_)
            | Check::FooterFill(_)
            | Check::DiagnosticsMention(_) => {
                return backend.check(self, predicate).unwrap_or_else(|| {
                    Err(format!(
                        "{} is not supported on this target",
                        predicate_name(predicate)
                    ))
                });
            }
            _ => self.scene().observation.evaluate(predicate),
        })
    }

    /// The editor's box and the left edge of a pane, in whole window pixels, with the
    /// window's width.
    fn editor_and_pane(
        &self,
        editor: &B::Node,
        pane: Option<&str>,
    ) -> Option<([i64; 4], i64, i64)> {
        let backend = self.backend;
        let scene = self.scene();
        let pane = scene
            .pane(pane)
            .and_then(|nodes| backend.layout(&nodes.pane, &self.window));
        let editor = backend.layout(editor, &self.window);
        let (editor, pane) = (editor?, pane?);
        let width = i64::from(backend.window_width(&self.window));
        Some((editor.pixels(), pane.x.floor() as i64, width))
    }
}
