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
            let theme = descendants(layer.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| button.tooltip_text().as_deref() == Some("Theme & appearance"))
                .expect("theme navigation button");
            theme.emit_clicked();
            for pixels in [24, 32, 48, 13] {
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
