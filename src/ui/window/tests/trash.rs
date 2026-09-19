// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn trash_drops_reject_empty_roots_and_already_trashed_sources() {
    use crate::ui::browser::BrowserView;

    assert!(BrowserView::can_trash_file_drop(&[
        Location::local("/home/user/first.txt"),
        Location::local("/home/user/second.txt"),
        Location::local("/home/user/third.txt"),
    ]));
    assert!(!BrowserView::can_trash_file_drop(&[]));
    assert!(!BrowserView::can_trash_file_drop(&[Location::local("/")]));
    assert!(!BrowserView::can_trash_file_drop(&[Location::uri(
        "trash:///"
    )]));
    assert!(!BrowserView::can_trash_file_drop(&[
        Location::local("/home/user/first.txt"),
        Location::uri("trash:///second.txt"),
    ]));
}

#[test]
fn the_empty_trash_row_and_its_separator_appear_only_for_confirmed_non_empty_trash() {
    assert_eq!(
        trash_menu_visibility(TrashContents::NonEmpty(1)),
        TrashMenuVisibility {
            separator: true,
            empty: true,
        }
    );
    assert_eq!(
        trash_menu_visibility(TrashContents::Empty),
        TrashMenuVisibility {
            separator: false,
            empty: false,
        }
    );
    assert_eq!(
        trash_menu_visibility(TrashContents::Unknown),
        TrashMenuVisibility {
            separator: false,
            empty: false,
        }
    );
}

#[test]
fn trash_probe_results_map_to_menu_state() {
    assert_eq!(trash_contents_from_probe(Ok(3)), TrashContents::NonEmpty(3));
    assert_eq!(trash_contents_from_probe(Ok(0)), TrashContents::Empty);
    assert_eq!(
        trash_contents_from_probe(Err(glib::Error::new(
            gtk::gio::IOErrorEnum::NotSupported,
            "trash backend unavailable",
        ))),
        TrashContents::Unknown
    );
}

#[test]
fn trash_mutating_operations_refresh_the_context_menu() {
    assert!(event_changes_trash_contents(
        &BrowserEvent::DeletionFinished { succeeded: true }
    ));
    assert!(event_changes_trash_contents(
        &BrowserEvent::RestorationFinished
    ));
    assert!(event_changes_trash_contents(
        &BrowserEvent::TransferFinished {
            moved_locations: Vec::new(),
        }
    ));
    assert!(!event_changes_trash_contents(&BrowserEvent::Reset));
    assert!(!event_changes_trash_contents(
        &BrowserEvent::HiddenToggled { show_hidden: true }
    ));
}

#[test]
fn the_trash_probe_counts_entries_up_to_the_icon_cap() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = gtk::gio::File::for_path(fixture.path());

    let empty = glib::MainContext::new()
        .block_on(trash_item_count(&root))
        .expect("an empty directory should enumerate");
    assert_eq!(empty, 0);

    for index in 0..3 {
        std::fs::write(fixture.path().join(format!("note{index}.txt")), b"trashed")
            .expect("fixture entry");
    }
    let count = glib::MainContext::new()
        .block_on(trash_item_count(&root))
        .expect("a populated directory should enumerate");
    assert_eq!(count, 3);

    let missing = glib::MainContext::new().block_on(trash_item_count(&gtk::gio::File::for_path(
        fixture.path().join("absent"),
    )));
    assert!(missing.is_err());
}
