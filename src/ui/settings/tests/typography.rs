// SPDX-License-Identifier: MIT

use super::super::*;
use crate::ui::{
    blur::BlurBin,
    theme::{TextSize, ThemeManager},
};
use gtk::glib;

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut found = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        found.extend(descendants(&widget));
    }
    found
}

fn settle() {
    let main_loop = glib::MainLoop::new(None, false);
    let stop = main_loop.clone();
    glib::timeout_add_local_once(Duration::from_millis(150), move || stop.quit());
    main_loop.run();
}

fn assert_key_and_filter_rows_wrap_only_when_needed(scroller: &gtk::ScrolledWindow) {
    for row in descendants(scroller.upcast_ref())
        .into_iter()
        .filter(|widget| widget.is_mapped())
        .filter_map(|widget| widget.downcast::<wrap::WrapRow>().ok())
    {
        let mut children = Vec::new();
        let mut child = row.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.is_mapped() {
                children.push(widget);
            }
        }
        let natural: i32 = children
            .iter()
            .map(|child| child.measure(gtk::Orientation::Horizontal, -1).1)
            .sum::<i32>()
            + row.spacing() * children.len().saturating_sub(1) as i32;
        let mut center: Option<f32> = None;
        for child in children {
            let bounds = child.compute_bounds(&row).expect("wrapped control bounds");
            assert!(
                bounds.x() >= -1.0 && bounds.x() + bounds.width() <= row.width() as f32 + 1.0,
                "wrapped control extends beyond its row: {bounds:?}"
            );
            if natural <= row.width() {
                let next = bounds.y() + bounds.height() / 2.0;
                if let Some(center) = center {
                    assert!(
                        (next - center).abs() <= 1.0,
                        "a fitting key/filter group must remain on one line"
                    );
                }
                center = Some(next);
            }
        }
    }
}

#[test]
fn settings_pages_reflow_without_horizontal_scrolling_as_text_grows() {
    crate::test_support::gtk_test(
        "ui::settings::tests::typography::settings_pages_reflow_without_horizontal_scrolling_as_text_grows",
        || {
            let manager = ThemeManager::shared();
            crate::ui::prepare_portal_ui();
            manager.set_text_size(TextSize::new(17));
            let button = gtk::Button::with_label("Settings");
            let root = BlurBin::new(&button);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder()
                .default_width(1200)
                .default_height(800)
                .child(&overlay)
                .build();
            let layer = build_layer(
                &button,
                &root,
                manager.clone(),
                Rc::new(|_| {}),
                install_guard(),
            );
            overlay.add_overlay(&layer);
            layer.set_visible(true);
            window.present();
            let stack = descendants(layer.upcast_ref())
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::Stack>().ok())
                .expect("settings pages");
            let responsive = descendants(layer.upcast_ref())
                .into_iter()
                .find_map(|widget| widget.downcast::<ResponsiveBin>().ok())
                .expect("responsive panel");
            for (width, height) in [
                (1600, 1100),
                (1200, 800),
                (1000, 800),
                (800, 560),
                (640, 480),
                (480, 560),
                (1600, 1100),
            ] {
                window.set_default_size(width, height);
                for pixels in [8, 11, 17, 24, 32, 48, 13] {
                    manager.set_text_size(TextSize::new(pixels));
                    settle();
                    for page in ["General", "Appearance", "Keybindings", "About", "Updates"] {
                        if page == "Updates" {
                            if stack.child_by_name("updates-test").is_none() {
                                let (updates, actions) = updates_page(
                                    manager.clone(),
                                    Rc::new(|_| {}),
                                    install_guard(),
                                    UpdateMethod::InPlace,
                                );
                                stack.add_named(&updates, Some("updates-test"));
                                for (row, button) in actions {
                                    responsive.add_action(row, button);
                                }
                            }
                            stack.set_visible_child_name("updates-test");
                        } else {
                            let navigation = descendants(layer.upcast_ref())
                                .into_iter()
                                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                                .find(|button| button.tooltip_text().as_deref() == Some(page))
                                .expect("navigation button");
                            navigation.emit_clicked();
                        }
                        settle();
                        let selected = stack.visible_child().expect("selected page");
                        let scroller = descendants(&selected)
                            .into_iter()
                            .filter(|widget| {
                                widget.is_mapped()
                                    && widget.has_css_class("settings-content-scroll")
                            })
                            .find_map(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
                            .expect("visible settings page");
                        let adjustment = scroller.hadjustment();
                        assert!(
                            adjustment.upper() <= adjustment.page_size() + 1.0,
                            "{page}, {pixels}px, {width}x{height}: horizontal extent {} > {}",
                            adjustment.upper(),
                            adjustment.page_size()
                        );
                        let panel = responsive.first_child().expect("settings panel");
                        let page_bounds = scroller.compute_bounds(&panel).expect("page bounds");
                        assert!(
                            page_bounds.x() + page_bounds.width() <= panel.width() as f32 + 1.0,
                            "{page}, {pixels}px, {width}x{height}: page extends beyond the panel"
                        );
                        assert_key_and_filter_rows_wrap_only_when_needed(&scroller);
                        for widget in
                            descendants(scroller.upcast_ref())
                                .into_iter()
                                .filter(|widget| {
                                    widget.is_mapped()
                                        && (widget.is::<gtk::Switch>()
                                            || widget.is::<gtk::Button>()
                                            || widget.has_css_class("settings-option")
                                            || widget.has_css_class("settings-keycap")
                                            || widget.has_css_class("about-detail-value")
                                            || widget.is::<gtk::Entry>()
                                            || widget.has_css_class("settings-control-label"))
                                })
                        {
                            let bounds = widget.compute_bounds(&scroller).expect("control bounds");
                            assert!(
                                bounds.x() >= -1.0
                                    && bounds.x() + bounds.width() <= scroller.width() as f32 + 1.0,
                                "{page}, {pixels}px: {} extends outside the page: {bounds:?}",
                                widget.type_().name()
                            );
                            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                                && label.has_css_class("settings-control-label")
                            {
                                assert_eq!(
                                    label.layout().line_count(),
                                    1,
                                    "{page}, {pixels}px, {width}x{height}: control label must remain readable: {}",
                                    label.text()
                                );
                            }
                        }
                    }
                }
            }
            window.destroy();
        },
    );
}

#[test]
fn custom_text_size_settings_remain_reachable_on_small_logical_displays() {
    crate::test_support::gtk_test(
        "ui::settings::tests::typography::custom_text_size_settings_remain_reachable_on_small_logical_displays",
        || {
            crate::ui::prepare_portal_ui();
            let manager = ThemeManager::shared();
            let button = gtk::Button::with_label("Settings");
            let root = BlurBin::new(&button);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder()
                .default_width(640)
                .default_height(480)
                .child(&overlay)
                .build();
            let layer = build_layer(
                &button,
                &root,
                manager.clone(),
                Rc::new(|_| {}),
                install_guard(),
            );
            overlay.add_overlay(&layer);
            layer.set_visible(true);
            window.present();
            manager.set_text_size(TextSize::new(32));
            settle();
            let theme = descendants(layer.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| button.tooltip_text().as_deref() == Some("Appearance"))
                .expect("theme navigation button");
            theme.emit_clicked();
            for pixels in [11, 17, 24, 32, 48, 13] {
                manager.set_text_size(TextSize::new(pixels));
                settle();
                let widgets = descendants(layer.upcast_ref());
                let panel = widgets
                    .iter()
                    .find(|widget| widget.has_css_class("settings-dialog"))
                    .expect("settings dialog");
                let bounds = panel.compute_bounds(&window).expect("settings bounds");
                assert_eq!((window.width(), window.height()), (640, 480));
                assert!(bounds.x() >= 0.0 && bounds.y() >= 0.0);
                assert!(
                    bounds.x() + bounds.width() <= 640.0 && bounds.y() + bounds.height() <= 480.0
                );
                let control = widgets
                    .iter()
                    .find_map(|widget| widget.downcast_ref::<gtk::SpinButton>())
                    .expect("text size control");
                assert!(theme.grab_focus());
                assert!(control.is_mapped() && control.grab_focus());
                assert_eq!(control.value_as_int(), pixels as i32);
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                loop {
                    settle();
                    let bounds = control
                        .compute_bounds(&window)
                        .expect("focused editor bounds");
                    if (bounds.y() >= 0.0 && bounds.y() + bounds.height() <= window.height() as f32)
                        || std::time::Instant::now() >= deadline
                    {
                        break;
                    }
                }
                let bounds = control
                    .compute_bounds(&window)
                    .expect("focused editor bounds");
                assert!(bounds.x() >= 0.0 && bounds.y() >= 0.0);
                assert!(
                    bounds.x() + bounds.width() <= window.width() as f32
                        && bounds.y() + bounds.height() <= window.height() as f32,
                    "focused text-size editor must remain visible at {pixels}px: {bounds:?}"
                );
            }
            window.destroy();
        },
    );
}
