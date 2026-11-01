// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn native_arrow_aliases_use_gtk_spatial_selection() {
    crate::test_support::gtk_test(
        "ui::focus_navigation::tests::native_arrow_aliases_use_gtk_spatial_selection",
        || {
            for grid in [false, true] {
                let model = gtk::StringList::new(&["a", "b", "c", "d", "e", "f", "g", "h", "i"]);
                let selection = gtk::SingleSelection::new(Some(model));
                let factory = gtk::SignalListItemFactory::new();
                factory.connect_setup(|_, object| {
                    let item = object.downcast_ref::<gtk::ListItem>().expect("list item");
                    let label = gtk::Label::new(Some("Item"));
                    label.set_size_request(80, 40);
                    item.set_child(Some(&label));
                });
                let collection: gtk::Widget = if grid {
                    gtk::GridView::builder()
                        .model(&selection)
                        .factory(&factory)
                        .min_columns(3)
                        .max_columns(3)
                        .build()
                        .upcast()
                } else {
                    gtk::ListView::new(Some(selection.clone()), Some(factory)).upcast()
                };
                let scroll = gtk::ScrolledWindow::builder().child(&collection).build();
                let window = gtk::Window::builder()
                    .child(&scroll)
                    .default_width(400)
                    .default_height(500)
                    .build();
                window.present();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while collection.width() == 0 {
                    glib::MainContext::default().iteration(false);
                    assert!(std::time::Instant::now() < deadline);
                }
                assert!(collection.grab_focus());
                assert_eq!(selection.selected(), 0);
                assert!(activate_native_arrow(&window, gdk::Key::Down));
                assert_eq!(selection.selected(), if grid { 3 } else { 1 });
                assert!(activate_native_arrow(&window, gdk::Key::Up));
                assert_eq!(selection.selected(), 0);
                if grid {
                    assert!(activate_native_arrow(&window, gdk::Key::Right));
                    assert_eq!(selection.selected(), 1);
                    assert!(activate_native_arrow(&window, gdk::Key::Left));
                    assert_eq!(selection.selected(), 0);
                }
                window.close();
            }
        },
    );
}

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

#[test]
#[ignore = "requires a GTK display; run this test alone"]
fn arrows_reach_a_notebook_tab_and_leave_tab_switching_to_gtk() {
    gtk::init().expect("GTK display");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let notebook = gtk::Notebook::new();
    let mut page_buttons = Vec::new();
    for title in ["General", "Script"] {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let button = gtk::Button::with_label(title);
        button.set_halign(gtk::Align::End);
        page.append(&button);
        notebook.append_page(&page, Some(&gtk::Label::new(Some(title))));
        page_buttons.push(button);
    }
    root.append(&close);
    root.append(&notebook);
    let window = gtk::Window::builder()
        .default_width(400)
        .child(&root)
        .build();
    window.present();
    let context = glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while page_buttons[0].width() == 0 {
        while context.pending() {
            context.iteration(false);
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let scope = root.upcast_ref::<gtk::Widget>();

    page_buttons[0].grab_focus();
    assert!(move_focus(scope, gtk::DirectionType::Up));
    assert!(notebook.has_focus());
    assert!(!move_focus(scope, gtk::DirectionType::Right));
    assert!(!move_focus(scope, gtk::DirectionType::Left));
    assert!(!activate(scope));
    assert!(move_focus(scope, gtk::DirectionType::Up));
    assert!(close.has_focus());
    assert!(move_focus(scope, gtk::DirectionType::Down));
    assert!(notebook.has_focus());
    assert!(move_focus(scope, gtk::DirectionType::Down));
    assert!(page_buttons[0].has_focus());

    notebook.set_show_tabs(false);
    while context.pending() {
        context.iteration(false);
    }
    page_buttons[0].grab_focus();
    assert!(move_focus(scope, gtk::DirectionType::Up));
    assert!(close.has_focus());
    window.destroy();
}
