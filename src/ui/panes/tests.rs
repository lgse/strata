// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn suffixes_accept_case_and_arrow_aliases() {
    for (key, command) in [
        (
            gtk::gdk::Key::v,
            PaneCommand::Layout(PaneLayout::SideBySide),
        ),
        (
            gtk::gdk::Key::V,
            PaneCommand::Layout(PaneLayout::SideBySide),
        ),
        (gtk::gdk::Key::s, PaneCommand::Layout(PaneLayout::Stacked)),
        (gtk::gdk::Key::S, PaneCommand::Layout(PaneLayout::Stacked)),
        (gtk::gdk::Key::h, PaneCommand::Focus(PaneDirection::Left)),
        (gtk::gdk::Key::Left, PaneCommand::Focus(PaneDirection::Left)),
        (gtk::gdk::Key::j, PaneCommand::Focus(PaneDirection::Down)),
        (gtk::gdk::Key::Down, PaneCommand::Focus(PaneDirection::Down)),
        (gtk::gdk::Key::K, PaneCommand::Focus(PaneDirection::Up)),
        (gtk::gdk::Key::Up, PaneCommand::Focus(PaneDirection::Up)),
        (gtk::gdk::Key::L, PaneCommand::Focus(PaneDirection::Right)),
        (
            gtk::gdk::Key::Right,
            PaneCommand::Focus(PaneDirection::Right),
        ),
        (gtk::gdk::Key::w, PaneCommand::Toggle),
        (gtk::gdk::Key::C, PaneCommand::Close),
    ] {
        assert_eq!(pane_command_for_key(key), Some(command));
    }
    assert_eq!(pane_command_for_key(gtk::gdk::Key::q), None);
}

#[test]
fn prefix_cancels_on_escape_unknown_keys_and_timeout() {
    let now = Instant::now();
    let mut prefix = PanePrefix::default();

    prefix.begin(now);
    assert_eq!(
        prefix.suffix(now, gtk::gdk::Key::Escape),
        PrefixResult::Cancelled
    );
    prefix.begin(now);
    assert_eq!(prefix.suffix(now, gtk::gdk::Key::q), PrefixResult::Unknown);
    prefix.begin(now);
    assert_eq!(
        prefix.suffix(
            now + PANE_PREFIX_TIMEOUT + Duration::from_millis(1),
            gtk::gdk::Key::v
        ),
        PrefixResult::Inactive
    );
}

#[test]
fn splits_target_the_active_pane_and_stop_at_four() {
    let mut state = PaneState::default();

    assert!(state.split(PaneLayout::SideBySide, 1));
    assert_eq!(state.active, 1);
    assert!(state.split(PaneLayout::Stacked, 2));
    assert_eq!(state.pane_count(), 3);
    assert!(state.focus(PaneDirection::Up));
    assert_eq!(state.active, 1);
    assert!(state.focus(PaneDirection::Down));
    assert!(state.split(PaneLayout::SideBySide, 3));
    assert_eq!(state.pane_count(), 4);
    assert!(!state.split(PaneLayout::Stacked, 4));
}

#[test]
fn directional_focus_never_wraps_and_toggle_cycles_panes() {
    let mut state = PaneState::default();
    state.set_grid([0, 1, 2, 3]);

    assert!(state.focus(PaneDirection::Right));
    assert_eq!(state.active, 2);
    assert!(!state.focus(PaneDirection::Right));
    assert!(state.focus(PaneDirection::Down));
    assert_eq!(state.active, 3);
    assert!(state.focus(PaneDirection::Left));
    assert_eq!(state.active, 1);
    assert!(state.toggle());
    assert_eq!(state.active, 2);
    assert!(state.toggle());
    assert_eq!(state.active, 3);
    assert!(state.toggle());
    assert_eq!(state.active, 0);
}

#[test]
fn closing_collapses_the_split_and_the_last_pane_is_refused() {
    let mut state = PaneState::default();
    assert_eq!(state.close_active(), None);
    state.split(PaneLayout::SideBySide, 1);
    state.split(PaneLayout::Stacked, 2);
    assert_eq!(state.close_active(), Some(2));
    assert_eq!(state.active, 1);
    assert_eq!(state.pane_count(), 2);
    assert_eq!(state.close_active(), Some(1));
    assert_eq!(state, PaneState::default());
}

#[test]
fn single_keeps_the_active_pane_and_grid_reuses_existing_panes() {
    let mut state = PaneState::default();
    state.split(PaneLayout::SideBySide, 1);
    state.split(PaneLayout::Stacked, 2);
    assert_eq!(state.keep_active(), vec![0, 1]);
    assert_eq!(state.pane_ids(), vec![2]);

    state.set_grid([2, 3, 4, 5]);
    assert_eq!(state.layout(), PaneLayout::Grid);
    assert_eq!(state.pane_ids(), vec![2, 3, 4, 5]);
    assert_eq!(state.active, 2);
}

#[test]
fn four_panes_collapse_directly_to_the_requested_pair_layout() {
    let mut state = PaneState::default();
    state.set_grid([0, 1, 2, 3]);
    assert_eq!(state.keep_pair(PaneLayout::SideBySide), vec![1, 3]);
    assert_eq!(state.pane_ids(), vec![0, 2]);
    assert_eq!(state.layout(), PaneLayout::SideBySide);

    state.set_grid([0, 1, 2, 3]);
    state.active = 3;
    assert_eq!(state.keep_pair(PaneLayout::Stacked), vec![0, 1]);
    assert_eq!(state.pane_ids(), vec![2, 3]);
    assert_eq!(state.layout(), PaneLayout::Stacked);
    assert_eq!(state.active, 3);
}
