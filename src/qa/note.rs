// SPDX-License-Identifier: MIT

use gtk::prelude::*;
use serde_json::{Value, json};

use super::{
    protocol::{Accessor, Shape},
    state, targets,
};

pub(super) fn note(
    window: &gtk::Window,
    value: Accessor,
    directory: Option<&str>,
    shape: &Shape<'_>,
    unless_first: Option<&str>,
) -> Result<Value, String> {
    let scene = state::scan(window);
    let observation = &scene.observation;
    let list = |names: Vec<String>| shape.apply(names);
    match value {
        Accessor::Directory => Ok(json!(observation.directory())),
        Accessor::Pane => Ok(json!(observation.panes.last().map(|pane| &pane.name))),
        Accessor::Panes => list(
            observation
                .panes
                .iter()
                .map(|pane| pane.name.clone())
                .collect(),
        ),
        Accessor::Entries => list(observation.entry_names(directory)),
        Accessor::Selected => list(observation.selected_names(directory)),
        Accessor::AllSelected => list(observation.all_selected()),
        Accessor::Matches => list(observation.matches(directory)),
        Accessor::Focused => Ok(json!(observation.focused_entry)),
        Accessor::FocusedPane => Ok(json!(observation.focused_pane_name())),
        Accessor::MiddleVisible => {
            let root = window.upcast_ref::<gtk::Widget>();
            let bounds = |widget: &gtk::Widget| {
                widget.compute_bounds(root).map(|bounds| {
                    [bounds.x(), bounds.y(), bounds.width(), bounds.height()]
                        .map(|value| f64::from(value).floor() as i64)
                })
            };
            let (Some(pane), Some(widgets)) = (observation.pane(None), scene.pane_widgets(None))
            else {
                return Ok(Value::Null);
            };
            let viewport = widgets.container.as_ref().and_then(|container| {
                std::iter::successors(container.parent(), WidgetExt::parent)
                    .find(|widget| widget.is::<gtk::ScrolledWindow>())
            });
            let Some(viewport) = viewport.as_ref().and_then(bounds) else {
                return Ok(Value::Null);
            };
            let entries: Vec<(String, [i64; 4])> = pane
                .entries
                .iter()
                .zip(&widgets.entries)
                .filter_map(|(entry, widget)| Some((entry.name.clone(), bounds(widget)?)))
                .collect();
            Ok(json!(state::middle_visible(
                &entries,
                viewport,
                unless_first.unwrap_or_default()
            )))
        }
        Accessor::Visible
        | Accessor::ScrollTarget
        | Accessor::Bounds
        | Accessor::Number
        | Accessor::VisibleTop => Err(
            "visible, visible_top, scroll_target, bounds, and number are read with the X screen"
                .to_owned(),
        ),
        Accessor::EditableFields => Ok(json!(targets::editable_fields(window))),
        Accessor::MenuItems => list(observation.menu.clone().unwrap_or_default()),
    }
}
