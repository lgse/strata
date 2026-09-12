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
            for (width, height) in [(1200, 800), (640, 480)] {
                window.set_default_size(width, height);
                for pixels in [8, 11, 17, 24, 32, 48, 13] {
                    manager.set_text_size(TextSize::new(pixels));
                    settle();
                    for page in [
                        "General",
                        "Theme & appearance",
                        "Keybindings",
                        "About",
                        "Updates",
                    ] {
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
                        let scroller = descendants(layer.upcast_ref())
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
                        for widget in
                            descendants(scroller.upcast_ref())
                                .into_iter()
                                .filter(|widget| {
                                    widget.is_mapped()
                                        && (widget.is::<gtk::Switch>()
                                            || widget.is::<gtk::Button>()
                                            || widget.has_css_class("settings-option"))
                                })
                        {
                            let bounds = widget.compute_bounds(&scroller).expect("control bounds");
                            assert!(
                                bounds.x() >= -1.0
                                    && bounds.x() + bounds.width() <= scroller.width() as f32 + 1.0,
                                "{page}, {pixels}px: {} extends outside the page: {bounds:?}",
                                widget.type_().name()
                            );
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
                .find(|button| button.tooltip_text().as_deref() == Some("Theme & appearance"))
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
                settle();
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
