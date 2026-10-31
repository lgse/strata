// SPDX-License-Identifier: MIT

use super::SidebarMode;

#[test]
fn a_click_steps_from_full_to_rail_to_hidden_and_back() {
    let cases = [
        ((true, false), SidebarMode::Expanded, SidebarMode::Compact),
        ((true, true), SidebarMode::Compact, SidebarMode::Hidden),
        ((false, false), SidebarMode::Hidden, SidebarMode::Expanded),
        ((false, true), SidebarMode::Hidden, SidebarMode::Expanded),
    ];
    for ((visible, rail), mode, next) in cases {
        assert_eq!(SidebarMode::of(visible, rail), mode);
        assert_eq!(mode.next(), next);
    }
}
