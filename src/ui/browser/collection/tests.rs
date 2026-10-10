// SPDX-License-Identifier: MIT

use super::*;
use std::time::Instant;

type BoundItem = (glib::WeakRef<gtk::ListItem>, glib::WeakRef<gtk::Button>);

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "collection request did not complete"
        );
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn grid_reveal_survives_reload_and_modal_focus_restoration() {
    crate::test_support::gtk_test(
        "ui::browser::collection::tests::grid_reveal_survives_reload_and_modal_focus_restoration",
        || {
            for closing_modal in [false, true] {
                let rows = Rc::new(RefCell::new(Vec::<BoundItem>::new()));
                let factory = gtk::SignalListItemFactory::new();
                factory.connect_setup(|_, item| {
                    let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                    item.set_child(Some(&gtk::Button::new()));
                });
                let bound = rows.clone();
                factory.connect_bind(move |_, item| {
                    let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                    let label = item
                        .item()
                        .and_downcast::<gtk::StringObject>()
                        .expect("entry");
                    let button = item
                        .child()
                        .and_downcast::<gtk::Button>()
                        .expect("entry button");
                    button.set_label(&label.string());
                    bound
                        .borrow_mut()
                        .push((item.downgrade(), button.downgrade()));
                });
                let entries = gtk::StringList::new(&["old.txt", "zzz.zip"]);
                let selection = gtk::MultiSelection::new(Some(entries.clone()));
                let grid = gtk::GridView::new(Some(selection.clone()), Some(factory));
                let scroll = gtk::ScrolledWindow::builder().child(&grid).build();
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&scroll));
                let window = gtk::Window::builder()
                    .child(&overlay)
                    .default_width(400)
                    .default_height(300)
                    .build();
                crate::ui::window::install_modal_focus_trap(&window);
                let fallback = grid.downgrade();
                crate::ui::modal::set_modal_focus_fallback(
                    &window,
                    Rc::new(move || {
                        if let Some(grid) = fallback.upgrade() {
                            grid.grab_focus();
                        }
                    }),
                );
                window.present();
                wait_until(|| grid.is_mapped());
                assert!(grid.grab_focus());
                let layer = closing_modal.then(|| {
                    let field = gtk::Entry::new();
                    let layer = crate::ui::modal::modal_layer(&field, &overlay, None, None);
                    crate::ui::modal::remember_modal_focus(&layer, &overlay);
                    overlay.add_overlay(&layer);
                    field.grab_focus();
                    layer
                });
                selection.set_model(None::<&gtk::StringList>);
                let visited = rows.clone();
                reveal_collection_after_layout(
                    grid.upcast_ref(),
                    1,
                    Rc::new(move |visit| {
                        for (item, button) in visited.borrow().iter() {
                            if let (Some(item), Some(button)) = (item.upgrade(), button.upgrade()) {
                                visit(item.position(), button.upcast_ref());
                            }
                        }
                    }),
                );
                selection.set_model(Some(&entries));
                selection.select_item(1, true);
                // Reload/dialog focus restoration still points at the preceding cursor.
                grid.scroll_to(0, gtk::ListScrollFlags::FOCUS, None);
                if let Some(layer) = layer.as_ref() {
                    crate::ui::modal::dismiss_modal_layer(layer, &overlay, None);
                    wait_until(|| layer.parent().is_none());
                }
                wait_until(|| {
                    let Some(focus) = gtk::prelude::RootExt::focus(&window) else {
                        return false;
                    };
                    rows.borrow().iter().any(|(item, button)| {
                        let (Some(item), Some(button)) = (item.upgrade(), button.upgrade()) else {
                            return false;
                        };
                        item.position() == 1
                            && (focus == button.clone().upcast::<gtk::Widget>()
                                || focus.is_ancestor(&button)
                                || button.parent().as_ref() == Some(&focus))
                    })
                });
                assert!(selection.is_selected(1));
                window.destroy();
            }
        },
    );
}
