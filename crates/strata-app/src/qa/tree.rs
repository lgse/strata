// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    rc::Rc,
};

use serde_json::Value;
use strata_components::{contract::EntryId, pdf_view::page_row_identity};
use strata_qa::{Mark, Orientation, Rect};

pub(super) use super::kept::Scrolled;
pub(crate) use super::kept::{Kept, Node};
use super::{
    kept::{SCROLLS_AT_LEAST, is_entry_key},
    roles,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct NameLabel {
    /// The keys of the elements around it, the nearest first.
    pub(super) above: Vec<String>,
    pub(super) text: String,
    /// In window points.
    pub(super) bounds: Rect,
    pub(super) badge: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RowIcon {
    /// The keys of the elements around it, the nearest first.
    pub(super) above: Vec<String>,
    /// The keyed element's box, in window points.
    pub(super) bounds: Rect,
    pub(super) glyph: Option<Rect>,
}

/// A row as the view draws it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DrawnRow {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) selected: bool,
    pub(super) bounds: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DrawnColumn {
    /// The key of the column's list.
    pub(super) list: String,
    pub(super) name: String,
    pub(super) bounds: Rect,
    /// The group around the list, which the view names by the folder.
    pub(super) pane: Rect,
    pub(super) rows: Vec<DrawnRow>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Shape {
    pub(super) name: &'static str,
    pub(super) list: &'static str,
    pub(super) entry: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DrawnButton {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) bounds: Rect,
    /// The list of the drawn column whose pane is among the button's ancestors, if any.
    pub(super) column: Option<String>,
}

/// What the snapshot is read from beside the accessibility tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Drawn {
    /// The shown view's shape. None when no view is shown, and then it has no lists.
    pub(super) view: Option<Shape>,
    /// The keys of the entry lists the view shows, one for each pane.
    pub(super) lists: Vec<String>,
    pub(super) columns: Vec<DrawnColumn>,
    pub(super) labels: Vec<NameLabel>,
    pub(super) icons: Vec<RowIcon>,
    pub(super) icon_descriptions: HashMap<EntryId, String>,
    pub(super) buttons: Vec<DrawnButton>,
    pub(super) sidebar: super::sidebar::Gaps,
    pub(super) jobs: super::jobs::Labels,
    pub(super) tenxer: super::tenxer::Labels,
    /// The texts drawn inside the controls Rune makes one leaf (`qa::control_words`).
    pub(super) words: super::control_words::Words,
    /// The entry the filter's Enter would open (`qa::filter`).
    pub(super) outlined: Option<EntryId>,
    pub(super) edit_range: Option<[usize; 2]>,
    pub(super) busy: HashSet<String>,
}

/// One node as Rune reports it.
#[derive(Clone, Debug, Default)]
pub(super) struct Seen {
    pub(super) node: Option<Node>,
    pub(super) parent: Option<usize>,
    pub(super) children: Vec<usize>,
    /// Rune's own role name: `list-box-option`.
    pub(super) role: String,
    pub(super) key: Option<String>,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) value: Option<String>,
    pub(super) number: Option<f64>,
    pub(super) states: Vec<String>,
    pub(super) actions: Vec<String>,
    /// In window points.
    pub(super) bounds: Option<Rect>,
    pub(super) active_descendant: Option<String>,
    pub(super) selection: Option<[u64; 4]>,
    pub(super) edit_range: Option<[usize; 2]>,
    pub(super) busy: bool,
    /// A scroll view's offsets and limits.
    pub(super) scroll: Option<Scrolled>,
    pub(super) mark: Option<Mark>,
}

impl Seen {
    pub(super) fn has_state(&self, state: &str) -> bool {
        self.states.iter().any(|held| held == state)
    }

    pub(super) fn has_action(&self, action: &str) -> bool {
        self.actions.iter().any(|offered| offered == action)
    }

    /// The AT-SPI role name the scenarios use.
    pub(super) fn role(&self) -> &'static str {
        if roles::is_search_box(&self.role, self.key.as_deref()) {
            return roles::SEARCH_BOX;
        }
        let toggled = self.has_state("checked") || self.has_state("unchecked");
        roles::at_spi_toggled(&self.role, !self.name.is_empty(), toggled)
    }
}

#[derive(Debug, Default)]
pub(super) struct Snapshot {
    nodes: Vec<Seen>,
    index: HashMap<Node, usize>,
    focus: Option<usize>,
    outlined: Option<EntryId>,
}

fn text(node: &Value, field: &str) -> String {
    node[field].as_str().unwrap_or_default().to_owned()
}

fn texts(node: &Value, field: &str) -> Vec<String> {
    let items = node[field].as_array().map_or(&[][..], Vec::as_slice);
    items
        .iter()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

fn description(node: &Value) -> String {
    let described = text(node, "description");
    let menu_row = node["role"]
        .as_str()
        .is_some_and(|role| role.starts_with("menu-item"));
    if described.is_empty() && menu_row {
        return text(node, "shortcut");
    }
    described
}

fn selection(node: &Value) -> Option<[u64; 4]> {
    let ends = node["selection"].as_array()?;
    let end = |at: usize| ends.get(at)?.as_u64();
    Some([end(0)?, end(1)?, end(2)?, end(3)?])
}

fn bounds(node: &Value) -> Option<Rect> {
    let [x, y, width, height] = node["bounds"].as_array()?.as_slice() else {
        return None;
    };
    Some(Rect {
        x: x.as_f64()?,
        y: y.as_f64()?,
        width: width.as_f64()?,
        height: height.as_f64()?,
    })
}

fn scrolled(node: &Value) -> Option<Scrolled> {
    let [x, y, down] = node["scroll"].as_array()?.as_slice() else {
        return None;
    };
    let own = bounds(node)?;
    let sideways = texts(node, "actions")
        .iter()
        .any(|action| action == "scroll-left");
    Some(Scrolled {
        offset: [x.as_f64()?, y.as_f64()?],
        scrolls: [sideways, down.as_f64()? >= SCROLLS_AT_LEAST],
        page: [own.width, own.height],
    })
}

fn segment_of(key: &str) -> String {
    let pdf_page = key
        .strip_prefix(crate::preview::PDF_KEY)
        .is_some_and(|rest| rest.starts_with('-'));
    pdf_page
        .then(|| page_row_identity(key))
        .flatten()
        .unwrap_or_else(|| key.to_owned())
}

const PANEL: &str = "group";

fn is_window_announcer(node: &Value) -> bool {
    let unit = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let bare = node["key"].is_null() && node["children"].is_null() && node["actions"].is_null();
    let live = texts(node, "states")
        .iter()
        .any(|state| matches!(state.as_str(), "live-polite" | "live-assertive"));
    bare && live && node["role"].as_str() == Some("status") && bounds(node) == Some(unit)
}

impl Snapshot {
    /// [`Snapshot::read_named`] under the browser window's root.
    #[cfg(test)]
    pub(super) fn read(tree: &Value, drawn: &Drawn) -> Self {
        Self::read_named("window", tree, drawn)
    }

    pub(super) fn read_named(root_key: &str, tree: &Value, drawn: &Drawn) -> Self {
        let mut snapshot = Self::default();
        if tree.is_object() {
            snapshot.add(tree, None, root_key.to_owned());
            super::submenus::nest(&mut snapshot.nodes);
        }
        snapshot.focus = snapshot.keyboard_focus();
        snapshot.apply_drawn_states(drawn);
        let panes = snapshot.present_view(drawn);
        for label in &drawn.labels {
            snapshot.add_label(label);
        }
        snapshot.unname_archive_members();
        snapshot.label_archive_text_buttons();
        for icon in &drawn.icons {
            snapshot.add_icon(icon, &drawn.icon_descriptions);
        }
        drawn.words.present(&mut snapshot);
        for button in &drawn.buttons {
            snapshot.add_hidden_button(button, &panes);
        }
        drawn.sidebar.present(&mut snapshot);
        drawn.jobs.present(&mut snapshot);
        drawn.tenxer.present(&mut snapshot);
        snapshot.widen_unscrolled_tab_strip();
        snapshot
    }

    fn present_view<'a>(&mut self, drawn: &'a Drawn) -> Vec<(&'a str, usize)> {
        let mut panes = Vec::new();
        let Some(view) = &drawn.view else {
            return panes;
        };
        for column in &drawn.columns {
            if let Some(pane) = self.add_hidden_column(column, view) {
                panes.push((column.list.as_str(), pane));
            }
        }
        for list in &drawn.lists {
            self.mark_column(list, view, &drawn.columns);
        }
        self.add_row_panels();
        panes
    }

    /// The entry the filter's Enter would open, as of this snapshot.
    pub(super) fn outlined(&self) -> Option<EntryId> {
        self.outlined
    }

    /// Presents `seen` under the window: a node a part draws and the tree leaves out.
    pub(super) fn push_under_window(&mut self, segment: &str, seen: Seen) {
        if !self.nodes.is_empty() {
            self.push(0, segment, seen);
        }
    }

    fn add_hidden_button(&mut self, button: &DrawnButton, panes: &[(&str, usize)]) {
        let listed = |seen: &Seen| seen.key.as_deref() == Some(button.key.as_str());
        let menu = |seen: &Seen| seen.role == "menu" && seen.has_state("modal");
        let modal = self.nodes.iter().any(menu);
        if !modal || self.nodes.iter().any(listed) {
            return;
        }
        let seen = Seen {
            role: "button".to_owned(),
            key: Some(button.key.clone()),
            name: button.name.clone(),
            bounds: Some(button.bounds),
            ..Seen::default()
        };
        let parent = panes
            .iter()
            .find(|(list, _)| button.column.as_deref() == Some(*list))
            .map_or(0, |(_, index)| *index);
        self.push(parent, &button.key, seen);
    }

    /// How many nodes the snapshot holds, presented ones included.
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Presents `seen` under the node at `parent`, at `segment` of its path.
    pub(super) fn push_under(&mut self, parent: usize, segment: &str, seen: Seen) {
        self.push(parent, segment, seen);
    }

    fn push(&mut self, parent: usize, segment: &str, seen: Seen) -> usize {
        let index = self.nodes.len();
        let path = self.nodes[parent]
            .node
            .as_ref()
            .map(|node| node.0.to_string());
        let node = Node(Rc::from(
            format!("{}/{segment}", path.unwrap_or_default()).as_str(),
        ));
        self.index.insert(node.clone(), index);
        self.nodes.push(Seen {
            node: Some(node),
            parent: Some(parent),
            ..seen
        });
        self.nodes[parent].children.push(index);
        index
    }

    fn add_hidden_column(&mut self, column: &DrawnColumn, view: &Shape) -> Option<usize> {
        let listed = |seen: &Seen| seen.key.as_deref() == Some(column.list.as_str());
        if self.nodes.is_empty() || self.nodes.iter().any(listed) {
            return None;
        }
        let named = |role: &str, bounds: Rect| Seen {
            role: role.to_owned(),
            name: column.name.clone(),
            bounds: Some(bounds),
            ..Seen::default()
        };
        let pane = self.push(
            0,
            &format!("@pane-{}", column.list),
            named("group", column.pane),
        );
        let list = Seen {
            key: Some(column.list.clone()),
            ..named(view.list, column.bounds)
        };
        let list = self.push(pane, &column.list, list);
        for row in &column.rows {
            let state = if row.selected {
                "selected"
            } else {
                "unselected"
            };
            let seen = Seen {
                role: view.entry.to_owned(),
                key: Some(row.key.clone()),
                name: row.name.clone(),
                states: vec![state.to_owned()],
                bounds: Some(row.bounds),
                ..Seen::default()
            };
            self.push(list, &row.key, seen);
        }
        Some(pane)
    }

    fn add_row_panels(&mut self) {
        let rows: Vec<(usize, Rect)> = (0..self.nodes.len())
            .filter(|index| self.nodes[*index].mark == Some(Mark::Entry))
            .filter_map(|index| Some((index, self.nodes[index].bounds?)))
            .collect();
        for (row, bounds) in rows {
            let panel = Seen {
                role: PANEL.to_owned(),
                bounds: Some(bounds),
                ..Seen::default()
            };
            self.push(row, "@panel", panel);
        }
    }

    /// Presents a row's drawn name (or Columns size/count badge) as a `label` under its entry.
    fn add_label(&mut self, label: &NameLabel) {
        let Some(row) = self.entry_around(&label.above) else {
            return;
        };
        let seen = Seen {
            role: "label".to_owned(),
            name: label.text.clone(),
            bounds: Some(label.bounds),
            ..Seen::default()
        };
        let segment = if label.badge { "@badge" } else { "@label" };
        self.push(row, segment, seen);
    }

    fn unname_archive_members(&mut self) {
        let in_archive = |nodes: &[Seen], mut at: Option<usize>| {
            while let Some(index) = at {
                if nodes[index].key.as_deref() == Some(crate::preview::ARCHIVE_KEY) {
                    return true;
                }
                at = nodes[index].parent;
            }
            false
        };
        let members: Vec<usize> = (0..self.nodes.len())
            .filter(|&index| self.nodes[index].role == "list-box-option")
            .filter(|&index| in_archive(&self.nodes, self.nodes[index].parent))
            .collect();
        for index in members {
            self.nodes[index].name.clear();
        }
    }

    fn label_archive_text_buttons(&mut self) {
        let crumb = format!("{}-crumb-", crate::preview::ARCHIVE_KEY);
        let unlock = format!("{}-unlock", crate::preview::PROMPT_KEY);
        let is_text_button = |seen: &Seen| {
            seen.role == "button"
                && !seen.name.is_empty()
                && seen.key.as_deref().is_some_and(|key| {
                    key == unlock
                        || key
                            .strip_prefix(&crumb)
                            .is_some_and(|level| level.parse::<usize>().is_ok())
                })
        };
        let buttons: Vec<usize> = (0..self.nodes.len())
            .filter(|&index| is_text_button(&self.nodes[index]))
            .collect();
        for index in buttons {
            let seen = Seen {
                role: "label".to_owned(),
                name: self.nodes[index].name.clone(),
                bounds: self.nodes[index].bounds,
                ..Seen::default()
            };
            self.push(index, "@label", seen);
        }
    }

    /// Presents a row's drawn icon as an `image` under the entry it is in.
    fn add_icon(&mut self, icon: &RowIcon, descriptions: &HashMap<EntryId, String>) {
        let Some(row) = self.entry_around(&icon.above) else {
            return;
        };
        let description = self.nodes[row]
            .node
            .as_ref()
            .and_then(|node| self.keep(node).entry())
            .and_then(|id| descriptions.get(&id).cloned())
            .unwrap_or_default();
        let seen = Seen {
            role: "image".to_owned(),
            description,
            bounds: Some(icon.glyph.unwrap_or(icon.bounds)),
            ..Seen::default()
        };
        self.push(row, "@icon", seen);
        let children = &mut self.nodes[row].children;
        children.rotate_right(1);
    }

    fn entry_around(&self, above: &[String]) -> Option<usize> {
        let entry = |key: &String| {
            let row_id = is_entry_key(key);
            self.nodes.iter().position(|seen| {
                seen.key.as_deref() == Some(key.as_str())
                    && (seen.mark == Some(Mark::Entry)
                        || (row_id && seen.role == "list-box-option"))
            })
        };
        above.iter().find_map(entry)
    }

    fn add(&mut self, value: &Value, parent: Option<usize>, path: String) -> usize {
        let index = self.nodes.len();
        let node = Node(Rc::from(path.as_str()));
        self.index.insert(node.clone(), index);
        self.nodes.push(Seen {
            node: Some(node),
            parent,
            children: Vec::new(),
            role: text(value, "role"),
            key: value["key"].as_str().map(str::to_owned),
            name: text(value, "name"),
            description: description(value),
            value: value["value"].as_str().map(str::to_owned),
            number: value["range"][0].as_f64(),
            states: texts(value, "states"),
            actions: texts(value, "actions"),
            bounds: bounds(value),
            active_descendant: value["activeDescendant"].as_str().map(str::to_owned),
            selection: selection(value),
            edit_range: None,
            busy: false,
            scroll: scrolled(value),
            mark: None,
        });
        let children = value["children"].as_array().map_or(&[][..], Vec::as_slice);
        let mut taken: Vec<String> = Vec::new();
        for (place, child) in children.iter().enumerate() {
            if parent.is_none() && is_window_announcer(child) {
                continue;
            }
            let mut segment = child["key"]
                .as_str()
                .map_or_else(|| format!("#{place}"), segment_of);
            if taken.contains(&segment) {
                let _ = write!(segment, "~{place}");
            }
            let added = self.add(child, Some(index), format!("{path}/{segment}"));
            self.nodes[index].children.push(added);
            taken.push(segment);
        }
        index
    }

    fn apply_drawn_states(&mut self, drawn: &Drawn) {
        self.outlined = drawn.outlined;
        if !drawn.busy.is_empty() {
            for seen in &mut self.nodes {
                seen.busy = seen
                    .node
                    .as_ref()
                    .is_some_and(|node| drawn.busy.contains(node.0.as_ref()));
            }
        }
        let Some(focused) = self.focus else {
            return;
        };
        let seen = &mut self.nodes[focused];
        if super::field::is_one(&seen.role) && seen.role != "password-input" {
            seen.edit_range = drawn.edit_range;
        }
    }

    fn widen_unscrolled_tab_strip(&mut self) {
        let Some(scroll_idx) = self
            .nodes
            .iter()
            .position(|seen| seen.key.as_deref() == Some("tabs-scroll"))
        else {
            return;
        };
        if self.nodes[scroll_idx]
            .scroll
            .is_some_and(|scroll| scroll.scrolls[0])
        {
            return;
        }
        let Some(parent_idx) = self.nodes[scroll_idx].parent else {
            return;
        };
        if self.nodes[parent_idx].key.as_deref() == Some("tabs")
            && let Some(strip_bounds) = self.nodes[parent_idx].bounds
        {
            self.nodes[scroll_idx].bounds = Some(strip_bounds);
        }
    }

    fn keyboard_focus(&self) -> Option<usize> {
        let focused = self
            .nodes
            .iter()
            .position(|seen| seen.has_state("focused"))?;
        let active = self.nodes[focused].active_descendant.as_deref();
        let inside = self.below(focused);
        let pointed = active.and_then(|key| {
            let named = |index: &usize| self.nodes[*index].key.as_deref() == Some(key);
            inside.into_iter().find(named)
        });
        Some(pointed.unwrap_or(focused))
    }

    /// Everything under `index`, parents first.
    fn below(&self, index: usize) -> Vec<usize> {
        let mut found = Vec::new();
        let mut stack: Vec<usize> = self.nodes[index].children.iter().rev().copied().collect();
        while let Some(next) = stack.pop() {
            stack.extend(self.nodes[next].children.iter().rev());
            found.push(next);
        }
        found
    }

    fn mark_column(&mut self, list: &str, view: &Shape, columns: &[DrawnColumn]) {
        let is_list = |seen: &Seen| seen.role == view.list && seen.key.as_deref() == Some(list);
        let Some(container) = self.nodes.iter().position(is_list) else {
            return;
        };
        self.nodes[container].mark = Some(Mark::EntryContainer);
        for index in self.below(container) {
            let seen = &mut self.nodes[index];
            if seen.role == view.entry && seen.key.as_deref().is_some_and(is_entry_key) {
                seen.mark = Some(Mark::Entry);
                super::stage::sync_entry_bounds(columns, list, seen);
            }
        }
        let above = std::iter::successors(self.nodes[container].parent, |index| {
            self.nodes[*index].parent
        });
        let pane = above.into_iter().find(|index| {
            let seen = &self.nodes[*index];
            seen.role == "group" && !seen.name.is_empty()
        });
        if let Some(pane) = pane {
            self.nodes[pane].mark = Some(Mark::Pane(view.name));
        }
    }

    pub(super) fn with_toplevel(&self, other: &Snapshot) -> Snapshot {
        let offset = self.nodes.len();
        let moved = other.nodes.iter().cloned().map(|mut seen| {
            seen.parent = seen.parent.map(|parent| parent + offset);
            for child in &mut seen.children {
                *child += offset;
            }
            seen
        });
        let mut index = self.index.clone();
        index.extend(
            other
                .index
                .iter()
                .map(|(node, at)| (node.clone(), at + offset)),
        );
        Snapshot {
            nodes: self.nodes.iter().cloned().chain(moved).collect(),
            index,
            focus: other.focus.map(|focus| focus + offset),
            outlined: self.outlined,
        }
    }

    pub(super) fn with_window_behind(&self, other: &Snapshot) -> Snapshot {
        let mut both = self.with_toplevel(other);
        both.focus = self.focus;
        both
    }

    pub(super) fn get(&self, node: &Node) -> Option<&Seen> {
        self.index.get(node).map(|index| &self.nodes[*index])
    }

    pub(super) fn scrollbar_target(&self, along: Orientation) -> Option<(Node, String, [f64; 2])> {
        for (index, seen) in self.nodes.iter().enumerate() {
            let Some(scrolled) = seen.scroll else {
                continue;
            };
            if !scrolled.along(along).scrollable {
                continue;
            }
            let node = seen.node.clone()?;
            let mut segments: Vec<&str> =
                std::iter::successors(Some(index), |at| self.nodes[*at].parent)
                    .filter_map(|at| self.nodes[at].key.as_deref())
                    .collect();
            if segments.is_empty() {
                continue;
            }
            segments.reverse();
            return Some((node, segments.join("/"), scrolled.offset));
        }
        None
    }

    pub(super) fn at(&self, index: usize) -> Option<&Seen> {
        self.nodes.get(index)
    }

    pub(super) fn root(&self) -> Option<&Seen> {
        self.nodes.first()
    }

    pub(super) fn focus(&self) -> Option<&Seen> {
        self.focus.and_then(|index| self.nodes.get(index))
    }

    pub(super) fn is_focus(&self, seen: &Seen) -> bool {
        self.focus().is_some_and(|focus| focus.node == seen.node)
    }

    pub(super) fn is_focusable(&self, seen: &Seen) -> bool {
        seen.has_action("focus") || self.reached_as_active_descendant(seen)
    }

    /// Whether `seen` is an entry whose nearest focus stop names an active descendant.
    fn reached_as_active_descendant(&self, seen: &Seen) -> bool {
        if seen.mark != Some(Mark::Entry) {
            return false;
        }
        let mut above = std::iter::successors(seen.parent, |index| self.nodes[*index].parent);
        above
            .find(|index| self.nodes[*index].has_action("focus"))
            .is_some_and(|stop| self.nodes[stop].active_descendant.is_some())
    }

    pub(super) fn keep(&self, node: &Node) -> Kept {
        Kept {
            node: node.clone(),
            role: self
                .get(node)
                .map(|seen| seen.role.clone())
                .unwrap_or_default(),
            since: 0,
        }
    }

    pub(super) fn recall(&self, kept: &Kept) -> Option<Node> {
        if let Some(seen) = self.get(&kept.node) {
            return (seen.role == kept.role).then(|| kept.node.clone());
        }
        let key = kept.row_key()?;
        self.nodes
            .iter()
            .find(|seen| seen.role == kept.role && seen.key.as_deref() == Some(key))
            .and_then(|seen| seen.node.clone())
    }

    pub(super) fn dump(&self) -> String {
        let mut output = String::new();
        if let Some(focus) = self.focus() {
            let _ = writeln!(
                output,
                "focus: {} role={:?} name={:?}",
                focus.role,
                focus.role(),
                focus.name
            );
        }
        if !self.nodes.is_empty() {
            self.dump_node(0, 0, &mut output);
        }
        output
    }

    fn dump_node(&self, index: usize, depth: usize, output: &mut String) {
        let seen = &self.nodes[index];
        let _ = write!(output, "{:indent$}{}", "", seen.role, indent = depth * 2);
        if let Some(key) = &seen.key {
            let _ = write!(output, "#{key}");
        }
        let _ = write!(output, " role={:?}", seen.role());
        self.dump_words(seen, output);
        if let Some(bounds) = seen.bounds {
            let _ = write!(
                output,
                " ({:.0},{:.0} {:.0}x{:.0})",
                bounds.x, bounds.y, bounds.width, bounds.height
            );
        }
        output.push('\n');
        for child in &seen.children {
            self.dump_node(*child, depth + 1, output);
        }
    }

    fn dump_words(&self, seen: &Seen, output: &mut String) {
        if !seen.name.is_empty() {
            let _ = write!(output, " name={:?}", seen.name);
        }
        if seen.has_state("selected") {
            output.push_str(" [selected]");
        }
        if !seen.description.is_empty() {
            let _ = write!(output, " description={:?}", seen.description);
        }
        if let Some(value) = seen.value.as_ref().filter(|value| **value != seen.name) {
            let _ = write!(output, " text={value:?}");
        }
        if self.is_focus(seen) {
            output.push_str(" [focused]");
        }
    }
}

#[cfg(test)]
mod tests;
