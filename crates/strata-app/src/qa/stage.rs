// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    rc::Rc,
    time::{Duration, Instant},
};

use rune::ElementInfo;
use strata_components::{
    collection::element::{ICON, LABEL},
    columns::keys::DETAIL,
    contract::EntryId,
    thumbnails::GLYPH,
};
use strata_qa::Rect;

use super::{
    kept::entry_id_of,
    tree::{Drawn, EntryParts, Shape, Snapshot},
};
use crate::{App, host::offscreen::Offscreen, shell};

const SETTLE_LIMIT: Duration = Duration::from_millis(250);

fn rect(element: &ElementInfo) -> Rect {
    let [x, y, width, height] = element.rect.map(f64::from);
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// The elements above `element`, the nearest first.
fn above<'a>(
    elements: &'a [ElementInfo],
    element: &ElementInfo,
) -> impl Iterator<Item = &'a ElementInfo> {
    std::iter::successors(element.parent, |index| elements.get(*index)?.parent)
        .filter_map(|index| elements.get(index))
}

fn label_bounds(elements: &[ElementInfo], element: &ElementInfo, app: &App) -> Rect {
    let mut bounds = rect(element);
    let chain: Vec<&ElementInfo> = above(elements, element).collect();
    let Some(row_pos) = chain.iter().position(|outer| {
        outer
            .key
            .as_deref()
            .is_some_and(|key| entry_id_of(key).is_some())
    }) else {
        return bounds;
    };
    if chain[row_pos].role == "grid-cell" {
        return bounds;
    }
    let right = if let Some(cell) = chain
        .iter()
        .take(row_pos)
        .find(|outer| outer.key.as_deref() == Some("cell-name"))
    {
        let pad = f64::from(strata_kit::slots::list::cell_pad(&app.look));
        f64::from(cell.rect[0] + cell.rect[2]) - pad
    } else if row_pos >= 2 {
        let content = chain[row_pos - 2];
        f64::from(content.rect[0] + content.rect[2])
    } else {
        bounds.x + bounds.width
    };
    if right > bounds.x + bounds.width {
        bounds.width = right - bounds.x;
    }
    bounds
}

/// Drawn label and icon geometry per browser entry, for pointer targeting only.
fn entry_parts(elements: &[ElementInfo], app: &App) -> HashMap<EntryId, EntryParts> {
    let mut parts: HashMap<EntryId, EntryParts> = HashMap::new();
    for element in elements.iter().filter(|element| !element.exiting) {
        match element.key.as_deref() {
            Some(LABEL) => {
                if let Some(id) = owning_entry(elements, element, true) {
                    parts
                        .entry(id)
                        .or_default()
                        .label
                        .get_or_insert_with(|| label_bounds(elements, element, app));
                }
            }
            Some(ICON) => {
                if let Some(id) = owning_entry(elements, element, false) {
                    let glyph = elements.iter().find(|inner| {
                        !inner.exiting
                            && inner.key.as_deref() == Some(GLYPH)
                            && inner
                                .parent
                                .and_then(|parent| elements.get(parent))
                                .is_some_and(|parent| std::ptr::eq(parent, element))
                    });
                    parts.entry(id).or_default().icon =
                        Some(glyph.map_or_else(|| rect(element), rect));
                }
            }
            _ => {}
        }
    }
    parts
}

fn owning_entry(
    elements: &[ElementInfo],
    element: &ElementInfo,
    skip_detail: bool,
) -> Option<EntryId> {
    for outer in above(elements, element) {
        if skip_detail && outer.key.as_deref() == Some(DETAIL) {
            return None;
        }
        if let Some(id) = outer.key.as_deref().and_then(entry_id_of) {
            return Some(id);
        }
    }
    None
}

/// Window-level `Tx::announce` live region (`a11y/tree.rs` at the pinned rev).
const ANNOUNCER_ID: u64 = 3;

fn child_segment(author_id: Option<&str>, place: usize, taken: &mut Vec<String>) -> String {
    let mut segment = author_id.map_or_else(|| format!("#{place}"), str::to_owned);
    if taken.contains(&segment) {
        let _ = write!(segment, "~{place}");
    }
    taken.push(segment.clone());
    segment
}

fn collect_busy(
    nodes: &HashMap<u64, &rune::accesskit::Node>,
    id: u64,
    path: &str,
    busy: &mut HashSet<String>,
) {
    let Some(node) = nodes.get(&id).copied() else {
        return;
    };
    if node.is_busy() {
        busy.insert(path.to_owned());
    }
    let children = node
        .children()
        .iter()
        .filter_map(|child| Some((child.0, *nodes.get(&child.0)?)))
        .filter(|(_, child)| child.role() != rune::accesskit::Role::TextRun);
    let mut taken: Vec<String> = Vec::new();
    for (place, (child_id, child)) in children.enumerate() {
        if path == "window" && child_id == ANNOUNCER_ID {
            continue;
        }
        let segment = child_segment(child.author_id(), place, &mut taken);
        collect_busy(nodes, child_id, &format!("{path}/{segment}"), busy);
    }
}

fn busy_paths(update: &rune::accesskit::TreeUpdate) -> HashSet<String> {
    if !update.nodes.iter().any(|(_, node)| node.is_busy()) {
        return HashSet::new();
    }
    let nodes: HashMap<u64, &rune::accesskit::Node> =
        update.nodes.iter().map(|(id, node)| (id.0, node)).collect();
    let root = update.tree.as_ref().map_or_else(
        || update.nodes.first().map_or(1, |(id, _)| id.0),
        |tree| tree.root.0,
    );
    let mut busy = HashSet::new();
    collect_busy(&nodes, root, "window", &mut busy);
    busy
}

pub(super) fn drawn(elements: &[ElementInfo], app: &App) -> Drawn {
    let probe = shell::shown_view(app).map(|view| &view.probe);
    let lists = probe.map_or_else(Vec::new, |probe| (probe.lists)(app));
    Drawn {
        view: probe.map(|probe| Shape {
            name: probe.name,
            list: probe.roles.0,
            entry: probe.roles.1,
        }),
        parts: entry_parts(elements, app),
        outlined: crate::filter::outlined(app),
        edit_range: elements
            .iter()
            .find(|element| !element.exiting && element.focused)
            .and_then(|element| element.edit_range),
        busy: HashSet::new(),
        lists,
    }
}

/// [`drawn`], plus the busy nodes from `update` (`Runtime::accessibility_tree`).
pub(super) fn drawn_with_busy(
    elements: &[ElementInfo],
    app: &App,
    update: &rune::accesskit::TreeUpdate,
) -> Drawn {
    Drawn {
        busy: busy_paths(update),
        ..drawn(elements, app)
    }
}

pub(super) struct Stage {
    root_key: String,
    host: Offscreen,
    /// The tree as of the last turn, read when first asked for.
    seen: Option<Rc<Snapshot>>,
}

impl Stage {
    pub(super) fn named(root_key: String, host: Offscreen) -> Self {
        Self {
            root_key,
            host,
            seen: None,
        }
    }

    pub(super) fn app(&self) -> &App {
        self.host.app()
    }

    /// The host, for what reading it changes nothing of.
    pub(super) fn app_host(&self) -> &Offscreen {
        &self.host
    }

    /// The host, for anything that can change what it shows: the tree is read again after.
    pub(super) fn host(&mut self) -> &mut Offscreen {
        self.seen = None;
        &mut self.host
    }

    pub(super) fn snapshot(&mut self) -> Rc<Snapshot> {
        let host = &mut self.host;
        let root_key = &self.root_key;
        self.seen
            .get_or_insert_with(|| {
                let update = host.runtime().accessibility_tree();
                let drawn = drawn_with_busy(&host.elements(), host.app(), &update);
                Rc::new(Snapshot::read_named(root_key, &host.tree(), &drawn))
            })
            .clone()
    }

    pub(super) fn turn(&mut self) -> bool {
        let host = self.host();
        let worked = host.turn();
        if worked {
            host.runtime().settle_all();
        }
        worked
    }

    pub(super) fn settle(&mut self) {
        let deadline = Instant::now() + SETTLE_LIMIT;
        while self.turn() && Instant::now() < deadline {}
    }

    /// Blocks until GLib has work, a Rune timer is due, or `limit` has passed.
    pub(super) fn wait(&mut self, limit: Duration) {
        self.host().wait(limit);
    }

    /// Lets the application run for `time`.
    pub(super) fn sleep(&mut self, time: Duration) {
        let deadline = Instant::now() + time;
        loop {
            let worked = self.turn();
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            if left.is_zero() {
                return;
            }
            if !worked {
                self.wait(left);
            }
        }
    }
}

#[cfg(test)]
mod tests;
