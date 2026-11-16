// SPDX-License-Identifier: MIT

//! Attaches `rune-kit` submenu surfaces (`*-sub-menu`), which mount as siblings on
//! `kit-root`'s overlay layer, under the `menu` surface of their owning `menu-item`,
//! matching GTK's nested `GtkPopoverMenu` hierarchy so `strata-qa`'s `open_menu` and
//! `Target::MenuItem` reach open submenus.

use super::tree::Seen;

const SUBMENU_SUFFIX: &str = "-sub-menu";

pub(super) fn nest(nodes: &mut [Seen]) {
    let subs: Vec<(usize, usize, usize)> = (0..nodes.len())
        .filter(|&sub| nodes[sub].role == "menu")
        .filter_map(|sub| submenu_reparent(nodes, sub))
        .collect();
    for (sub, old_parent, parent_menu) in subs {
        nodes[old_parent].children.retain(|&c| c != sub);
        nodes[sub].parent = Some(parent_menu);
        nodes[parent_menu].children.push(sub);
    }
}

fn submenu_reparent(nodes: &[Seen], sub: usize) -> Option<(usize, usize, usize)> {
    let old_parent = nodes[sub].parent?;
    let owner_key = nodes[sub].key.as_deref()?.strip_suffix(SUBMENU_SUFFIX)?;
    let owner_item = nodes
        .iter()
        .position(|n| n.key.as_deref() == Some(owner_key))?;
    let parent_menu = std::iter::successors(nodes[owner_item].parent, |&p| nodes[p].parent)
        .find(|&p| nodes[p].role == "menu")?;
    (old_parent != parent_menu).then_some((sub, old_parent, parent_menu))
}
