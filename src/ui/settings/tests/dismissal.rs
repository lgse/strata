// SPDX-License-Identifier: MIT

use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gdk, glib, prelude::*};

use crate::ui::{blur::BlurBin, preferences::PreferenceManager, theme::ThemeManager};

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        widgets.extend(descendants(&current));
        child = current.next_sibling();
    }
    widgets
}

fn button_named(layer: &gtk::Widget, name: &str) -> gtk::Button {
    descendants(layer)
        .into_iter()
        .find(|widget| widget.widget_name() == name)
        .and_downcast::<gtk::Button>()
        .unwrap_or_else(|| panic!("{name} button"))
}

fn button_with_class(layer: &gtk::Widget, class: &str) -> gtk::Button {
    descendants(layer)
        .into_iter()
        .find(|widget| widget.has_css_class(class))
        .and_downcast::<gtk::Button>()
        .unwrap_or_else(|| panic!("{class} button"))
}

fn accent_picker(layer: &gtk::Widget) -> gtk::ColorDialogButton {
    descendants(layer)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ColorDialogButton>().ok())
        .find(|picker| {
            picker
                .next_sibling()
                .and_downcast::<gtk::Label>()
                .is_some_and(|label| label.text() == "Accent")
        })
        .expect("accent picker")
}

fn editor_revealer(layer: &gtk::Widget) -> gtk::Revealer {
    descendants(layer)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Revealer>().ok())
        .find(|revealer| {
            revealer
                .child()
                .is_some_and(|child| child.has_css_class("theme-editor"))
        })
        .expect("theme editor revealer")
}

fn settles(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        if Instant::now() >= deadline {
            return false;
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    true
}

#[derive(Clone, Copy, Debug)]
enum Close {
    Button,
    Escape,
    Backdrop,
    WindowDestroyed,
}

fn close(route: Close, layer: &gtk::Box, window: &gtk::Window) {
    match route {
        Close::Button => button_with_class(layer.upcast_ref(), "settings-close").emit_clicked(),
        Close::Escape => {
            let controllers = layer.observe_controllers();
            let handled = (0..controllers.n_items())
                .filter_map(|index| {
                    controllers
                        .item(index)
                        .and_downcast::<gtk::EventControllerKey>()
                })
                .any(|keys| {
                    keys.emit_by_name::<bool>(
                        "key-pressed",
                        &[&gdk::Key::Escape, &0u32, &gdk::ModifierType::empty()],
                    )
                });
            assert!(handled, "Settings handles Escape");
        }
        Close::Backdrop => {
            let controllers = layer.observe_controllers();
            let click = (0..controllers.n_items())
                .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
                .next()
                .expect("backdrop click gesture");
            click.emit_by_name::<()>("pressed", &[&1i32, &1.0f64, &1.0f64]);
        }
        Close::WindowDestroyed => window.destroy(),
    }
}

#[test]
fn closing_settings_cancels_the_theme_preview_and_collapses_the_editor() {
    crate::test_support::gtk_test(
        "ui::settings::tests::dismissal::closing_settings_cancels_the_theme_preview_and_collapses_the_editor",
        || {
            crate::ui::prepare_portal_ui();
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(false);
            manager.select_theme("azure-glow");
            let saved = manager.active_model_palette();
            let saved_accent = gdk::RGBA::parse(
                manager
                    .current_tokens()
                    .expect("selected theme tokens")
                    .accent
                    .as_str(),
            )
            .expect("saved accent");
            let mut failures = Vec::new();
            for (index, route) in [
                Close::Button,
                Close::Escape,
                Close::Backdrop,
                Close::WindowDestroyed,
            ]
            .into_iter()
            .enumerate()
            {
                manager.cancel_preview();
                manager.select_theme("azure-glow");
                let button = gtk::Button::with_label("Settings");
                let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
                content.append(&button);
                let root = BlurBin::new(&content);
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&root));
                let layer = super::super::build_layer(
                    &button,
                    &root,
                    PreferenceManager::shared(),
                    Rc::new(|_| {}),
                    super::super::install_guard(),
                );
                overlay.add_overlay(&layer);
                let window = gtk::Window::builder()
                    .child(&overlay)
                    .default_width(1100)
                    .default_height(760)
                    .build();
                window.present();
                layer.set_visible(true);
                assert!(settles(|| layer.is_mapped()));
                button_named(layer.upcast_ref(), "theme").emit_clicked();
                button_with_class(layer.upcast_ref(), "add-theme-card").emit_clicked();
                let revealer = editor_revealer(layer.upcast_ref());
                assert!(revealer.reveals_child());
                let preview = format!("#1357{:02x}", 0x10 + index);
                accent_picker(layer.upcast_ref())
                    .set_rgba(&gdk::RGBA::parse(preview.as_str()).expect("fixture color"));
                assert_ne!(manager.active_model_palette(), saved, "{route:?} previews");

                close(route, &layer, window.upcast_ref());
                if !matches!(route, Close::WindowDestroyed) {
                    assert!(
                        settles(|| !layer.is_visible()),
                        "{route:?}: Settings did not close"
                    );
                }
                if manager.is_previewing() || manager.active_model_palette() != saved {
                    failures.push(format!(
                        "{route:?}: the unsaved preview stays applied (accent {:06x}, saved {:06x})",
                        manager.active_model_palette().accent,
                        saved.accent
                    ));
                }
                if matches!(route, Close::WindowDestroyed) {
                    continue;
                }
                if revealer.reveals_child() {
                    failures.push(format!("{route:?}: the editor stays revealed"));
                }
                layer.set_visible(true);
                button_with_class(layer.upcast_ref(), "add-theme-card").emit_clicked();
                let reopened = accent_picker(layer.upcast_ref()).rgba();
                if reopened != saved_accent {
                    failures.push(format!(
                        "{route:?}: reopening Add theme shows {reopened} instead of the selected theme's {saved_accent}"
                    ));
                }
                window.destroy();
            }
            manager.cancel_preview();
            assert!(
                failures.is_empty(),
                "closing Settings must discard the theme preview:\n{}",
                failures.join("\n")
            );
        },
    );
}
