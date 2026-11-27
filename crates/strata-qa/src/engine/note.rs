// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::{fields::editable_fields, locate::viewport, scope::Scope};
use crate::{Accessor, Backend, Shape, middle_visible};

impl<B: Backend> Scope<'_, B> {
    /// Reads what a `note` names from the snapshot.
    pub(super) fn note(
        &self,
        value: Accessor,
        directory: Option<&str>,
        shape: &Shape<'_>,
        unless_first: Option<&str>,
    ) -> Result<Value, String> {
        let backend = self.backend;
        let scene = self.scene();
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
                let bounds = |node: &B::Node| {
                    backend
                        .layout(node, &self.window)
                        .map(|bounds| bounds.pixels())
                };
                let (Some(pane), Some(nodes)) = (observation.pane(None), scene.pane(None)) else {
                    return Ok(Value::Null);
                };
                let viewport = nodes
                    .container
                    .as_ref()
                    .and_then(|container| viewport(backend, container));
                let Some(viewport) = viewport.as_ref().and_then(bounds) else {
                    return Ok(Value::Null);
                };
                let entries: Vec<(String, [i64; 4])> = pane
                    .entries
                    .iter()
                    .zip(&nodes.entries)
                    .filter_map(|(entry, node)| Some((entry.name.clone(), bounds(node)?)))
                    .collect();
                Ok(json!(middle_visible(
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
            Accessor::EditableFields => Ok(json!(editable_fields(backend, &self.window))),
            Accessor::MenuItems => list(observation.menu.clone().unwrap_or_default()),
        }
    }
}
