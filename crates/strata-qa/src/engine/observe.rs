// SPDX-License-Identifier: MIT

//! Reads that predicates and notes share: rows in view, numbers, child processes, preferences.

use serde_json::{Value, json};

use super::{
    locate::{screen_rect, viewport},
    lookup::{presented, rendered},
    scope::Scope,
    wanted::has_state,
};
use crate::{Aim, Backend, Expr, HitPath, LookupError, Measure, Target};

/// The state of each direct child of this process whose command name starts with `prefix`.
pub(crate) fn child_processes(prefix: &str) -> Result<Vec<(u32, String)>, String> {
    let parent = std::process::id();
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/proc").map_err(|error| format!("/proc: {error}"))? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // The command name sits in parentheses and may hold spaces.
        let (Some(open), Some(close)) = (stat.find(" ("), stat.rfind(") ")) else {
            continue;
        };
        let comm = &stat[open + 2..close];
        let mut rest = stat[close + 2..].split_whitespace();
        let (Some(state), Some(ppid)) = (rest.next(), rest.next()) else {
            continue;
        };
        if ppid.parse::<u32>().ok() == Some(parent) && comm.starts_with(prefix) {
            found.push((pid, state.to_owned()));
        }
    }
    Ok(found)
}

/// A list's rows under the heading each follows. A row that cannot be selected is a heading,
/// named by its first named child.
pub(crate) fn list_sections<B: Backend>(backend: &B, root: &B::Node) -> Vec<(String, Vec<String>)> {
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for row in presented(backend, root) {
        if backend.role(&row) != "list item" || !rendered(backend, &row) {
            continue;
        }
        if !backend.states(&row).selectable {
            let mut heading = String::new();
            for child in backend.children(&row) {
                if !backend.visible(&child) {
                    continue;
                }
                let name = backend.name(&child);
                if !name.is_empty() {
                    heading = name;
                    break;
                }
            }
            if !sections.iter().any(|(existing, _)| *existing == heading) {
                sections.push((heading.clone(), Vec::new()));
            }
            let index = sections
                .iter()
                .position(|(existing, _)| *existing == heading)
                .unwrap_or_default();
            let section = sections.remove(index);
            sections.push(section);
        } else if let Some((_, rows)) = sections.last_mut() {
            rows.push(backend.name(&row));
        }
    }
    sections
}

/// The path shown beside a search mark, when it is one of the hits.
pub(crate) fn hit_path<B: Backend>(backend: &B, window: &B::Node, hit: &HitPath) -> Option<String> {
    let mark = presented(backend, window).into_iter().find(|node| {
        backend.role(node) == "label" && backend.name(node) == hit.mark && rendered(backend, node)
    })?;
    let parent = backend.parent(&mark)?;
    for part in backend.children(&parent) {
        if !backend.visible(&part) || backend.role(&part) != "panel" {
            continue;
        }
        let mut text = String::new();
        for child in backend.children(&part) {
            if backend.visible(&child)
                && backend.role(&child) == "label"
                && rendered(backend, &child)
            {
                text.push_str(&backend.name(&child));
            }
        }
        if hit.hits.values().any(|path| *path == text) {
            return Some(text);
        }
    }
    None
}

pub struct VisibleRow {
    pub name: String,
    pub selected: bool,
    pub name_box: [i64; 4],
}

/// Runs a check's own disk read on another thread and waits for it, so a
/// main-thread I/O audit, which traces only the UI thread, sees no QA reads.
pub fn off_ui_thread<T: Send>(read: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(read)
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// The application's saved preferences as JSON; an absent file is no preferences.
pub fn preferences<B: Backend + ?Sized>(backend: &B) -> Result<Value, String> {
    let path = backend.settings_file();
    let text = match off_ui_thread(|| std::fs::read_to_string(&path)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let table: toml::Table = toml::from_str(&text)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    serde_json::to_value(table).map_err(|error| error.to_string())
}

impl<B: Backend> Scope<'_, B> {
    /// Outer error: a broken lookup or malformed number; inner: not there yet, so a wait keeps polling.
    pub fn evaluate(&self, expr: &Expr) -> Result<Result<f64, String>, String> {
        let pair = |items: &[Expr; 2]| -> Result<Result<(f64, f64), String>, String> {
            Ok(
                match (self.evaluate(&items[0])?, self.evaluate(&items[1])?) {
                    (Ok(left), Ok(right)) => Ok((left, right)),
                    (Err(missing), _) | (_, Err(missing)) => Err(missing),
                },
            )
        };
        let backend = self.backend;
        Ok(match expr {
            Expr::Number(number) => Ok(number.as_f64().unwrap_or_default()),
            Expr::Of { target, get } => match self.locate_now(target, None, Aim::Point) {
                Err(LookupError::Broken(error)) => return Err(error),
                Err(LookupError::Missing(missing)) => Err(missing),
                Ok(located) => {
                    let [x, y, width, height] = located.rect.pixels();
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "screen coordinates are far below 2^52"
                    )]
                    let pixels = |value: i64| Ok(value as f64);
                    match get {
                        Measure::X => pixels(x),
                        Measure::Y => pixels(y),
                        Measure::Width => pixels(width),
                        Measure::Height => pixels(height),
                        Measure::Right => pixels(x + width),
                        Measure::Bottom => pixels(y + height),
                        Measure::CenterX => pixels(x + width.div_euclid(2)),
                        Measure::CenterY => pixels(y + height.div_euclid(2)),
                        Measure::Value => backend
                            .value(&located.node)
                            .ok_or_else(|| "the target has no numeric value".to_owned()),
                    }
                }
            },
            Expr::Pref { key, default } => match preferences(backend)?.get(key) {
                Some(Value::Number(number)) => Ok(number.as_f64().unwrap_or_default()),
                Some(Value::String(text)) => text
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| format!("{key} is {text:?}, not a number")),
                Some(other) => Err(format!("{key} is {other}, not a number")),
                None => default
                    .as_ref()
                    .and_then(serde_json::Number::as_f64)
                    .ok_or_else(|| format!("settings.toml has no {key}")),
            },
            Expr::Count(spec) => {
                let scope = match &spec.within {
                    None => self.window.clone(),
                    Some(within) => match self.locate_now(within, None, Aim::Point) {
                        Err(LookupError::Broken(error)) => return Err(error),
                        Err(LookupError::Missing(missing)) => return Ok(Err(missing)),
                        Ok(located) => located.node,
                    },
                };
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a widget count is far below 2^52"
                )]
                let count = presented(backend, &scope)
                    .iter()
                    .filter(|node| {
                        spec.role
                            .as_deref()
                            .is_none_or(|role| backend.role(node) == role)
                            && spec
                                .name
                                .as_deref()
                                .is_none_or(|name| backend.name(node) == name)
                            && spec
                                .description
                                .as_deref()
                                .is_none_or(|description| backend.description(node) == description)
                            && (!spec.rendered || rendered(backend, node))
                            && spec
                                .states
                                .iter()
                                .all(|wanted| has_state(backend, node, *wanted))
                    })
                    .count() as f64;
                Ok(count)
            }
            Expr::Sum(items) => {
                let mut total = 0.0;
                for item in items {
                    match self.evaluate(item)? {
                        Ok(value) => total += value,
                        Err(missing) => return Ok(Err(missing)),
                    }
                }
                Ok(total)
            }
            Expr::Minus(items) => pair(items)?.map(|(left, right)| left - right),
            Expr::Times(items) => pair(items)?.map(|(left, right)| left * right),
            Expr::Divide(items) => pair(items)?.and_then(|(left, right)| {
                if right == 0.0 {
                    Err("division by zero".to_owned())
                } else {
                    Ok(left / right)
                }
            }),
            Expr::FloorDivide(items) => pair(items)?.and_then(|(left, right)| {
                if right == 0.0 {
                    Err("division by zero".to_owned())
                } else {
                    Ok((left / right).floor())
                }
            }),
            Expr::Ceil(inner) => self.evaluate(inner)?.map(f64::ceil),
            Expr::Abs(inner) => self.evaluate(inner)?.map(f64::abs),
        })
    }

    /// The entries of the deepest pane whose top edge is inside its scroll pane.
    pub fn visible_top(&self) -> Result<Vec<String>, String> {
        let backend = self.backend;
        let scene = self.scene();
        let (Some(pane), Some(nodes)) = (scene.observation.pane(None), scene.pane(None)) else {
            return Ok(Vec::new());
        };
        let Some(viewport) = nodes
            .container
            .as_ref()
            .and_then(|container| viewport(backend, container))
        else {
            return Ok(Vec::new());
        };
        let screen = |node: &B::Node| screen_rect(backend, node).map(|rect| rect.pixels());
        let Ok([_, top, _, height]) = screen(&viewport) else {
            return Ok(Vec::new());
        };
        Ok(pane
            .entries
            .iter()
            .zip(&nodes.entries)
            .filter(|(_, node)| screen(node).is_ok_and(|[_, y, _, _]| top <= y && y < top + height))
            .map(|(entry, _)| entry.name.clone())
            .collect())
    }

    /// The entries of a pane that lie wholly inside its scroll pane.
    pub fn visible_rows(&self, directory: Option<&str>) -> Result<Vec<VisibleRow>, String> {
        let backend = self.backend;
        let scene = self.scene();
        let (Some(pane), Some(nodes)) = (scene.observation.pane(directory), scene.pane(directory))
        else {
            return Ok(Vec::new());
        };
        let Some(viewport) = nodes
            .container
            .as_ref()
            .and_then(|container| viewport(backend, container))
        else {
            return Ok(Vec::new());
        };
        let screen = |node: &B::Node| screen_rect(backend, node).map(|rect| rect.pixels());
        let Ok([_, top, _, height]) = screen(&viewport) else {
            return Ok(Vec::new());
        };
        let mut rows = Vec::new();
        for (entry, node) in pane.entries.iter().zip(&nodes.entries) {
            let Ok([_, y, _, row_height]) = screen(node) else {
                continue;
            };
            if row_height <= 0 || y < top || y + row_height > top + height {
                continue;
            }
            let label = presented(backend, node)
                .into_iter()
                .find(|label| backend.role(label) == "label");
            let name_box = label
                .as_ref()
                .and_then(|label| screen(label).ok())
                .map_or_else(|| screen(node), Ok)
                .map_err(|error| error.message().to_owned())?;
            rows.push(VisibleRow {
                name: entry.name.clone(),
                selected: entry.selected,
                name_box,
            });
        }
        Ok(rows)
    }

    /// A pane's scroll pane and the position of the scroll bar it shows.
    fn pane_scroll(&self, directory: Option<&str>) -> Option<(B::Node, f64)> {
        let viewport = self
            .scene()
            .pane(directory)
            .and_then(|nodes| nodes.container.clone())
            .and_then(|container| viewport(self.backend, &container))?;
        let position = self.backend.scroll_position(&viewport)?;
        Some((viewport, position))
    }

    pub fn scroll_value(&self, directory: Option<&str>) -> Option<f64> {
        self.pane_scroll(directory).map(|(_, position)| position)
    }

    /// The scroll position that moves `of` just out of the top of its pane.
    pub fn scroll_target(&self, of: &Target, directory: Option<&str>) -> Result<Value, String> {
        let Some((viewport, position)) = self.pane_scroll(directory) else {
            return Err("the pane has no scroll bar showing".to_owned());
        };
        let anchor = self
            .locate_now(of, None, Aim::Point)
            .map_err(|error| error.message().to_owned())?
            .rect
            .pixels();
        let [_, top, ..] = screen_rect(self.backend, &viewport)
            .map_err(|error| error.message().to_owned())?
            .pixels();
        #[expect(
            clippy::cast_precision_loss,
            reason = "screen coordinates are far below 2^52"
        )]
        let past = (anchor[1] + anchor[3] - top + 1).max(1) as f64;
        Ok(json!(position + past))
    }
}
