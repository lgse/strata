// SPDX-License-Identifier: MIT

//! Attaches `rune-kit` submenu surfaces (`*-sub-menu`), which mount as siblings on
//! `kit-root`'s overlay layer, under their owning `menu-item`, matching GTK's nested
//! `GtkPopoverMenu`, whose submenu popover is a child of the item that opens it. A walk of
//! the top menu then meets an open submenu's rows right after its trigger, as GTK's does, so
//! `strata-qa`'s `open_menu` lists them in GTK's order and `Target::MenuItem` reaches them.

use super::tree::Seen;

const SUBMENU_SUFFIX: &str = "-sub-menu";

pub(super) fn nest(nodes: &mut [Seen]) {
    let subs: Vec<(usize, usize, usize)> = (0..nodes.len())
        .filter(|&sub| nodes[sub].role == "menu")
        .filter_map(|sub| submenu_reparent(nodes, sub))
        .collect();
    for (sub, old_parent, owner_item) in subs {
        nodes[old_parent].children.retain(|&c| c != sub);
        nodes[sub].parent = Some(owner_item);
        nodes[owner_item].children.push(sub);
    }
}

fn submenu_reparent(nodes: &[Seen], sub: usize) -> Option<(usize, usize, usize)> {
    let old_parent = nodes[sub].parent?;
    let owner_key = nodes[sub].key.as_deref()?.strip_suffix(SUBMENU_SUFFIX)?;
    let owner_item = nodes
        .iter()
        .position(|n| n.key.as_deref() == Some(owner_key))?;
    (old_parent != owner_item).then_some((sub, old_parent, owner_item))
}
