// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::time::Instant;

fn settle() {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn text_left(label: &gtk::Label, relative_to: &gtk::Widget) -> f32 {
    let (x, y) = label.layout_offsets();
    label
        .compute_point(relative_to, &gtk::graphene::Point::new(x as f32, y as f32))
        .expect("label position")
        .x()
}

#[test]
#[ignore = "requires a GTK display and isolated XDG directories; run this test alone"]
fn setup_copy_aligns_and_success_replaces_the_explanation() {
    gtk::init().expect("GTK display");
    gio::resources_register_include!("strata.gresource").expect("bundled icons");
    crate::ui::prepare_portal_ui();
    let display = gtk::gdk::Display::default().expect("display");
    let provider = gtk::CssProvider::new();
    provider.load_from_string(".portal-setup-dialog { font-size: 16px; }");
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    for offer in [true, false] {
        let overlay = gtk::Overlay::new();
        let window = gtk::Window::builder()
            .default_width(1000)
            .default_height(700)
            .child(&overlay)
            .build();
        window.present();
        let dialog = build_dialog(&window, offer).expect("setup dialog");
        dialog.message("Your current file chooser has not been changed. You can enable Strata now or later in Settings → General → System file chooser.", false);
        settle();
        let description = dialog.description.upgrade().expect("explanation");
        let status = dialog.status.upgrade().expect("status");
        let success = dialog.success.upgrade().expect("success icon");
        assert!(
            (text_left(&description, overlay.upcast_ref())
                - text_left(&status, overlay.upcast_ref()))
            .abs()
                <= 1.0,
            "both paragraphs share the same text inset"
        );
        assert!(description.is_mapped());
        assert!(!success.is_visible());
        let content = status.parent().expect("body").parent().expect("dialog");
        assert!(
            content.width() <= 760,
            "initial offer stays compact: {}",
            content.width()
        );
        dialog
            .confirm
            .upgrade()
            .expect("confirm")
            .set_label("Restore previous chooser");
        dialog.message("Strata is currently configured as your preferred file chooser. Restoring removes this integration and preserves unrelated configuration edits.", false);
        settle();
        assert!(
            content.width() <= 760,
            "configured Settings dialog stays compact: {}",
            content.width()
        );
        dialog.message("Setup failed; your previous chooser is unchanged.", true);
        assert!(status.has_css_class("error"));
        settle();
        assert!(content.width() <= 760);
        assert!(description.is_visible());
        assert!(!success.is_visible());

        dialog.complete(&format!(
            "Installed Strata as the per-user file chooser.\nConfiguration: /example/{}/portals.conf",
            "long-config-directory/".repeat(8),
        ));
        settle();
        assert!(!description.is_visible());
        assert!(success.is_mapped());
        assert!(
            content.width() <= 760,
            "long configuration paths wrap: {}",
            content.width()
        );
        let icon = success.paintable().expect("bundled success icon");
        crate::assets::set_primary_icon_color("#aabbcc");
        assert_ne!(success.paintable().expect("recolored success icon"), icon);
        crate::assets::set_primary_icon_color("#8bc9eb");
        assert!(!status.has_css_class("error"));
        assert!(status.text().contains("\nConfiguration:"));
        let icon_bounds = success.compute_bounds(&overlay).expect("icon bounds");
        let status_bounds = status.compute_bounds(&overlay).expect("status bounds");
        assert!(icon_bounds.y() + icon_bounds.height() <= status_bounds.y());
        assert!((icon_bounds.center().x() - status_bounds.center().x()).abs() <= 1.0);
        assert_eq!(status.justify(), gtk::Justification::Center);
        assert_eq!(
            dialog.confirm.upgrade().expect("done").label().as_deref(),
            Some("Done")
        );
        assert!(!dialog.cancel.upgrade().expect("cancel").is_visible());
        assert!(dialog.finished.get());
        window.destroy();
    }
    gtk::style_context_remove_provider_for_display(&display, &provider);
}
