// SPDX-License-Identifier: MIT

//! The words inside a control that Rune hears as one thing, as the QA backend presents them.
//!
//! Since Rune `7a707454` (anomalyco/rune#1115, after #1099) the accessibility tree leaves out
//! a text inside a checkbox, radio button, switch, tab, menu item or option when the control's
//! name already says its words, as Chromium makes such a control's content one leaf. GTK keeps
//! them: each of these rows holds a `gtk::Label` (`role: "label"`), which shared scenarios aim
//! at, such as the language menu's autonyms and the search palette's names and folders. The
//! snapshot presents each such text as a `label` under its control, as the tree held it at
//! Rune `631e8a67`, without changing what assistive technology hears.
//!
//! Buttons and links are left as they are: the tree made them one leaf before, and the snapshot
//! presents the labels a scenario needs under them on its own (`label_archive_text_buttons`).

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

/// The one-leaf control `element` is drawn in, if a nearer focusable element, button or link
/// does not hold it first.
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

    /// Presents the words under each one-leaf control that has no children in the snapshot and
    /// whose name says them: the texts the tree left out.
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
