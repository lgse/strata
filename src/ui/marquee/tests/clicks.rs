// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;

#[test]
fn single_selection_blocks_drag_selection_but_keeps_background_clearing() {
    gtk_test(
        "ui::marquee::tests::clicks::single_selection_blocks_drag_selection_but_keeps_background_clearing",
        || {
            let view = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let items = [gtk::Label::new(Some("one")), gtk::Label::new(Some("two"))];
            for item in &items {
                view.append(item);
            }
            let scroll = gtk::ScrolledWindow::builder().child(&view).build();
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&scroll));
            let selection = gtk::MultiSelection::new(Some(gtk::StringList::new(&["one", "two"])));
            let selection_to_clear = selection.clone();
            let allow_drag = Rc::new(Cell::new(false));
            let marquee = install(MarqueeSetup {
                view: view.upcast(),
                surface: scroll.clone().upcast(),
                scroll: scroll.clone(),
                overlay: overlay.clone(),
                targets: Rc::new(RefCell::new(vec![MarqueeTarget {
                    selection: selection.clone(),
                    visit_items: Rc::new(move |visit| {
                        for (position, item) in items.iter().enumerate() {
                            visit(position as u32, item.upcast_ref());
                        }
                    }),
                }])),
                is_item: Rc::new(|_, _, _| false),
                clear_selection: Rc::new(move || {
                    selection_to_clear.unselect_all();
                }),
                allow_drag: allow_drag.clone(),
            });
            let window = gtk::Window::builder()
                .child(&overlay)
                .default_width(320)
                .default_height(240)
                .build();
            window.present();
            scrolling::pump_until(|| scroll.is_mapped() && scroll.height() > 0);
            let state = &marquee.state;
            let start = (1.0, f64::from(scroll.height()) - 1.0);
            let offset = (f64::from(scroll.width()) - 2.0, -start.1);
            for multiple in [false, true] {
                allow_drag.set(multiple);
                selection.select_item(0, true);
                state.begin(start, gtk::gdk::ModifierType::empty());
                state.clear_at_press();
                assert!(
                    selection.selection().is_empty(),
                    "blank press clears in either policy"
                );
                assert_eq!(
                    state.update_drag(scroll.upcast_ref(), start, offset),
                    multiple
                );
                if multiple {
                    scrolling::pump_until(|| selection.selection().size() == 2);
                } else {
                    assert!(
                        selection.selection().is_empty(),
                        "drag must not select in single-selection mode"
                    );
                    assert!(
                        !state.band.is_visible(),
                        "single-selection mode must not show a drag box"
                    );
                }
                state.finish();
                scrolling::pump_until(|| !state.active.get());
                assert_eq!(selection.selection().size(), if multiple { 2 } else { 0 });
            }
            window.destroy();
        },
    );
}

#[test]
fn plain_background_gestures_clear_once() {
    gtk_test(
        "ui::marquee::tests::clicks::plain_background_gestures_clear_once",
        || {
            let view = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let scroll = gtk::ScrolledWindow::builder().child(&view).build();
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&scroll));
            let clears = Rc::new(Cell::new(0));
            let on_clear = clears.clone();
            let marquee = install(MarqueeSetup {
                view: view.clone().upcast(),
                surface: scroll.clone().upcast(),
                scroll: scroll.clone(),
                overlay: overlay.clone(),
                targets: Rc::new(RefCell::new(Vec::new())),
                is_item: Rc::new(|_, _, _| false),
                clear_selection: Rc::new(move || on_clear.set(on_clear.get() + 1)),
                allow_drag: Rc::new(Cell::new(true)),
            });
            let state = &marquee.state;
            state.begin((0.0, 0.0), gtk::gdk::ModifierType::empty());
            assert_eq!(clears.get(), 0, "begin only records gesture state");
            state.finish();
            assert_eq!(clears.get(), 1);
            state.finish();
            assert_eq!(clears.get(), 1, "unpaired release");

            for modifier in [
                gtk::gdk::ModifierType::CONTROL_MASK,
                gtk::gdk::ModifierType::SHIFT_MASK,
            ] {
                state.begin((0.0, 0.0), modifier);
                state.clear_at_press();
                assert_eq!(clears.get(), 1, "modified background press");
                state.finish();
                assert_eq!(clears.get(), 1, "modified background click");
            }

            state.begin((0.0, 0.0), gtk::gdk::ModifierType::empty());
            state.clear_on_click.set(false);
            state.finish();
            assert_eq!(
                clears.get(),
                1,
                "inert space inside an item still clicks the item"
            );

            state.begin((0.0, 0.0), gtk::gdk::ModifierType::empty());
            state.dragging.set(true);
            state.finish();
            assert_eq!(clears.get(), 1, "marquee release keeps its selection");

            state.begin((0.0, 0.0), gtk::gdk::ModifierType::empty());
            state.end();
            state.finish();
            assert_eq!(clears.get(), 1, "cancelled gesture");

            state.begin((0.0, 0.0), gtk::gdk::ModifierType::empty());
            state.clear_at_press();
            assert_eq!(clears.get(), 2, "press-time clear");
            state.finish();
            assert_eq!(clears.get(), 2, "release does not clear again");
        },
    );
}
