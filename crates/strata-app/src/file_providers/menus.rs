// SPDX-License-Identifier: MIT

//! Provider sections on the open context menu: building labelled provider groups from
//! [`MenuAction`] trees, refreshing the live [`MenuModel`] while preserving open submenu
//! navigation across branch renames and sibling changes, and keeping each leaf's latest
//! `context` token in place (`src/ui/file_providers/menus.rs`).

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use strata_components::context_menu::{ContextMenuState, Item, MenuModel, Section};
use strata_fs::services::file_providers::{self as protocol, MenuAction};

use super::Provider;

pub(super) type MenuKey = (Vec<String>, bool);
pub(super) type MenuResult = Vec<(usize, MenuAction)>;
pub(super) type Leaves = HashMap<(usize, String), Option<String>>;

pub(crate) const ACTION_PREFIX: &str = "provider:";

/// Tracks the one open context menu's provider sections and leaf `context` tokens.
#[derive(Default)]
pub(super) struct OpenMenu {
    key: Option<MenuKey>,
    sections: Vec<Section>,
    leaves: Leaves,
}

impl OpenMenu {
    pub(super) fn clear(&mut self) {
        self.key = None;
        self.sections.clear();
        self.leaves.clear();
    }

    pub(super) fn key(&self) -> Option<&MenuKey> {
        self.key.as_ref()
    }

    /// Starts watching a newly opened menu and returns its initial cached provider sections.
    pub(super) fn begin(
        &mut self,
        key: Option<MenuKey>,
        built: (Vec<Section>, Leaves),
    ) -> Vec<Section> {
        self.clear();
        let Some(key) = key else {
            return Vec::new();
        };
        let (sections, leaves) = built;
        self.key = Some(key);
        self.sections = sections.clone();
        self.leaves = leaves;
        sections
    }

    /// Updates leaf `context` tokens and, when presentation changed, splices the updated
    /// provider sections into `state` while retaining open submenu navigation.
    pub(super) fn refresh(
        &mut self,
        state: &mut ContextMenuState,
        provider_slot: usize,
        built: (Vec<Section>, Leaves),
    ) -> bool {
        let (sections, leaves) = built;
        // Always update leaf context tokens in place, even when labels and structure match.
        self.leaves = leaves;
        if sections_equal(&self.sections, &sections) {
            return false;
        }
        let Some(mut model) = state.model().cloned() else {
            return false;
        };
        let open_branches = open_branch_ids(&model, state.submenu());
        self.sections = sections.clone();
        splice_provider_sections(&mut model, provider_slot, sections);
        let reopened = resolve_branch_ids(&model, &open_branches);
        state.update_model(model);
        state.close_submenu(0);
        if !reopened.is_empty() {
            state.open_submenu(&reopened);
        }
        true
    }

    /// Resolves a clicked `"provider:<idx>:<id>"` action against the latest menu state.
    pub(super) fn take_activation(&mut self, action: &str) -> Option<ChosenAction> {
        let (provider, action_id) = parse_action_id(action)?;
        let (paths, background) = self.key.clone()?;
        let context = self.leaves.get(&(provider, action_id.clone()))?.clone();
        self.clear();
        Some(ChosenAction {
            provider,
            paths,
            background,
            action_id,
            context,
        })
    }
}

pub(super) struct ChosenAction {
    pub(super) provider: usize,
    pub(super) paths: Vec<String>,
    pub(super) background: bool,
    pub(super) action_id: String,
    pub(super) context: Option<String>,
}

/// Converts native `paths` into a validated [`MenuKey`], or `None` when ineligible.
pub(super) fn menu_key(paths: &[PathBuf], background: bool) -> Option<MenuKey> {
    if paths.is_empty() || paths.len() > protocol::PATH_LIMIT {
        return None;
    }
    let strings = paths
        .iter()
        .map(|p| p.to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    protocol::selection_fits(&strings).then_some((strings, background))
}

pub(crate) fn is_provider_action(action: &str) -> bool {
    action.starts_with(ACTION_PREFIX)
}

pub(crate) fn splice_provider_sections(
    model: &mut MenuModel,
    provider_slot: usize,
    sections: Vec<Section>,
) {
    model
        .sections
        .retain(|section| !is_provider_section(section));
    let slot = provider_slot.min(model.sections.len());
    model.sections.splice(slot..slot, sections);
}

fn is_provider_section(section: &Section) -> bool {
    section
        .items
        .iter()
        .any(|item| is_provider_action(&item.action))
}

fn parse_action_id(action: &str) -> Option<(usize, String)> {
    let rest = action.strip_prefix(ACTION_PREFIX)?;
    let (provider_str, id) = rest.split_once(':')?;
    let provider = provider_str.parse::<usize>().ok()?;
    (!id.is_empty()).then(|| (provider, id.to_owned()))
}

fn format_action_id(provider: usize, id: &str) -> String {
    format!("{ACTION_PREFIX}{provider}:{id}")
}

pub(super) fn build_sections_and_leaves(
    providers: &[Provider],
    result: MenuResult,
) -> (Vec<Section>, Leaves) {
    let mut grouped: BTreeMap<usize, Vec<MenuAction>> = BTreeMap::new();
    for (provider, action) in result {
        grouped.entry(provider).or_default().push(action);
    }
    let mut leaves = HashMap::new();
    let mut sections = Vec::with_capacity(grouped.len());
    for (provider, actions) in grouped {
        let Some(info) = providers.get(provider) else {
            continue;
        };
        let built: Vec<Item> = actions
            .iter()
            .map(|action| build_item(provider, action, &mut leaves))
            .collect();
        if !built.is_empty() {
            sections.push(Section::new(built).labelled(info.name.clone()));
        }
    }
    (sections, leaves)
}

fn build_item(provider: usize, action: &MenuAction, leaves: &mut Leaves) -> Item {
    let id = format_action_id(provider, &action.id);
    if let Some(children) = &action.children {
        let sub_items: Vec<Item> = children
            .iter()
            .map(|child| build_item(provider, child, leaves))
            .collect();
        Item::submenu(id, action.label.clone(), [Section::new(sub_items)])
    } else {
        leaves.insert((provider, action.id.clone()), action.context.clone());
        Item::new(id, action.label.clone())
    }
}

fn open_branch_ids(model: &MenuModel, submenu_path: &[usize]) -> Vec<String> {
    let mut ids = Vec::with_capacity(submenu_path.len());
    for depth in 1..=submenu_path.len() {
        let Some(item) = model.item_at(&submenu_path[..depth]) else {
            break;
        };
        ids.push(item.action.to_string());
    }
    ids
}

fn resolve_branch_ids(model: &MenuModel, branch_ids: &[String]) -> Vec<usize> {
    let mut path = Vec::with_capacity(branch_ids.len());
    let mut sections: &[Section] = &model.sections;
    for wanted in branch_ids {
        let found = sections
            .iter()
            .flat_map(|s| &s.items)
            .enumerate()
            .find(|(_, item)| &*item.action == wanted && item.submenu.is_some());
        let Some((index, item)) = found else {
            break;
        };
        path.push(index);
        let Some(sub) = item.submenu.as_deref() else {
            break;
        };
        sections = sub;
    }
    path
}

fn sections_equal(left: &[Section], right: &[Section]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.label == b.label && items_equal(&a.items, &b.items))
}

#[derive(PartialEq, Eq)]
struct ItemKey<'a> {
    action: &'a str,
    label: &'a str,
    icon: Option<strata_kit::icons::IconName>,
    shortcut: Option<&'a str>,
    description: Option<&'a str>,
    enabled: bool,
    danger: bool,
    checked: Option<bool>,
}

fn item_fields(item: &Item) -> ItemKey<'_> {
    ItemKey {
        action: &item.action,
        label: &item.label,
        icon: item.icon,
        shortcut: item.shortcut.as_deref(),
        description: item.description.as_deref(),
        enabled: item.enabled,
        danger: item.danger,
        checked: item.checked,
    }
}

fn submenus_equal(left: &Option<Vec<Section>>, right: &Option<Vec<Section>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(sa), Some(sb)) => sections_equal(sa, sb),
        _ => false,
    }
}

fn items_equal(left: &[Item], right: &[Item]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| {
            item_fields(a) == item_fields(b) && submenus_equal(&a.submenu, &b.submenu)
        })
}
