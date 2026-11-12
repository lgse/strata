// SPDX-License-Identifier: MIT

//! The offscreen host as the QA mode drives it: every turn lands its motions, and the tree
//! is read once for as long as nothing ran.

use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use rune::ElementInfo;
use strata_components::collection::element::LABEL;
use strata_qa::Rect;

use super::tree::{Drawn, DrawnColumn, DrawnRow, NameLabel, Snapshot};
use crate::{App, host::offscreen::Offscreen};

/// How long one settle keeps turning a host that never comes to rest. The GTK harness waits
/// this long for a frame.
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

/// The names the view draws in rows, each with the keys of the elements around it.
fn name_labels(elements: &[ElementInfo]) -> Vec<NameLabel> {
    let drawn = |element: &&ElementInfo| !element.exiting && element.key.as_deref() == Some(LABEL);
    let label = |element: &ElementInfo| {
        Some(NameLabel {
            above: above(elements, element)
                .filter_map(|outer| outer.key.clone())
                .collect(),
            text: element.text.clone()?,
            bounds: rect(element),
        })
    };
    elements.iter().filter(drawn).filter_map(label).collect()
}

/// The column whose list is keyed `list`, as drawn: its rows are the keyed, named elements
/// directly under the list's rows, and its pane the nearest named element around the list.
fn drawn_column(elements: &[ElementInfo], list: &str) -> Option<DrawnColumn> {
    let keyed = |element: &ElementInfo, key: &str| element.key.as_deref() == Some(key);
    let at = elements
        .iter()
        .position(|element| !element.exiting && keyed(element, list))?;
    let pane = above(elements, &elements[at]).find(|outer| outer.label.is_some())?;
    let inside = |element: &&ElementInfo| {
        above(elements, element).any(|outer| std::ptr::eq(outer, &elements[at]))
    };
    let row = |element: &ElementInfo| {
        let key = element.key.clone().filter(|key| key.starts_with("row-"))?;
        Some(DrawnRow {
            key,
            name: element.label.clone()?,
            selected: element.item_selected,
            bounds: rect(element),
        })
    };
    let rows = elements.iter().filter(|element| !element.exiting);
    Some(DrawnColumn {
        list: list.to_owned(),
        name: elements[at].label.clone().unwrap_or_default(),
        bounds: rect(&elements[at]),
        pane: rect(pane),
        rows: rows.filter(inside).filter_map(row).collect(),
    })
}

/// What the snapshot reads beside the accessibility tree.
fn drawn(host: &Offscreen) -> Drawn {
    let elements = host.elements();
    let lists = host.app().columns.list_keys();
    Drawn {
        columns: lists
            .iter()
            .filter_map(|list| drawn_column(&elements, list))
            .collect(),
        labels: name_labels(&elements),
        lists,
    }
}

pub(super) struct Stage {
    host: Offscreen,
    /// The tree as of the last turn, read when first asked for.
    seen: Option<Rc<Snapshot>>,
}

impl Stage {
    pub(super) fn new(host: Offscreen) -> Self {
        Self { host, seen: None }
    }

    pub(super) fn app(&self) -> &App {
        self.host.app()
    }

    /// The host, for anything that can change what it shows: the tree is read again after.
    pub(super) fn host(&mut self) -> &mut Offscreen {
        self.seen = None;
        &mut self.host
    }

    pub(super) fn snapshot(&mut self) -> Rc<Snapshot> {
        let host = &self.host;
        self.seen
            .get_or_insert_with(|| Rc::new(Snapshot::read(&host.tree(), &drawn(host))))
            .clone()
    }

    /// One turn of the host. A turn that did something lands every motion at its end
    /// (`Runtime::settle_all`): `Runtime::frame` reports work for some hundred milliseconds
    /// after a key otherwise, with the look's animations off too.
    pub(super) fn turn(&mut self) -> bool {
        let host = self.host();
        let worked = host.turn();
        if worked {
            host.runtime().settle_all();
        }
        worked
    }

    /// Turns until neither GLib nor Rune has anything ready. A timer still running, such as
    /// the wait before a child column opens, does not hold it up: a scenario waits for that.
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
