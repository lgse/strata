// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::{
    backend::Gtk,
    protocol::{self, Scope},
    state, targets,
};

pub(super) fn theme_accent(backend: &Gtk) -> Result<[u8; 3], String> {
    let preferences = protocol::preferences(backend)?;
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

pub(super) fn footer_fill(
    scope: &Scope<'_, Gtk>,
    fill: &protocol::FooterFill,
) -> Result<Result<(), Value>, String> {
    let scene = scope.scene();
    let (Some(pane), Some(widgets)) = (scene.observation.pane(None), scene.pane(None)) else {
        return Ok(Err(json!("no pane")));
    };
    let window = protocol::screen_rect(scope.backend, &scope.window)
        .map_err(|error| error.message().to_owned())?
        .pixels();
    let ends: Vec<(&protocol::EntryObservation, &gtk::Widget)> = pane
        .entries
        .iter()
        .zip(&widgets.entries)
        .filter(|(entry, _)| fill.ends.contains(&entry.name))
        .collect();
    let ends_ready = ends.len() == fill.ends.len()
        && ends.iter().all(|(entry, widget)| {
            !entry.selected
                && protocol::screen_rect(scope.backend, widget)
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
    let found = targets::presented(&scope.window).iter().any(|widget| {
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
