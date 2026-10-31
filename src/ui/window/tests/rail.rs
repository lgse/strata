// SPDX-License-Identifier: MIT

use super::super::{RailPolicy, SidebarFit};

const FITS: SidebarFit = SidebarFit {
    fits: true,
    preview: false,
};
const SHORT: SidebarFit = SidebarFit {
    fits: false,
    preview: false,
};
const SHORT_WITH_PREVIEW: SidebarFit = SidebarFit {
    fits: false,
    preview: true,
};

#[test]
fn space_compacts_once_per_change_and_releases_only_what_it_compacted() {
    let policy = RailPolicy::default();
    assert_eq!(policy.observe(FITS), None);
    assert_eq!(policy.observe(SHORT), Some(true));
    assert_eq!(policy.observe(SHORT), None, "same fit is not a new change");
    assert_eq!(policy.observe(FITS), Some(false));
    assert!(!policy.rail());
}

#[test]
fn expanding_by_choice_holds_until_space_changes_again() {
    let policy = RailPolicy::default();
    assert_eq!(policy.observe(SHORT), Some(true));
    assert_eq!(policy.set_compact(false), Some(false));
    assert_eq!(
        policy.observe(SHORT),
        None,
        "the same shortage must not re-rail"
    );
    assert!(!policy.rail());
    assert_eq!(
        policy.observe(SHORT_WITH_PREVIEW),
        Some(true),
        "a preview arriving is a new shortage"
    );
    assert_eq!(policy.observe(FITS), Some(false));
}

#[test]
fn compact_by_choice_ignores_space_until_expanded() {
    let policy = RailPolicy::default();
    assert_eq!(policy.set_compact(true), Some(true));
    assert_eq!(policy.observe(FITS), None);
    assert_eq!(policy.observe(SHORT), None);
    assert!(policy.rail());
    assert_eq!(
        policy.set_compact(true),
        None,
        "repeating the choice changes nothing"
    );
    assert_eq!(policy.set_compact(false), Some(false));
    assert_eq!(policy.observe(FITS), None);
    assert_eq!(policy.observe(SHORT), Some(true));
}

#[test]
fn choosing_compact_while_automatically_railed_keeps_the_rail_after_space_returns() {
    let policy = RailPolicy::default();
    assert_eq!(policy.observe(SHORT), Some(true));
    assert_eq!(policy.set_compact(true), None);
    assert_eq!(policy.observe(FITS), None);
    assert!(policy.rail());
}
