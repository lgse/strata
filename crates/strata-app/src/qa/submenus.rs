// SPDX-License-Identifier: MIT

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
