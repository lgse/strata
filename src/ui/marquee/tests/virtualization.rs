// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::test_support::gtk_test;

#[test]
fn scrolling_out_of_bound_rows_preserves_and_retracts_their_selection() {
    gtk_test(
        "ui::marquee::tests::virtualization::scrolling_out_of_bound_rows_preserves_and_retracts_their_selection",
        || {
            let view = gtk::Fixed::new();
            let row = gtk::Label::new(Some("first"));
            row.set_size_request(200, 30);
            view.put(&row, 0.0, 0.0);
            view.allocate(200, 100, -1, None);
            let scroll = gtk::ScrolledWindow::builder().child(&view).build();
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&scroll));
            let bound = Rc::new(Cell::new(true));
            let bound_for_visit = bound.clone();
            let selection = gtk::MultiSelection::new(Some(gtk::StringList::new(&["first"])));
            let marquee = install(MarqueeSetup {
                view: view.clone().upcast(),
                surface: scroll.clone().upcast(),
                scroll,
                overlay,
                targets: Rc::new(RefCell::new(vec![MarqueeTarget {
                    selection: selection.clone(),
                    visit_items: Rc::new(move |visit| {
                        if bound_for_visit.get() {
                            visit(0, row.upcast_ref());
                        }
                    }),
                }])),
                is_item: Rc::new(|_, _, _| false),
                clear_selection: Rc::new(|| {}),
            });
            let state = &marquee.state;
            state.apply_selection(view.upcast_ref(), 0.0, 0.0, 200.0, 40.0);
            assert!(selection.is_selected(0));
            bound.set(false);
            state.apply_selection(view.upcast_ref(), 0.0, 0.0, 200.0, 100.0);
            assert!(
                selection.is_selected(0),
                "unbinding is not leaving the band"
            );
            state.apply_selection(view.upcast_ref(), 0.0, 40.0, 200.0, 100.0);
            assert!(
                !selection.is_selected(0),
                "retracting the band removes the old hit"
            );
            state.end();
            assert!(state.item_bounds.borrow().is_empty());
        },
    );
}
