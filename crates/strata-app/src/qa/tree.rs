// SPDX-License-Identifier: MIT

//! Rune's accessibility tree as the QA engine walks it: one snapshot of
//! `Runtime::accessibility_json`, with each node under the path of keys that leads to it.

use std::{collections::HashMap, fmt::Write as _, rc::Rc};

use serde_json::Value;
use strata_components::{collection::element::row_key, contract::EntryId};
use strata_qa::{Mark, Orientation, Rect, Scroll};

use super::roles;

/// A node of the tree: the keys from the window down to it. An element without a key stands
/// under its place among its siblings. A row's key holds its entry's id, so the path of a
/// row a list has given to another entry is another path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Node(Rc<str>);

/// What `remember` holds of a node: its path, and the role it had, so a different element
/// that later stands at the same place is not taken for it; and for an entry's row, the
/// browser's replacements as of then (`qa::replaced`), which the backend fills in.
#[derive(Clone, Debug)]
pub(crate) struct Kept {
    node: Node,
    role: String,
    since: u64,
}

impl Kept {
    /// Kept after the browser's `count`th replacement.
    pub(super) fn after(self, count: u64) -> Self {
        Self {
            since: count,
            ..self
        }
    }

    pub(super) fn since(&self) -> u64 {
        self.since
    }

    /// The entry whose row this is, read from the row's key.
    pub(super) fn entry(&self) -> Option<EntryId> {
        let key = self.node.0.rsplit('/').next()?;
        let digits = key.trim_start_matches(|c: char| !c.is_ascii_digit());
        is_entry_key(key).then(|| digits.parse().ok().map(EntryId::new))?
    }
}

/// A row's name as the view draws it. Rune's row is one accessible node, and its name is an
/// element the tree leaves out; the GTK row has a `label` child, which a scenario aims at as
/// an entry's `part: label`. The component keys the element for its hosts
/// (`strata_components::collection::element::LABEL`), so its box is read from the drawn
/// elements, and the snapshot presents it under its row.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct NameLabel {
    /// The keys of the elements around it, the nearest first.
    pub(super) above: Vec<String>,
    pub(super) text: String,
    /// In window points.
    pub(super) bounds: Rect,
}

/// A row's icon as the view draws it: the GTK row's `image`, which a scenario aims at as an
/// entry's `part: icon` and a drag starts on. The component keys it as it keys the name
/// (`strata_components::collection::element::ICON`), and the snapshot presents it under its
/// row the same way.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RowIcon {
    /// The keys of the elements around it, the nearest first.
    pub(super) above: Vec<String>,
    /// In window points.
    pub(super) bounds: Rect,
}

/// A row as the view draws it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DrawnRow {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) selected: bool,
    pub(super) bounds: Rect,
}

/// A column as the view draws it: its list, the group around it, and its rows. Behind an
/// open menu Rune's accessibility tree holds only the menu, since a menu is modal, while
/// the GTK tree keeps the panes, and scenarios read a pane's selection with its menu open.
/// The snapshot then presents each column from what is drawn: names, selection and boxes.
/// A drawn element carries no description, so those rows have none.
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

/// What the shown view says of its panes (`slots::Probe`): its name in the protocol, and
/// Rune's role of an entry list and of an entry in one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Shape {
    pub(super) name: &'static str,
    pub(super) list: &'static str,
    pub(super) entry: &'static str,
}

/// A button as drawn: its key, its name and its box. Behind an open menu Rune's tree holds
/// only the menu, where GTK's keeps the window's buttons, and a scenario presses one with
/// a menu open (`keyboard_navigation/back-history-dropdown` lets go of a press held on
/// Back after its history opened). The snapshot then presents each keyed, named button the
/// tree left out, from what is drawn, if any of it is in view and it is not inert.
///
/// It is presented with nothing else: it reads `sensitive` whatever it is (Rune's drawn
/// elements say nothing of being disabled), it is not focusable and offers no click action,
/// so a run that asserts a state of a button behind a menu needs an `a11y` twin. It hangs
/// off the window's root: a target `within` a pane misses it, and `nth: -1` finds it first.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DrawnButton {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) bounds: Rect,
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
    pub(super) buttons: Vec<DrawnButton>,
    pub(super) sidebar: super::sidebar::Gaps,
}

/// How far a scroll view has scrolled, whether it can, and how much it shows: along x, then
/// y, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Scrolled {
    pub(super) offset: [f64; 2],
    pub(super) scrolls: [bool; 2],
    pub(super) page: [f64; 2],
}

/// Less than this far to go is nowhere: Rune's tree rounds to a tenth of a point.
const SCROLLS_AT_LEAST: f64 = 0.5;

impl Scrolled {
    fn axis(along: Orientation) -> usize {
        match along {
            Orientation::Horizontal => 0,
            Orientation::Vertical => 1,
        }
    }

    pub(super) fn along(&self, along: Orientation) -> Scroll {
        let axis = Self::axis(along);
        Scroll {
            value: self.offset[axis],
            page: self.page[axis],
            scrollable: self.scrolls[axis],
        }
    }

    /// Where the scroll bar GTK would show stands: a scrolled window shows the vertical one
    /// when its content is taller than it, else the horizontal one when it is wider, else
    /// none.
    pub(super) fn bar(&self) -> Option<f64> {
        [1, 0]
            .into_iter()
            .find(|axis| self.scrolls[*axis])
            .map(|axis| self.offset[axis])
    }
}

/// One node as Rune reports it.
#[derive(Debug, Default)]
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
    /// A text field's selection as Rune reports it: the line and the place in it of the end
    /// that stays, then of the end that moves, which is the caret.
    pub(super) selection: Option<[u64; 4]>,
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
        let toggled = self.has_state("checked") || self.has_state("unchecked");
        roles::at_spi_toggled(&self.role, !self.name.is_empty(), toggled)
    }
}

#[derive(Debug, Default)]
pub(super) struct Snapshot {
    nodes: Vec<Seen>,
    index: HashMap<Node, usize>,
    /// The node that takes key presses: the focused element's active descendant when it
    /// names one, since a Rune list keeps focus and points at its current row.
    focus: Option<usize>,
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

/// What a node says beside its name. A menu row's shortcut is what Rune reports as its
/// keyboard shortcut and the GTK menu as its description, where scenarios read it.
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

/// What a scroll view reports. Rune's tree gives its offsets and how far it scrolls down
/// (`"scroll": [x, y, y_max]`), and no horizontal limit. It offers `scroll-left` exactly when
/// it can scroll sideways and input may reach it (`a11y/node.rs` at the pinned rev), which is
/// whether it scrolls sideways for a user.
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

/// Rune's role of an unnamed box, which the role map presents as GTK's `panel`.
const PANEL: &str = "group";

/// Whether `key` is the collection's key for an entry's row ([`row_key`]). A list holds
/// other nodes of an entry's role: the List table's headings are a `row` of its grid too.
fn is_entry_key(key: &str) -> bool {
    let digits = key.trim_start_matches(|c: char| !c.is_ascii_digit());
    let id = digits.parse::<u64>();
    id.is_ok_and(|id| row_key(EntryId::new(id)) == key)
}

impl Snapshot {
    /// Reads `tree`, the nested JSON of `Runtime::accessibility_json`, and what the view
    /// draws that the tree leaves out.
    pub(super) fn read(tree: &Value, drawn: &Drawn) -> Self {
        let mut snapshot = Self::default();
        if tree.is_object() {
            snapshot.add(tree, None, "window".to_owned());
        }
        snapshot.focus = snapshot.keyboard_focus();
        if let Some(view) = &drawn.view {
            for column in &drawn.columns {
                snapshot.add_hidden_column(column, view);
            }
            for list in &drawn.lists {
                snapshot.mark_column(list, view);
            }
            snapshot.add_row_panels();
        }
        for label in &drawn.labels {
            snapshot.add_label(label);
        }
        for icon in &drawn.icons {
            snapshot.add_icon(icon);
        }
        for button in &drawn.buttons {
            snapshot.add_hidden_button(button);
        }
        drawn.sidebar.present(&mut snapshot);
        snapshot
    }

    /// Presents `seen` under the window: a node a part draws and the tree leaves out.
    pub(super) fn push_under_window(&mut self, segment: &str, seen: Seen) {
        if !self.nodes.is_empty() {
            self.push(0, segment, seen);
        }
    }

    /// Presents a button an open menu keeps out of the tree, under the window. Not behind a
    /// dialog, to stay cautious: GTK's dialogs are overlay layers that leave the window's
    /// widgets in the tree, so a run aiming behind one fails on Rune rather than passing
    /// falsely.
    fn add_hidden_button(&mut self, button: &DrawnButton) {
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
        self.push(0, &button.key, seen);
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

    /// Presents a column the tree left out, under the window, as the tree would hold it.
    fn add_hidden_column(&mut self, column: &DrawnColumn, view: &Shape) {
        let listed = |seen: &Seen| seen.key.as_deref() == Some(column.list.as_str());
        if self.nodes.is_empty() || self.nodes.iter().any(listed) {
            return;
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
    }

    /// Presents under each entry the box its content is drawn in, as a `panel`. A GTK row is a
    /// `list item` with such a box inside, which a scenario aims at to check the row's content
    /// is shown (`rename_visibility/committed-rename-stays-visible`, line 14). A Rune row is one
    /// element that draws its content in its own box, so the panel's box is the row's: read,
    /// not guessed (slice decision 20).
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

    /// Presents a row's drawn name as a `label` under the entry it is in.
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
        self.push(row, "@label", seen);
    }

    /// Presents a row's drawn icon as an `image` under the entry it is in.
    fn add_icon(&mut self, icon: &RowIcon) {
        let Some(row) = self.entry_around(&icon.above) else {
            return;
        };
        let seen = Seen {
            role: "image".to_owned(),
            bounds: Some(icon.bounds),
            ..Seen::default()
        };
        self.push(row, "@icon", seen);
    }

    /// The nearest entry among the elements `above` an element, by their keys.
    fn entry_around(&self, above: &[String]) -> Option<usize> {
        let entry = |key: &String| {
            self.nodes.iter().position(|seen| {
                seen.mark == Some(Mark::Entry) && seen.key.as_deref() == Some(key.as_str())
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
            scroll: scrolled(value),
            mark: None,
        });
        let children = value["children"].as_array().map_or(&[][..], Vec::as_slice);
        let mut taken: Vec<String> = Vec::new();
        for (place, child) in children.iter().enumerate() {
            let mut segment = child["key"]
                .as_str()
                .map_or_else(|| format!("#{place}"), str::to_owned);
            if taken.contains(&segment) {
                let _ = write!(segment, "~{place}");
            }
            let added = self.add(child, Some(index), format!("{path}/{segment}"));
            self.nodes[index].children.push(added);
            taken.push(segment);
        }
        index
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

    /// Marks one column where the adapter put it: the list with the column's key is the entry
    /// container, the nodes of the entry role under it keyed as the collection keys its rows
    /// are the entries, and the named group around it, which the view names by the folder, is
    /// the pane.
    fn mark_column(&mut self, list: &str, view: &Shape) {
        let is_list = |seen: &Seen| seen.role == view.list && seen.key.as_deref() == Some(list);
        let Some(container) = self.nodes.iter().position(is_list) else {
            return;
        };
        self.nodes[container].mark = Some(Mark::EntryContainer);
        for index in self.below(container) {
            let seen = &self.nodes[index];
            if seen.role == view.entry && seen.key.as_deref().is_some_and(is_entry_key) {
                self.nodes[index].mark = Some(Mark::Entry);
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

    pub(super) fn get(&self, node: &Node) -> Option<&Seen> {
        self.index.get(node).map(|index| &self.nodes[*index])
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
        let seen = self.get(&kept.node)?;
        (seen.role == kept.role).then(|| kept.node.clone())
    }

    /// The tree as text, a line for each node: Rune's role and key, the AT-SPI role, the name,
    /// the description, the states the scenarios test and the box. The GTK harness prints its
    /// widgets in the same shape.
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

    /// What a node says of itself, in the order the GTK harness prints it.
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
