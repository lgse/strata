// SPDX-License-Identifier: MIT

use gtk::prelude::*;
use serde_json::{Value, json};

use super::{
    driver::Input,
    protocol::{self, Expr, Measure, Target},
    state,
    targets::{self, Aim, LookupError},
};

pub(super) fn child_processes(prefix: &str) -> Result<Vec<(u32, String)>, String> {
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

pub(super) fn list_sections(root: &gtk::Widget) -> Vec<(String, Vec<String>)> {
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for row in targets::presented(root) {
        if targets::role_name(&row) != "list item" || state::rendered_bounds(&row).is_none() {
            continue;
        }
        if !targets::states(&row).selectable {
            let mut child = row.first_child();
            let mut heading = String::new();
            while let Some(current) = child {
                child = current.next_sibling();
                if !current.is_visible() {
                    continue;
                }
                let name = targets::accessible_name(&current);
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
            rows.push(targets::accessible_name(&row));
        }
    }
    sections
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "only whole numbers well inside the i64 range are converted"
)]
pub(super) fn number_json(number: f64) -> Value {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        json!(number as i64)
    } else {
        json!(number)
    }
}

/// Outer error: a broken lookup or malformed number; inner: not there yet, so a wait keeps polling.
pub(super) fn evaluate(input: &Input<'_>, expr: &Expr) -> Result<Result<f64, String>, String> {
    let pair = |items: &[Expr; 2]| -> Result<Result<(f64, f64), String>, String> {
        Ok(
            match (evaluate(input, &items[0])?, evaluate(input, &items[1])?) {
                (Ok(left), Ok(right)) => Ok((left, right)),
                (Err(missing), _) | (_, Err(missing)) => Err(missing),
            },
        )
    };
    Ok(match expr {
        Expr::Number(number) => Ok(number.as_f64().unwrap_or_default()),
        Expr::Of { target, get } => match input.locate_now(target, None, Aim::Point) {
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
                    Measure::Value => located
                        .widget
                        .downcast_ref::<gtk::Range>()
                        .map(|range| range.value())
                        .ok_or_else(|| "the target has no numeric value".to_owned()),
                }
            }
        },
        Expr::Pref { key, default } => match preferences()?.get(key) {
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
                None => input.window.clone().upcast::<gtk::Widget>(),
                Some(within) => match input.locate_now(within, None, Aim::Point) {
                    Err(LookupError::Broken(error)) => return Err(error),
                    Err(LookupError::Missing(missing)) => return Ok(Err(missing)),
                    Ok(located) => located.widget,
                },
            };
            #[expect(
                clippy::cast_precision_loss,
                reason = "a widget count is far below 2^52"
            )]
            let count = targets::presented(&scope)
                .iter()
                .filter(|widget| {
                    spec.role
                        .as_deref()
                        .is_none_or(|role| targets::role_name(widget) == role)
                        && spec
                            .name
                            .as_deref()
                            .is_none_or(|name| targets::accessible_name(widget) == name)
                        && spec.description.as_deref().is_none_or(|description| {
                            targets::accessible_description(widget) == description
                        })
                        && (!spec.rendered || state::rendered_bounds(widget).is_some())
                        && spec
                            .states
                            .iter()
                            .all(|wanted| targets::has_state(widget, *wanted))
                })
                .count() as f64;
            Ok(count)
        }
        Expr::Sum(items) => {
            let mut total = 0.0;
            for item in items {
                match evaluate(input, item)? {
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
        Expr::Ceil(inner) => evaluate(input, inner)?.map(f64::ceil),
        Expr::Abs(inner) => evaluate(input, inner)?.map(f64::abs),
    })
}

pub(super) fn visible_top(input: &Input<'_>) -> Result<Vec<String>, String> {
    let scene = state::scan(input.window);
    let (Some(pane), Some(widgets)) = (scene.observation.pane(None), scene.pane_widgets(None))
    else {
        return Ok(Vec::new());
    };
    let Some(viewport) = widgets.container.as_ref().and_then(|container| {
        std::iter::successors(container.parent(), WidgetExt::parent)
            .find(|widget| widget.is::<gtk::ScrolledWindow>())
    }) else {
        return Ok(Vec::new());
    };
    let screen = |widget: &gtk::Widget| {
        targets::screen_rect(widget, input.injector).map(|rect| rect.pixels())
    };
    let Ok([_, top, _, height]) = screen(&viewport) else {
        return Ok(Vec::new());
    };
    Ok(pane
        .entries
        .iter()
        .zip(&widgets.entries)
        .filter(|(_, widget)| screen(widget).is_ok_and(|[_, y, _, _]| top <= y && y < top + height))
        .map(|(entry, _)| entry.name.clone())
        .collect())
}

pub(super) fn theme_accent() -> Result<[u8; 3], String> {
    let preferences = preferences()?;
    let theme = preferences
        .get("theme")
        .and_then(Value::as_str)
        .ok_or("settings.toml names no theme")?;
    catalog_accent(include_str!("../../data/themes/catalog.toml"), theme)
}

pub(super) fn catalog_accent(catalog: &str, theme: &str) -> Result<[u8; 3], String> {
    let catalog: toml::Table = toml::from_str(catalog).map_err(|error| error.to_string())?;
    let accent = catalog
        .get("themes")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .find(|entry| entry.get("id").and_then(toml::Value::as_str) == Some(theme))
        .and_then(|entry| entry.get("accent"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("the theme catalog has no accent for {theme:?}"))?;
    let hex = accent
        .strip_prefix('#')
        .filter(|hex| hex.len() == 6)
        .ok_or_else(|| format!("{accent:?} is not a #rrggbb colour"))?;
    let channel = |index: usize| {
        u8::from_str_radix(&hex[index..index + 2], 16).map_err(|error| error.to_string())
    };
    Ok([channel(0)?, channel(2)?, channel(4)?])
}

pub(super) fn hit_path(window: &gtk::Window, hit: &protocol::HitPath) -> Option<String> {
    let mark = targets::presented(window.upcast_ref())
        .into_iter()
        .find(|widget| {
            targets::role_name(widget) == "label"
                && targets::accessible_name(widget) == hit.mark
                && state::rendered_bounds(widget).is_some()
        })?;
    let parent = mark.parent()?;
    let mut part = parent.first_child();
    while let Some(current) = part {
        part = current.next_sibling();
        if !current.is_visible() || targets::role_name(&current) != "panel" {
            continue;
        }
        let mut text = String::new();
        let mut label = current.first_child();
        while let Some(child) = label {
            label = child.next_sibling();
            if child.is_visible()
                && targets::role_name(&child) == "label"
                && state::rendered_bounds(&child).is_some()
            {
                text.push_str(&targets::accessible_name(&child));
            }
        }
        if hit.hits.values().any(|path| *path == text) {
            return Some(text);
        }
    }
    None
}

pub(super) struct VisibleRow {
    pub(super) name: String,
    pub(super) selected: bool,
    pub(super) name_box: [i64; 4],
}

pub(super) fn visible_rows(
    input: &Input<'_>,
    directory: Option<&str>,
) -> Result<Vec<VisibleRow>, String> {
    let scene = state::scan(input.window);
    let (Some(pane), Some(widgets)) = (
        scene.observation.pane(directory),
        scene.pane_widgets(directory),
    ) else {
        return Ok(Vec::new());
    };
    let Some(viewport) = widgets.container.as_ref().and_then(|container| {
        std::iter::successors(container.parent(), WidgetExt::parent)
            .find(|widget| widget.is::<gtk::ScrolledWindow>())
    }) else {
        return Ok(Vec::new());
    };
    let screen = |widget: &gtk::Widget| {
        targets::screen_rect(widget, input.injector).map(|rect| rect.pixels())
    };
    let Ok([_, top, _, height]) = screen(&viewport) else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for (entry, widget) in pane.entries.iter().zip(&widgets.entries) {
        let Ok([_, y, _, row_height]) = screen(widget) else {
            continue;
        };
        if row_height <= 0 || y < top || y + row_height > top + height {
            continue;
        }
        let label = targets::presented(widget)
            .into_iter()
            .find(|label| targets::role_name(label) == "label");
        let name_box = label
            .as_ref()
            .and_then(|label| screen(label).ok())
            .map_or_else(|| screen(widget), Ok)
            .map_err(|error| error.message().to_owned())?;
        rows.push(VisibleRow {
            name: entry.name.clone(),
            selected: entry.selected,
            name_box,
        });
    }
    Ok(rows)
}

pub(super) fn scroll_value(
    input: &Input<'_>,
    directory: Option<&str>,
) -> Result<Option<f64>, String> {
    Ok(pane_scroll(input, directory)?.map(|(_, scrollbar)| scrollbar.adjustment().value()))
}

fn pane_scroll(
    input: &Input<'_>,
    directory: Option<&str>,
) -> Result<Option<(gtk::Widget, gtk::Scrollbar)>, String> {
    let scene = state::scan(input.window);
    let Some(viewport) = scene
        .pane_widgets(directory)
        .and_then(|widgets| widgets.container.clone())
        .and_then(|container| {
            std::iter::successors(container.parent(), WidgetExt::parent)
                .find(|widget| widget.is::<gtk::ScrolledWindow>())
        })
    else {
        return Ok(None);
    };
    let scrollbar = targets::presented(&viewport)
        .into_iter()
        .find_map(|widget| {
            state::rendered_bounds(&widget)?;
            widget.downcast::<gtk::Scrollbar>().ok()
        });
    Ok(scrollbar.map(|scrollbar| (viewport, scrollbar)))
}

pub(super) fn scroll_target(
    input: &Input<'_>,
    of: &Target,
    directory: Option<&str>,
) -> Result<Value, String> {
    let Some((viewport, scrollbar)) = pane_scroll(input, directory)? else {
        return Err("the pane has no scroll bar showing".to_owned());
    };
    let anchor = input
        .locate_now(of, None, Aim::Point)
        .map_err(|error| error.message().to_owned())?
        .rect
        .pixels();
    let [_, top, ..] = targets::screen_rect(&viewport, input.injector)
        .map_err(|error| error.message().to_owned())?
        .pixels();
    #[expect(
        clippy::cast_precision_loss,
        reason = "screen coordinates are far below 2^52"
    )]
    let past = (anchor[1] + anchor[3] - top + 1).max(1) as f64;
    Ok(json!(scrollbar.adjustment().value() + past))
}

pub(super) fn footer_fill(
    input: &Input<'_>,
    fill: &protocol::FooterFill,
) -> Result<Result<(), Value>, String> {
    let scene = state::scan(input.window);
    let (Some(pane), Some(widgets)) = (scene.observation.pane(None), scene.pane_widgets(None))
    else {
        return Ok(Err(json!("no pane")));
    };
    let window = targets::screen_rect(input.window.upcast_ref(), input.injector)
        .map_err(|error| error.message().to_owned())?
        .pixels();
    let ends: Vec<(&state::EntryObservation, &gtk::Widget)> = pane
        .entries
        .iter()
        .zip(&widgets.entries)
        .filter(|(entry, _)| fill.ends.contains(&entry.name))
        .collect();
    let ends_ready = ends.len() == fill.ends.len()
        && ends.iter().all(|(entry, widget)| {
            !entry.selected
                && targets::screen_rect(widget, input.injector)
                    .is_ok_and(|rect| state::on_screen(rect.pixels(), window))
        });
    if !ends_ready {
        return Ok(Err(
            json!({ "ends": ends.iter().map(|(entry, _)| (&entry.name, entry.selected)).collect::<Vec<_>>() }),
        ));
    }
    let cleared = pane.entries.iter().filter(|entry| !entry.selected).count();
    let remaining = format!(
        "{} of {} items selected",
        fill.total.saturating_sub(cleared),
        fill.total
    );
    let found = targets::presented(input.window.upcast_ref())
        .iter()
        .any(|widget| {
            targets::role_name(widget) == "label"
                && state::rendered_bounds(widget).is_some()
                && crate::ui::accessibility_qa::naming(widget)
                    .and_then(|naming| naming.description)
                    .is_some_and(|description| description.starts_with(&remaining))
        });
    Ok(if found {
        Ok(())
    } else {
        Err(json!({ "wanted": remaining }))
    })
}

/// Runs a check's own disk read on another thread and waits for it, so a
/// main-thread I/O audit, which traces only the UI thread, sees no QA reads.
pub(super) fn off_ui_thread<T: Send>(read: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(read)
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

pub(super) fn preferences() -> Result<Value, String> {
    let path = crate::storage::config_directory().join("settings.toml");
    let text = match off_ui_thread(|| std::fs::read_to_string(&path)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let table: toml::Table = toml::from_str(&text)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    serde_json::to_value(table).map_err(|error| error.to_string())
}
