// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::test_support::gtk_test;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum Presentation {
    List,
    Wrapped,
}

#[test]
fn stationary_marquees_follow_native_and_wrapped_scrolling_after_layout() {
    gtk_test(
        "ui::marquee::tests::scrolling::stationary_marquees_follow_native_and_wrapped_scrolling_after_layout",
        || {
            for presentation in [Presentation::List, Presentation::Wrapped] {
                check_scrolling(presentation);
            }
        },
    );
}

fn check_scrolling(presentation: Presentation) {
    let names: Vec<_> = (0..400).map(|position| position.to_string()).collect();
    let model = gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let selection = gtk::MultiSelection::new(Some(model));
    let (view, visit) = collection(presentation, &selection);
    let scroll = gtk::ScrolledWindow::builder().child(&view).build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    let marquee = install(MarqueeSetup {
        view,
        surface: scroll.clone().upcast(),
        scroll: scroll.clone(),
        overlay: overlay.clone(),
        targets: Rc::new(RefCell::new(vec![MarqueeTarget {
            selection: selection.clone(),
            visit_items: visit.clone(),
        }])),
        is_item: Rc::new(|_, _, _| false),
        clear_selection: Rc::new(|| {}),
    });
    let window = gtk::Window::builder()
        .child(&overlay)
        .default_width(320)
        .default_height(320)
        .build();
    window.present();
    pump_until(|| scroll.vadjustment().upper() > 4000.0);
    let mut anchor = None;
    visit(&mut |position, widget| {
        if position == 4 {
            anchor = widget.compute_bounds(&scroll);
        }
    });
    let anchor = anchor.expect("bound anchor row");
    let start = (f64::from(anchor.x()) + 1.0, f64::from(anchor.y()) + 1.0);
    let pointer = (
        f64::from(scroll.width()) - 40.0,
        f64::from(scroll.height()) - 40.0,
    );
    let state = &marquee.state;
    state.begin(start, gtk::gdk::ModifierType::empty());
    state.dragging.set(true);
    state.drag_to(scroll.upcast_ref(), pointer);
    pump_until(|| selection.is_selected(4));
    let content_anchor = state.anchor.get();

    scroll.vadjustment().set_value(3000.0);
    pump_until(|| {
        let mut count = 0;
        let mut selected = true;
        visit(&mut |position, widget| {
            if !widget.is_mapped() {
                return;
            }
            let Some(bounds) = widget.compute_bounds(&scroll) else {
                return;
            };
            if position > 60
                && bounds.height() > 0.0
                && bounds.y() >= 0.0
                && f64::from(bounds.y() + bounds.height()) < pointer.1
                && f64::from(bounds.x()) < pointer.0
                && f64::from(bounds.x() + bounds.width()) > start.0
            {
                count += 1;
                selected &= selection.is_selected(position);
            }
        });
        count > 3 && selected
    });
    assert_eq!(state.anchor.get(), content_anchor);
    assert!(
        selection.is_selected(4),
        "scrolling must not erase the initial hit"
    );
    assert!(
        !selection.is_selected(0),
        "recycled row geometry must not select entries above the anchor"
    );
    assert_eq!(
        state.band.margin_top(),
        0,
        "the band is clipped to the viewport"
    );

    scroll.vadjustment().set_value(3100.0);
    state.finish();
    pump_until(|| !state.active.get());
    assert!(state.frame_handler.borrow().is_none());
    let finished = selection.selection().copy();
    scroll.vadjustment().set_value(0.0);
    pump_until(|| {
        let mut visible = false;
        visit(&mut |position, widget| {
            if position == 4 && widget.is_mapped() {
                visible = widget
                    .compute_bounds(&scroll)
                    .is_some_and(|bounds| bounds.y() >= 0.0);
            }
        });
        visible
    });
    assert!(
        selection.selection().equals(&finished),
        "scrolling after release is not a marquee"
    );
    window.close();
}

fn collection(
    presentation: Presentation,
    selection: &gtk::MultiSelection,
) -> (gtk::Widget, ItemVisitor) {
    if matches!(presentation, Presentation::Wrapped) {
        let view = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let mut rows = Vec::new();
        for position in 0..selection.n_items() {
            let row = gtk::Label::new(Some(&position.to_string()));
            row.set_size_request(80, 30);
            view.append(&row);
            rows.push(row);
        }
        return (
            view.upcast(),
            Rc::new(move |visit| {
                for (position, row) in rows.iter().enumerate() {
                    visit(position as u32, row.upcast_ref());
                }
            }),
        );
    }
    let rows = Rc::new(RefCell::new(Vec::<glib::WeakRef<gtk::ListItem>>::new()));
    let rows_for_setup = rows.clone();
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, object| {
        let item = object.downcast_ref::<gtk::ListItem>().expect("list item");
        let row = gtk::Label::new(Some("item"));
        row.set_size_request(80, 30);
        item.set_child(Some(&row));
        rows_for_setup.borrow_mut().push(item.downgrade());
    });
    let view = gtk::ListView::new(Some(selection.clone()), Some(factory)).upcast();
    (
        view,
        Rc::new(move |visit| {
            for row in rows.borrow().iter().filter_map(glib::WeakRef::upgrade) {
                if let Some(widget) = row.child() {
                    visit(row.position(), &widget);
                }
            }
        }),
    )
}

pub(super) fn pump_until(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "scroll layout and selection should settle"
        );
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(2));
    }
}
