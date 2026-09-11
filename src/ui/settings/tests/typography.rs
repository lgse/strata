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
fn settings_text_size_selector_places_the_value_between_native_step_buttons() {
    crate::test_support::gtk_test(
        "ui::settings::tests::typography::settings_text_size_selector_places_the_value_between_native_step_buttons",
        || {
            let manager = ThemeManager::shared();
            let (page, _) = theme_page(manager.clone());
            let control = descendants(&page)
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::SpinButton>().ok())
                .expect("text size control");
            let decrease = control.first_child().expect("decrease button");
            assert!(decrease.has_css_class("down"));
            let value = decrease.next_sibling().expect("numeric entry");
            assert!(value.is::<gtk::Text>());
            let increase = value.next_sibling().expect("increase button");
            assert!(increase.has_css_class("up"));
            assert_eq!(control.alignment(), 0.5);
            manager.set_text_size(TextSize::new(17));
            control.spin(gtk::SpinType::StepForward, 1.0);
            assert_eq!(manager.text_size(), TextSize::new(18));
            control.spin(gtk::SpinType::StepBackward, 1.0);
            assert_eq!(manager.text_size(), TextSize::new(17));
        },
    );
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
            for (width, height) in [(1200, 800), (640, 480)] {
                window.set_default_size(width, height);
                for pixels in [8, 11, 17, 24, 32, 48, 13] {
                    manager.set_text_size(TextSize::new(pixels));
                    settle();
                    for page in ["General", "Theme & appearance", "Keybindings", "About"] {
                        let navigation = descendants(layer.upcast_ref())
                            .into_iter()
                            .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                            .find(|button| button.tooltip_text().as_deref() == Some(page))
                            .expect("navigation button");
                        navigation.emit_clicked();
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
                assert!(control.is_mapped() && control.grab_focus());
                assert_eq!(control.value_as_int(), pixels as i32);
                let text = control
                    .first_child()
                    .expect("decrement")
                    .next_sibling()
                    .and_downcast::<gtk::Text>()
                    .expect("numeric entry");
                let start = text.compute_cursor_extents(0).0;
                let end = text.compute_cursor_extents(text.text().chars().count()).0;
                let center = (start.x() + end.x()) / 2.0;
                assert!(
                    (center - text.width() as f32 / 2.0).abs() <= 1.0,
                    "{pixels}px: number center {center}, entry width {}",
                    text.width()
                );
                let reset = widgets
                    .iter()
                    .filter_map(|widget| widget.downcast_ref::<gtk::Button>())
                    .find(|button| button.label().as_deref() == Some("Reset"))
                    .expect("reset");
                assert!(reset.has_css_class("action-dialog-cancel"));
                let label = reset
                    .child()
                    .and_downcast::<gtk::Label>()
                    .expect("reset label");
                assert!(!label.wraps());
                assert_eq!(label.layout().line_count(), 1);
                for grid in widgets
                    .iter()
                    .filter(|widget| !widget.has_css_class("text-size-actions"))
                    .filter_map(|widget| widget.downcast_ref::<gtk::FlowBox>())
                {
                    assert_eq!(grid.max_children_per_line(), 1);
                }
                assert!(
                    widgets
                        .iter()
                        .any(|widget| widget.has_css_class("settings-navigation")
                            && widget.has_css_class("compact"))
                );
            }
            window.destroy();
        },
    );
}
