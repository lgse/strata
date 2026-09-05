// SPDX-License-Identifier: GPL-3.0-or-later

use std::{process::Command, rc::Rc};

use super::*;

const GTK_CHILD: &str = "STRATA_MARQUEE_GTK_CHILD";
const PIN_TEST: &str = "ui::marquee::tests::marquee_does_not_pin_the_collection_view";

#[test]
fn pointer_inside_the_viewport_does_not_scroll() {
    assert_eq!(auto_scroll_step(200.0, 400.0), 0.0);
    assert_eq!(auto_scroll_step(AUTO_SCROLL_MARGIN, 400.0), 0.0);
}

#[test]
fn pointer_near_an_edge_scrolls_towards_it() {
    assert!(auto_scroll_step(4.0, 400.0) < 0.0);
    assert!(auto_scroll_step(396.0, 400.0) > 0.0);
}

#[test]
fn scroll_speed_saturates_beyond_the_viewport() {
    assert_eq!(auto_scroll_step(-500.0, 400.0), -AUTO_SCROLL_MAX_STEP);
    assert_eq!(auto_scroll_step(900.0, 400.0), AUTO_SCROLL_MAX_STEP);
}

#[test]
fn short_viewports_never_auto_scroll() {
    assert_eq!(auto_scroll_step(0.0, AUTO_SCROLL_MARGIN), 0.0);
}

#[test]
fn band_starting_above_the_view_is_clipped_to_the_overlay() {
    assert_eq!(
        band_placement(10.0, -40.0, 100.0, 90.0, 400.0, 300.0),
        Some((10, 0, 100, 50))
    );
}

#[test]
fn band_extending_past_the_overlay_is_clipped_to_it() {
    assert_eq!(
        band_placement(350.0, 250.0, 200.0, 200.0, 400.0, 300.0),
        Some((350, 250, 50, 50))
    );
}

#[test]
fn band_entirely_outside_the_overlay_is_hidden() {
    assert_eq!(
        band_placement(-200.0, 10.0, 100.0, 50.0, 400.0, 300.0),
        None
    );
    assert_eq!(band_placement(10.0, 400.0, 100.0, 50.0, 400.0, 300.0), None);
}

#[test]
fn items_touching_the_band_are_captured() {
    let bounds = graphene::Rect::new(0.0, 100.0, 300.0, 24.0);
    assert!(intersects(&bounds, 10.0, 90.0, 40.0, 110.0));
    assert!(!intersects(&bounds, 10.0, 0.0, 40.0, 90.0));
}

#[test]
fn an_unmoved_band_still_captures_the_item_under_it() {
    let bounds = graphene::Rect::new(0.0, 100.0, 300.0, 24.0);
    assert!(intersects(&bounds, 20.0, 110.0, 20.0, 110.0));
}

fn assert_marquee_releases_the_collection_view() {
    let overlay = gtk::Overlay::new();
    let model = gtk::StringList::new(&["fv\talpha"]);
    let selection = gtk::NoSelection::new(Some(model));
    let list = gtk::ListView::new(Some(selection), Some(gtk::SignalListItemFactory::new()));
    let scroll = gtk::ScrolledWindow::builder().child(&list).build();
    overlay.set_child(Some(&scroll));
    let weak_list = list.downgrade();
    let weak_scroll = scroll.downgrade();
    let marquee = install(MarqueeSetup {
        view: list.clone().upcast(),
        scroll,
        overlay: overlay.clone(),
        targets: Rc::new(RefCell::new(Vec::new())),
        is_item: Rc::new(|_| false),
    });
    marquee.add_origin_surface(&overlay);
    drop(list);
    drop(marquee);
    overlay.set_child(None::<&gtk::Widget>);
    drop(overlay);
    while glib::MainContext::default().iteration(false) {}
    assert!(
        weak_list.upgrade().is_none(),
        "marquee must not pin the collection view after the pane drops"
    );
    assert!(
        weak_scroll.upgrade().is_none(),
        "marquee must not pin the scrolled window after the pane drops"
    );
}

#[test]
fn marquee_does_not_pin_the_collection_view() {
    if std::env::var_os(GTK_CHILD).is_some() {
        if gtk::init().is_err() {
            return;
        }
        assert_marquee_releases_the_collection_view();
        return;
    }

    let status = Command::new(std::env::current_exe().expect("test executable should exist"))
        .args(["--exact", PIN_TEST])
        .env(GTK_CHILD, "1")
        .status()
        .expect("isolated GTK marquee test should start");
    assert!(status.success(), "isolated GTK marquee test failed");
}
