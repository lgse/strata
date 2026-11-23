// SPDX-License-Identifier: MIT

//! Re-exposes text inside leaf controls as `label` nodes in QA snapshots because Rune omits
//! redundant text nodes inside checkboxes, radios, switches, tabs, menu items, and options
//! ([anomalyco/rune#1115](https://github.com/anomalyco/rune/issues/1115)).

use rune::ElementInfo;
use strata_qa::Rect;

use super::tree::{Seen, Snapshot};

/// The element roles whose content Rune makes one leaf, buttons and links aside.
const ONE_LEAF: &[&str] = &[
    "checkbox",
    "radio",
    "switch",
    "tab",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
];

/// The same roles, as the accessibility tree names them.
const ONE_LEAF_NODES: &[&str] = &[
    "check-box",
    "radio-button",
    "switch",
    "tab",
    "menu-item",
    "menu-item-check-box",
    "menu-item-radio",
    "list-box-option",
];

/// What stops the walk from a text up to its control: the text belongs to a nearer node.
const OWN_NODE: &[&str] = &["button", "link"];

#[derive(Clone, Debug, PartialEq)]
struct Word {
    /// The key of the control the text is drawn in.
    control: String,
    /// The control's drawn box.
    at: Rect,
    text: String,
    bounds: Rect,
}

/// The texts drawn inside one-leaf controls in the current frame, in tree order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Words(Vec<Word>);

fn bounds(element: &ElementInfo) -> Rect {
    let [x, y, width, height] = element.rect.map(f64::from);
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn control_of<'a>(elements: &'a [ElementInfo], element: &ElementInfo) -> Option<&'a ElementInfo> {
    let mut at = element.parent.and_then(|parent| elements.get(parent));
    while let Some(ancestor) = at {
        if ONE_LEAF.contains(&ancestor.role) {
            return Some(ancestor);
        }
        if ancestor.focusable || OWN_NODE.contains(&ancestor.role) {
            return None;
        }
        at = ancestor.parent.and_then(|parent| elements.get(parent));
    }
    None
}

fn contains_centre(outer: Rect, inner: Rect) -> bool {
    let (x, y) = (inner.x + inner.width / 2.0, inner.y + inner.height / 2.0);
    x >= outer.x && y >= outer.y && x <= outer.x + outer.width && y <= outer.y + outer.height
}

impl Words {
    pub(super) fn drawn(elements: &[ElementInfo]) -> Self {
        let words = elements
            .iter()
            .filter(|element| !element.exiting && !element.focusable)
            .filter_map(|element| {
                let text = element.text.as_deref()?.trim();
                if text.is_empty() {
                    return None;
                }
                let control = control_of(elements, element)?;
                Some(Word {
                    control: control.key.clone()?,
                    at: bounds(control),
                    text: text.to_owned(),
                    bounds: bounds(element),
                })
            })
            .collect();
        Self(words)
    }

    pub(super) fn present(&self, snapshot: &mut Snapshot) {
        for index in 0..snapshot.len() {
            let Some(control) = snapshot.at(index) else {
                continue;
            };
            if !ONE_LEAF_NODES.contains(&control.role.as_str()) || !control.children.is_empty() {
                continue;
            }
            let (Some(key), Some(at)) = (control.key.as_deref(), control.bounds) else {
                continue;
            };
            let said: Vec<Seen> = self
                .0
                .iter()
                .filter(|word| word.control == key && contains_centre(at, word.at))
                .filter(|word| control.name.contains(word.text.as_str()))
                .map(|word| Seen {
                    role: "label".to_owned(),
                    name: word.text.clone(),
                    bounds: Some(word.bounds),
                    ..Seen::default()
                })
                .collect();
            for (place, seen) in said.into_iter().enumerate() {
                let segment = if place == 0 {
                    "@label".to_owned()
                } else {
                    format!("@label~{place}")
                };
                snapshot.push_under(index, &segment, seen);
            }
        }
    }
}

#[cfg(test)]
mod tests;
