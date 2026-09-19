// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn entry(location: crate::model::Location) -> FileEntry {
    FileEntry {
        location,
        native_name: "file.txt".into(),
        thumbnail_path: None,
        display_name: "file.txt".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    }
}

#[test]
fn restore_flight_releases_only_when_every_entry_lives_in_trash() {
    let trashed = [
        entry(crate::model::Location::uri("trash:///a.txt")),
        entry(crate::model::Location::uri("trash:///b.txt")),
    ];
    assert_eq!(restore_flight(&trashed), Flight::Release);

    let restored = [entry(crate::model::Location::local("/home/user/a.txt"))];
    assert_eq!(restore_flight(&restored), Flight::Outbound);

    let mixed = [
        entry(crate::model::Location::uri("trash:///a.txt")),
        entry(crate::model::Location::local("/home/user/b.txt")),
    ];
    assert_eq!(restore_flight(&mixed), Flight::Outbound);
}

#[test]
fn outbound_flights_leave_restored_rows_visible() {
    crate::test_support::gtk_test(
        "ui::browser::fly_to_trash::tests::outbound_flights_leave_restored_rows_visible",
        || {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.add_css_class("list-row");
            let name_cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            name_cell.append(&gtk::Label::new(Some("file.txt")));
            row.append(&name_cell);
            let overlay = gtk::Overlay::builder().child(&row).build();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            assert!(find_row_by_name(row.upcast_ref(), "file.txt").is_some());
            let flyers = create_outbound_flyers(
                &overlay,
                row.upcast_ref(),
                &[entry(crate::model::Location::local("/fixture/file.txt"))],
                (0.0, 0.0),
            );
            assert_eq!(flyers.len(), 1);
            assert_eq!(row.opacity(), 1.0);
            let done = std::rc::Rc::new(std::cell::Cell::new(false));
            let completed = done.clone();
            animate_flyers(
                &overlay,
                row.upcast_ref(),
                flyers,
                Flight::Outbound,
                move || {
                    completed.set(true);
                },
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !done.get() {
                assert!(std::time::Instant::now() < deadline);
                glib::MainContext::default().iteration(false);
                assert_eq!(row.opacity(), 1.0);
                std::thread::sleep(Duration::from_millis(1));
            }
            window.destroy();
        },
    );
}

#[test]
fn overlapping_flights_preserve_the_latest_trash_probe() {
    crate::test_support::gtk_test(
        "ui::browser::fly_to_trash::tests::overlapping_flights_preserve_the_latest_trash_probe",
        || {
            let image = crate::assets::primary_icon(crate::assets::icons::TRASH, 18);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            content.append(&image);
            let button = gtk::Button::builder().child(&content).build();
            let first = open_trash_lid(&button);
            let second = open_trash_lid(&button);
            set_trash_icon(&image, crate::assets::icons::TRASH_FULL);
            close_trash_lid(first);
            assert_eq!(
                crate::assets::primary_icon_name(&image).as_deref(),
                Some("strata-trash-full-open")
            );
            close_trash_lid(second);
            assert_eq!(
                crate::assets::primary_icon_name(&image).as_deref(),
                Some(crate::assets::icons::TRASH_FULL)
            );
            assert!(TRASH_FLIGHTS.with(|flights| flights.borrow().is_empty()));
        },
    );
}

#[test]
fn release_end_rises_above_the_row_and_fans_out() {
    let row = (300.0, 400.0);
    let single = release_end(row, 0, 1);
    assert_eq!(single.0, row.0);
    assert!(single.1 < row.1);

    let ends: Vec<_> = (0..3).map(|index| release_end(row, index, 3)).collect();
    assert!(ends[0].0 < ends[1].0 && ends[1].0 < ends[2].0);
    assert_eq!(ends[1].0, row.0);
    for end in &ends {
        assert!(end.1 < row.1);
    }

    let near_top = release_end((300.0, 60.0), 0, 1);
    assert!(near_top.1 <= 60.0 - 90.0);
}
