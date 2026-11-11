// SPDX-License-Identifier: MIT

//! Assembles the snapshot of a window from its nodes.

use super::{
    fields::focused_field,
    lookup::{
        descendants, is_ancestor, open_dialog, presented, rendered, rendered_with_role,
        search_results,
    },
};
use crate::{Backend, EntryObservation, Labels, Mark, Observation, PaneObservation, Rect};

/// A snapshot together with the nodes it was read from.
pub struct Scene<Node> {
    pub observation: Observation,
    panes: Vec<PaneNodes<Node>>,
}

/// The nodes of one pane, its entries in the order the snapshot lists them.
pub struct PaneNodes<Node> {
    pub pane: Node,
    pub container: Option<Node>,
    pub entries: Vec<Node>,
}

impl<Node> Scene<Node> {
    pub fn entry(&self, name: &str, directory: Option<&str>) -> Option<&Node> {
        let panes = &self.observation.panes;
        let candidates: Vec<usize> = match directory {
            Some(_) => self.pane_index(directory).into_iter().collect(),
            None => (0..panes.len()).collect(),
        };
        candidates.into_iter().find_map(|pane| {
            let index = panes[pane]
                .entries
                .iter()
                .position(|entry| entry.name == name)?;
            self.panes[pane].entries.get(index)
        })
    }

    /// The last pane with the name wins; without a name, the deepest pane.
    pub fn pane_index(&self, directory: Option<&str>) -> Option<usize> {
        let panes = &self.observation.panes;
        match directory {
            None => panes.len().checked_sub(1),
            Some(directory) => panes.iter().rposition(|pane| pane.name == directory),
        }
    }

    pub fn pane(&self, directory: Option<&str>) -> Option<&PaneNodes<Node>> {
        self.pane_index(directory)
            .and_then(|index| self.panes.get(index))
    }

    pub fn pane_at(&self, index: usize) -> Option<&PaneNodes<Node>> {
        self.panes.get(index)
    }
}

/// Panes are the drawn nodes marked as one, left to right. A pane's entries are those of the
/// first drawn entry container inside it, top to bottom and then left to right.
pub fn scan<B: Backend + ?Sized>(backend: &B, window: &B::Node) -> Scene<B::Node> {
    let labels = backend.labels();
    let focus = backend.focus(window);

    let mut panes: Vec<(f64, PaneObservation, PaneNodes<B::Node>, bool)> = Vec::new();
    for node in &descendants(backend, window) {
        let Some(Mark::Pane(view)) = backend.mark(node) else {
            continue;
        };
        let Some(frame) = backend.frame(node) else {
            continue;
        };
        let (entries, nodes) = pane_entries(backend, node);
        let holds_focus = focus
            .as_ref()
            .is_some_and(|focus| focus == node || is_ancestor(backend, node, focus));
        let pane = PaneObservation {
            name: backend.name(node),
            view,
            entries,
        };
        panes.push((frame.x, pane, nodes, holds_focus));
    }
    panes.sort_by(|left, right| left.0.total_cmp(&right.0));

    // Only the node that takes key presses can be the focused entry.
    let focused_entry = focus
        .as_ref()
        .filter(|focus| rendered(backend, focus))
        .and_then(|focus| backend.entry(focus))
        .map(|row| row.name);
    let focused_pane = panes.iter().rposition(|pane| pane.3);
    let (observed, nodes) = panes
        .into_iter()
        .map(|(_, pane, nodes, _)| (pane, nodes))
        .unzip();
    let presented = presented(backend, window);
    Scene {
        observation: Observation {
            panes: observed,
            focused_entry,
            focused_pane,
            window_width: Some(backend.window_width(window)),
            dialog: open_dialog(backend, &presented).map(|dialog| backend.name(&dialog)),
            menu: open_menu(backend, &presented),
            preview: open_preview(backend, &presented, &labels),
            peek: presented
                .iter()
                .any(|node| rendered(backend, node) && backend.name(node) == labels.peek),
            results: result_names(backend, &presented, &labels),
            field: focused_field(backend, &presented).and_then(|field| backend.field(&field)),
        },
        panes: nodes,
    }
}

fn pane_entries<B: Backend + ?Sized>(
    backend: &B,
    pane: &B::Node,
) -> (Vec<EntryObservation>, PaneNodes<B::Node>) {
    let container = descendants(backend, pane)
        .into_iter()
        .find(|node| backend.mark(node) == Some(Mark::EntryContainer) && rendered(backend, node));
    let mut nodes = PaneNodes {
        pane: pane.clone(),
        container: container.clone(),
        entries: Vec::new(),
    };
    let Some(container) = container else {
        return (Vec::new(), nodes);
    };
    let mut found: Vec<(Rect, _)> = backend
        .entries(&container)
        .into_iter()
        .filter_map(|row| Some((backend.frame(&row.node)?, row)))
        .collect();
    found.sort_by(|left, right| {
        left.0
            .y
            .total_cmp(&right.0.y)
            .then(left.0.x.total_cmp(&right.0.x))
    });
    let (entries, rows): (Vec<EntryObservation>, Vec<B::Node>) = found
        .into_iter()
        .map(|(_, row)| {
            let entry = EntryObservation {
                name: row.name,
                selected: row.selected,
                focused: row.focused,
            };
            (entry, row.node)
        })
        .unzip();
    nodes.entries = rows;
    (entries, nodes)
}

fn open_menu<B: Backend + ?Sized>(backend: &B, presented: &[B::Node]) -> Option<Vec<String>> {
    let menu = rendered_with_role(backend, presented, "menu").next()?;
    let inside = self::presented(backend, menu);
    Some(
        rendered_with_role(backend, &inside, "menu item")
            .map(|item| backend.name(item))
            .collect(),
    )
}

fn open_preview<B: Backend + ?Sized>(
    backend: &B,
    presented: &[B::Node],
    labels: &Labels,
) -> Option<Vec<String>> {
    let drawer = presented
        .iter()
        .find(|node| rendered(backend, node) && backend.name(node) == labels.preview)?;
    let mut shown = Vec::new();
    for node in self::presented(backend, drawer) {
        let name = backend.name(&node);
        let text = backend.text(&node);
        if !name.is_empty() {
            shown.push(name);
        }
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            shown.push(text);
        }
    }
    Some(shown)
}

fn result_names<B: Backend + ?Sized>(
    backend: &B,
    presented: &[B::Node],
    labels: &Labels,
) -> Option<Vec<String>> {
    let (results, row_role) = search_results(backend, presented, &labels.search_results)?;
    let inside = self::presented(backend, &results);
    Some(
        rendered_with_role(backend, &inside, row_role)
            .map(|row| backend.name(row))
            .filter(|name| !name.is_empty())
            .collect(),
    )
}
