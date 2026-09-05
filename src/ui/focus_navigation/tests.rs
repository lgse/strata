// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn directional_neighbors_prefer_aligned_controls_and_exclude_the_opposite_direction() {
    let origin = gtk::graphene::Rect::new(100.0, 100.0, 40.0, 30.0);
    let right = gtk::graphene::Rect::new(160.0, 100.0, 40.0, 30.0);
    let diagonal = gtk::graphene::Rect::new(120.0, 160.0, 40.0, 30.0);
    assert!(directional_distance(&origin, &right, gtk::DirectionType::Left).is_none());
    assert!(directional_distance(&origin, &origin, gtk::DirectionType::Down).is_none());
    assert!(
        directional_distance(&origin, &right, gtk::DirectionType::Right)
            < directional_distance(&origin, &diagonal, gtk::DirectionType::Right)
    );
    assert!(directional_distance(&origin, &diagonal, gtk::DirectionType::Down).is_some());
}

#[test]
#[ignore = "requires a GTK display; run this test alone"]
fn arrows_and_enter_reach_controls_without_stealing_text_or_popover_keys() {
    gtk::init().expect("GTK display");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let first = gtk::Button::with_label("Refresh");
    let disabled = gtk::Button::with_label("Disabled");
    disabled.set_sensitive(false);
    let toggle = gtk::ToggleButton::with_label("Filter");
    let check = gtk::CheckButton::with_label("Compress files");
    let menu = gtk::MenuButton::builder().label("Encoding").build();
    let popover = gtk::Popover::new();
    let option = gtk::Button::with_label("UTF-8");
    popover.set_child(Some(&option));
    menu.set_popover(Some(&popover));
    for widget in [
        first.upcast_ref::<gtk::Widget>(),
        disabled.upcast_ref(),
        toggle.upcast_ref(),
        check.upcast_ref(),
        menu.upcast_ref(),
    ] {
        row.append(widget);
    }
    let entry = gtk::Entry::new();
    root.append(&row);
    root.append(&entry);
    let switch = gtk::Switch::new();
    switch.set_halign(gtk::Align::Start);
    root.append(&switch);
    let window = gtk::Window::builder()
        .default_width(800)
        .child(&root)
        .build();
    window.present();
    let context = glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while menu.width() == 0 {
        while context.pending() {
            context.iteration(false);
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    first.grab_focus();
    assert!(move_focus(root.upcast_ref(), gtk::DirectionType::Right));
    assert!(toggle.has_focus());
    assert!(!toggle.is_active());
    assert!(activate(root.upcast_ref()));
    assert!(toggle.is_active());
    assert!(move_focus(root.upcast_ref(), gtk::DirectionType::Right));
    assert!(check.has_focus());
    assert!(activate(root.upcast_ref()));
    assert!(check.is_active());
    assert!(move_focus(root.upcast_ref(), gtk::DirectionType::Right));
    assert!(move_focus(root.upcast_ref(), gtk::DirectionType::Down));
    assert!(popover.is_visible());
    option.grab_focus();
    assert!(!move_focus(root.upcast_ref(), gtk::DirectionType::Left));
    assert!(!activate(root.upcast_ref()));
    popover.popdown();
    entry.grab_focus();
    assert!(!move_focus(root.upcast_ref(), gtk::DirectionType::Up));
    assert!(!activate(root.upcast_ref()));
    switch.grab_focus();
    assert!(activate(root.upcast_ref()));
    assert!(switch.is_active());
    window.destroy();

    let overlay = gtk::Overlay::new();
    let settings = gtk::Box::new(gtk::Orientation::Vertical, 0);
    settings.add_css_class("app-modal-layer");
    let confirmation = gtk::Box::new(gtk::Orientation::Vertical, 0);
    confirmation.add_css_class("app-modal-layer");
    overlay.add_overlay(&settings);
    overlay.add_overlay(&confirmation);
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    assert_eq!(
        super::super::window::visible_modal_layer(&window),
        Some(confirmation.clone().upcast())
    );
    confirmation.set_visible(false);
    assert_eq!(
        super::super::window::visible_modal_layer(&window),
        Some(settings.upcast())
    );
    window.destroy();
}
