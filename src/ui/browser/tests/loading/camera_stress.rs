// SPDX-License-Identifier: MIT

use super::*;

fn scroll_lists(widget: &impl IsA<gtk::Widget>, to_end: bool) {
    if let Some(scroll) = widget.as_ref().downcast_ref::<gtk::ScrolledWindow>() {
        let adjustment = scroll.vadjustment();
        adjustment.set_value(if to_end {
            adjustment.upper() - adjustment.page_size()
        } else {
            adjustment.lower()
        });
    }
    let mut child = widget.as_ref().first_child();
    while let Some(widget) = child {
        scroll_lists(&widget, to_end);
        child = widget.next_sibling();
    }
}

fn listing_scroll(widget: &impl IsA<gtk::Widget>) -> Option<gtk::ScrolledWindow> {
    if let Some(scroll) = widget.as_ref().downcast_ref::<gtk::ScrolledWindow>()
        && scroll.has_css_class("browser-listing-scroll")
        && scroll.is_mapped()
    {
        return Some(scroll.clone());
    }
    let mut child = widget.as_ref().first_child();
    while let Some(widget) = child {
        if let Some(scroll) = listing_scroll(&widget) {
            return Some(scroll);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn camera_batches_keep_the_top_until_the_user_scrolls() {
    crate::test_support::gtk_test(
        "ui::browser::tests::loading::camera_stress::camera_batches_keep_the_top_until_the_user_scrolls",
        || {
            for (mode, user_scroll) in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Columns]
                .into_iter()
                .flat_map(|mode| [false, true].map(|user_scroll| (mode, user_scroll)))
            {
                let source = Rc::new(HeldSource::default());
                let view = BrowserView::new(source.clone(), PeekBehavior::default());
                view.set_view_mode(mode);
                view.set_group_by_type(true);
                crate::ui::thumbnail::hold_thumbnail_workers();
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(700)
                    .default_height(500)
                    .build();
                window.present();
                let browser = view.browser();
                browser.navigate(Location::uri("gphoto2://camera/"));
                let publish = |start| {
                    let request = source.0.borrow();
                    let request = request.as_ref().expect("camera request");
                    (request.emit)(DirectoryEvent::Batch {
                        request_id: request.id,
                        entries: (start..start + 32)
                            .map(|index| {
                                let name = format!("IMG_{index:04}.JPG");
                                FileEntry {
                                    location: Location::uri(format!(
                                        "gphoto2://camera/DCIM/{name}"
                                    )),
                                    native_name: name.clone().into(),
                                    display_name: name,
                                    thumbnail_path: None,
                                    kind: crate::model::EntryKind::File,
                                    size: crate::model::MetadataValue::Known(100),
                                    modified_unix_seconds: crate::model::MetadataValue::Known(1),
                                    mode: crate::model::MetadataValue::Unavailable,
                                    image_dimensions: crate::model::MetadataValue::Unknown,
                                    child_count: crate::model::MetadataValue::Unknown,
                                    duration_seconds: crate::model::MetadataValue::Unknown,
                                    is_hidden: false,
                                }
                            })
                            .collect(),
                    });
                    settle();
                };
                settle();
                publish(128);
                let scroll = listing_scroll(&view.widget()).expect("camera viewport");
                let adjustment = scroll.vadjustment();
                assert!(
                    adjustment.value() < 1.0,
                    "{mode:?}: first batch starts at the top: {}",
                    adjustment.value()
                );
                publish(0);
                assert!(
                    adjustment.value() < 1.0,
                    "{mode:?}: a prepended batch must not pull the top down: {}",
                    adjustment.value()
                );
                assert!(browser.column_snapshot(0).expect("camera").loading);
                if user_scroll {
                    adjustment.set_value(adjustment.upper() - adjustment.page_size());
                    settle();
                    assert!(
                        adjustment.value() > 0.0,
                        "the user must be able to scroll during indexing"
                    );
                }
                let user_position = adjustment.value();
                publish(160);
                assert!(
                    (adjustment.value() - user_position).abs() < 1.0,
                    "{mode:?}: appended results must not move the user's viewport"
                );
                source.finish();
                settle();
                if user_scroll {
                    assert!(
                        adjustment.value() > 0.0,
                        "finishing must not pin the user back to the top"
                    );
                } else {
                    assert!(
                        adjustment.value() < 1.0,
                        "{mode:?}: finishing/grouping must keep the untouched viewport at the top: {}",
                        adjustment.value()
                    );
                }
                browser.clear_observer();
                crate::ui::thumbnail::cancel_thumbnails_in(&view.widget());
                window.destroy();
                crate::ui::thumbnail::clear_thumbnail_runtime();
            }
        },
    );
}

fn horizontal_scroll(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Adjustment> {
    if let Some(scroll) = widget.as_ref().downcast_ref::<gtk::ScrolledWindow>() {
        let adjustment = scroll.hadjustment();
        if adjustment.upper() > adjustment.page_size() {
            return Some(adjustment);
        }
    }
    let mut child = widget.as_ref().first_child();
    while let Some(widget) = child {
        if let Some(adjustment) = horizontal_scroll(&widget) {
            return Some(adjustment);
        }
        child = widget.next_sibling();
    }
    None
}

fn has_type_headers(widget: &impl IsA<gtk::Widget>) -> bool {
    if widget
        .as_ref()
        .downcast_ref::<gtk::ListView>()
        .is_some_and(|list| list.header_factory().is_some())
    {
        return true;
    }
    let mut child = widget.as_ref().first_child();
    while let Some(widget) = child {
        if has_type_headers(&widget) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

#[test]
fn camera_streaming_interleaved_names_survives_scrolling() {
    crate::test_support::gtk_test(
        "ui::browser::tests::loading::camera_stress::camera_streaming_interleaved_names_survives_scrolling",
        || {
            for mode in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Columns] {
                let source = Rc::new(HeldSource::default());
                let view = BrowserView::new(source.clone(), PeekBehavior::default());
                view.set_view_mode(mode);
                view.set_group_by_type(true);
                crate::ui::thumbnail::hold_thumbnail_workers();
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(if mode == BrowserMode::List { 560 } else { 900 })
                    .default_height(600)
                    .build();
                window.present();
                let browser = view.browser();
                browser.navigate(Location::uri("gphoto2://camera/"));
                settle();
                if mode == BrowserMode::List {
                    assert!(
                        !has_type_headers(&view.widget()),
                        "sectioned GTK lists must not receive live camera inserts"
                    );
                }
                for batch in 0..24 {
                    let entries = (0..256)
                        .map(|index| {
                            let extension = ["JPG", "HEIC", "MOV"][index % 3];
                            let name = format!("IMG_{index:04}.{extension}");
                            FileEntry {
                                location: Location::uri(format!(
                                    "gphoto2://camera/folder-{:03}/{name}",
                                    24 - batch
                                )),
                                native_name: name.clone().into(),
                                display_name: name,
                                thumbnail_path: None,
                                kind: crate::model::EntryKind::File,
                                size: crate::model::MetadataValue::Known(100),
                                modified_unix_seconds: crate::model::MetadataValue::Known(1),
                                mode: crate::model::MetadataValue::Unavailable,
                                is_hidden: false,
                                image_dimensions: crate::model::MetadataValue::Unknown,
                                child_count: crate::model::MetadataValue::Unknown,
                                duration_seconds: crate::model::MetadataValue::Unknown,
                            }
                        })
                        .collect();
                    let request = source.0.borrow();
                    let request = request.as_ref().expect("camera request");
                    (request.emit)(DirectoryEvent::Batch {
                        request_id: request.id,
                        entries,
                    });
                    scroll_lists(&view.widget(), batch % 2 == 0);
                    settle();
                    if batch == 0 {
                        browser.select(0, 10);
                    }
                }
                let horizontal = if mode == BrowserMode::List {
                    view.state.mode_views.borrow().focus_visible_pane(0);
                    settle();
                    let focused = view
                        .state
                        .mode_views
                        .borrow()
                        .focused_position()
                        .expect("focused camera row");
                    assert_eq!(
                        browser
                            .entry_at(focused.0, focused.1)
                            .expect("focused photo")
                            .location,
                        Location::uri("gphoto2://camera/folder-024/IMG_0010.HEIC")
                    );
                    let adjustment = horizontal_scroll(&view.widget()).expect("wide list columns");
                    adjustment.set_value(80.0_f64.min(adjustment.upper() - adjustment.page_size()));
                    settle();
                    Some((adjustment.clone(), adjustment.value()))
                } else {
                    None
                };
                source.finish();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
                while browser.column_snapshot(0).expect("camera root").loading {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "camera publication must drain"
                    );
                    settle();
                }
                settle();
                assert_eq!(browser.selected_entries().len(), 1);
                assert_eq!(
                    browser.selected_entries()[0].location,
                    Location::uri("gphoto2://camera/folder-024/IMG_0010.HEIC")
                );
                if mode == BrowserMode::List {
                    assert!(
                        has_type_headers(&view.widget()),
                        "saved grouping must return after discovery finishes"
                    );
                }
                assert_eq!(browser.column_snapshot(0).expect("camera root").count, 6144);
                if let Some((adjustment, offset)) = horizontal {
                    assert!(offset > 0.0);
                    assert!(
                        (adjustment.value() - offset).abs() < 1.0,
                        "grouping must preserve horizontal position"
                    );
                    let focused = view
                        .state
                        .mode_views
                        .borrow()
                        .focused_position()
                        .expect("focus survives grouping");
                    assert_eq!(
                        browser
                            .entry_at(focused.0, focused.1)
                            .expect("focused photo")
                            .location,
                        Location::uri("gphoto2://camera/folder-024/IMG_0010.HEIC")
                    );
                }
                if mode == BrowserMode::List {
                    browser.reload_active();
                    settle();
                    assert!(
                        !has_type_headers(&view.widget()),
                        "refresh must defer grouping again"
                    );
                    source.batch_at(Location::uri("gphoto2://camera/202606/reloaded.JPG"));
                    settle();
                    source.finish();
                    settle();
                    assert!(has_type_headers(&view.widget()));
                }
                browser.clear_observer();
                crate::ui::thumbnail::cancel_thumbnails_in(&view.widget());
                window.destroy();
                crate::ui::thumbnail::clear_thumbnail_runtime();
            }
        },
    );
}
