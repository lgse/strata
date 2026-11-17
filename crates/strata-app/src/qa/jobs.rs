// SPDX-License-Identifier: MIT

//! The Jobs footer indicator text and dashboard row status text as the QA backend presents them.
//! GTK's `JobsIndicator` (`src/ui/jobs.rs`) draws both as `gtk::Label`s (`role: "label"`), while
//! C16 (`strata_components::progress_dock::jobs::Jobs`) folds them into the footer button's and
//! row group's accessible names/descriptions (`a11y_hidden` on the inner text elements).
//! Presenting the drawn text elements as labels lets shared scenarios read them without changing
//! assistive-technology semantics.

use rune::ElementInfo;
use strata_qa::Rect;

use super::tree::{Seen, Snapshot};

const BUTTON_KEY: &str = "jobs";
const ROW_PREFIX: &str = "jobs-row-";
const STATUS_SUFFIX: &str = "-status";

#[derive(Clone, Debug, PartialEq)]
struct Item {
    segment: String,
    name: String,
    bounds: Rect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Labels(Vec<Item>);

fn bounds(element: &ElementInfo) -> Rect {
    let [x, y, width, height] = element.rect.map(f64::from);
    Rect {
        x,
        y,
        width,
        height,
    }
}

impl Labels {
    pub(super) fn drawn(elements: &[ElementInfo]) -> Self {
        let mut items = Vec::new();
        for element in elements.iter().filter(|element| !element.exiting) {
            if let Some(item) = status_item(element).or_else(|| indicator_item(elements, element)) {
                items.push(item);
            }
        }
        Self(items)
    }

    pub(super) fn present(&self, snapshot: &mut Snapshot) {
        for item in &self.0 {
            let seen = Seen {
                role: "label".to_owned(),
                name: item.name.clone(),
                bounds: Some(item.bounds),
                ..Seen::default()
            };
            snapshot.push_under_window(&item.segment, seen);
        }
    }
}

fn status_item(element: &ElementInfo) -> Option<Item> {
    let key = element.key.as_deref()?;
    let is_status = key.starts_with(ROW_PREFIX) && key.ends_with(STATUS_SUFFIX);
    if !is_status {
        return None;
    }
    Some(Item {
        segment: format!("@{key}"),
        name: element.text.clone()?,
        bounds: bounds(element),
    })
}

fn indicator_item(elements: &[ElementInfo], element: &ElementInfo) -> Option<Item> {
    if element.key.is_some() {
        return None;
    }
    let parent_key = element
        .parent
        .and_then(|parent| elements.get(parent))
        .and_then(|parent| parent.key.as_deref())?;
    if parent_key != BUTTON_KEY {
        return None;
    }
    Some(Item {
        segment: "@jobs-indicator-label".to_owned(),
        name: element.text.clone()?,
        bounds: bounds(element),
    })
}
