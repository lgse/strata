// SPDX-License-Identifier: MIT

use super::*;

fn split_with(active: Side) -> PaneLayout {
    let mut layout = PaneLayout::default();
    layout.toggle();
    layout.activate(active);
    layout
}

#[test]
fn toggling_opens_the_extra_pane_active_and_closing_keeps_the_active_folder() {
    let mut layout = PaneLayout::default();
    assert!(!layout.split());
    assert_eq!(layout.other(), None);

    assert_eq!(layout.toggle(), Toggle::Opened);
    assert!(layout.split());
    assert_eq!(layout.active(), Side::Extra);
    assert_eq!(layout.other(), Some(Side::Main));

    for (active, adopt) in [(Side::Extra, true), (Side::Main, false)] {
        let mut layout = split_with(active);
        assert_eq!(layout.toggle(), Toggle::Closed { adopt }, "{active:?}");
        assert_eq!(layout, PaneLayout::default(), "{active:?}");
    }
}

#[test]
fn focus_moves_between_two_shown_panes_only() {
    let mut single = PaneLayout::default();
    assert!(!single.focus_other());
    assert!(
        !single.activate(Side::Extra),
        "a hidden pane cannot become active"
    );
    assert!(!single.activate(Side::Main), "the main pane already is");
    assert_eq!(single.active(), Side::Main);

    let mut split = split_with(Side::Extra);
    assert!(split.focus_other());
    assert_eq!(split.active(), Side::Main);
    assert_eq!(split.other(), Some(Side::Extra));
    assert!(split.focus_other());
    assert_eq!(split.active(), Side::Extra);
    assert!(!split.activate(Side::Extra));
    assert!(split.activate(Side::Main));
    assert_eq!(split.active(), Side::Main);
}

#[test]
fn transfers_go_to_the_other_panes_folder_when_it_takes_files() {
    let folder = Location::local("/data/projects");
    assert_eq!(
        transfer_destination(true, Some(folder.clone())),
        Ok(folder.clone())
    );
    let remote = Location::uri("sftp://host/srv/");
    assert_eq!(transfer_destination(true, Some(remote.clone())), Ok(remote));
    for (split, other, refusal) in [
        (false, Some(folder), "Open the other pane first"),
        (true, None, "The other pane has no folder"),
        (
            true,
            Some(Location::uri("trash:///")),
            "Can\u{2019}t copy or move items into this folder",
        ),
        (
            true,
            Some(Location::uri("recent:///")),
            "Can\u{2019}t copy or move items into this folder",
        ),
    ] {
        assert_eq!(transfer_destination(split, other), Err(refusal));
    }
}

#[test]
fn paned_separator_is_configured_with_resize_cursor_and_wide_handle() {
    if gtk::init().is_err() {
        return;
    }
    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.add_css_class("dual-pane");
    paned.set_wide_handle(true);
    crate::ui::window::sidebar_rail::configure_paned_handle(&paned, "col-resize");

    let separator =
        crate::ui::window::sidebar_rail::paned_separator(&paned).expect("separator widget");
    assert!(paned.is_wide_handle());
    assert_eq!(
        separator.cursor().and_then(|c| c.name()).as_deref(),
        Some("col-resize")
    );
}
