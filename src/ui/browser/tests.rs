// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

mod focus;
mod navigate;
mod recursive_search;
mod sidebar;

#[test]
fn global_activity_uses_the_latest_active_label() {
    let mut activity = GlobalActivityState::default();
    let connecting = activity.begin("Connecting…");
    let copying = activity.begin("Copying…");
    assert_eq!(activity.current_label(), Some("Copying…"));

    activity.finish(copying);
    assert_eq!(activity.current_label(), Some("Connecting…"));
    activity.finish(connecting);
    assert_eq!(activity.current_label(), None);
}

#[test]
fn vim_focus_keys_map_to_dialog_directions() {
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::h),
        Some(gtk::DirectionType::Left)
    );
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::j),
        Some(gtk::DirectionType::Down)
    );
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::k),
        Some(gtk::DirectionType::Up)
    );
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::l),
        Some(gtk::DirectionType::Right)
    );
    assert_eq!(vim_focus_direction(gtk::gdk::Key::Down), None);
}

#[test]
fn new_folder_prefers_the_focused_pane_then_falls_back_safely() {
    assert_eq!(new_folder_destination_depth(Some(1), Some(2), 3), Some(1));
    assert_eq!(new_folder_destination_depth(None, Some(2), 3), Some(2));
    assert_eq!(new_folder_destination_depth(Some(5), Some(1), 3), Some(1));
    assert_eq!(new_folder_destination_depth(None, None, 3), Some(2));
    assert_eq!(new_folder_destination_depth(None, None, 0), None);
}

#[test]
fn single_pane_modes_reserve_half_for_preview_sizing() {
    assert_eq!(single_pane_preview_reservation(800), 400);
    assert_eq!(single_pane_preview_reservation(0), 0);
}
