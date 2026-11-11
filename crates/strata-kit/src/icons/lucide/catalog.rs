//! The icons `build.rs` generates from `crates/strata-kit/icons/`: [`LUCIDE`], [`MOCHA`] and
//! [`LATTE`], each sorted by `Icon::name`, and each icon's [`index`] in all three.

use std::sync::OnceLock;

use rune::Icon;

use super::super::IconName;

include!(concat!(env!("OUT_DIR"), "/catalog.rs"));

/// The icon `key` names in `all`: its `Icon::name`, else an [`IconName::id`], which finds
/// `semantic(name)`. A binary search, then one over the ids, so rows can look icons up while
/// they draw.
pub(in crate::icons) fn find(
    all: &'static [Icon],
    key: &str,
    semantic: fn(IconName) -> &'static Icon,
) -> Option<&'static Icon> {
    match all.binary_search_by(|icon| icon.name.cmp(key)) {
        Ok(i) => Some(&all[i]),
        Err(_) => by_id(key).map(semantic),
    }
}

/// The semantic name whose [`IconName::id`] is `key`.
fn by_id(key: &str) -> Option<IconName> {
    static IDS: OnceLock<Vec<(&'static str, IconName)>> = OnceLock::new();
    let ids = IDS.get_or_init(|| {
        let mut ids: Vec<_> = IconName::ALL.iter().map(|&n| (n.id(), n)).collect();
        ids.sort_unstable();
        ids
    });
    ids.binary_search_by(|(id, _)| (*id).cmp(key))
        .ok()
        .map(|i| ids[i].1)
}
