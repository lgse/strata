// SPDX-License-Identifier: MIT

use rune::ElementInfo;
use strata_qa::Rect;

use super::tree::{Seen, Snapshot};

const FLASH: &str = "footer-flash";
const CHORD_PILL: &str = "footer-chord-pill";
const CHORD_OPTION: &str = "footer-chord-option-";
const RANGE: &str = "footer-range";
const TENXER: &str = "footer-tenxer";
const PROMPT_HINT: &str = "prompt-hint";

#[derive(Clone, Debug, PartialEq)]
struct Item {
    segment: String,
    name: String,
    bounds: Rect,
}

/// The 10xer footer labels drawn in the current frame.
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
            match element.key.as_deref() {
                Some(key @ (FLASH | CHORD_PILL | RANGE | TENXER)) => {
                    if let Some(name) = element.label.clone() {
                        items.push(Item {
                            segment: format!("@{key}"),
                            name,
                            bounds: bounds(element),
                        });
                    }
                }
                Some(PROMPT_HINT) => {
                    if let Some(name) = element.text.clone() {
                        items.push(Item {
                            segment: format!("@{PROMPT_HINT}"),
                            name,
                            bounds: bounds(element),
                        });
                    }
                }
                None => {
                    let parent_key = element
                        .parent
                        .and_then(|parent| elements.get(parent))
                        .and_then(|parent| parent.key.as_deref());
                    if let (Some(parent_key), Some(name)) = (parent_key, element.text.clone())
                        && parent_key.starts_with(CHORD_OPTION)
                    {
                        items.push(Item {
                            segment: format!("@{parent_key}-action"),
                            name,
                            bounds: bounds(element),
                        });
                    }
                }
                _ => {}
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
