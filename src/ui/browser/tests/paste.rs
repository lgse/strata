// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "paste timed out");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn has_scrolled_items(widget: &gtk::Widget) -> bool {
    if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>()
        && scroll.is_mapped()
        && scroll.vadjustment().value() > 0.0
    {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if has_scrolled_items(&widget) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

#[test]
fn paste_reveals_and_selects_completed_items_in_every_mode() {
    crate::test_support::gtk_test(
        "ui::browser::tests::paste::paste_reveals_and_selects_completed_items_in_every_mode",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                for (already_open, moving) in
                    [(false, false), (true, false), (false, true), (true, true)]
                {
                    let fixture = tempfile::tempdir().expect("paste fixture");
                    let root = Location::local(fixture.path());
                    let destination = Location::local(fixture.path().join("folder"));
                    std::fs::create_dir(fixture.path().join("folder")).expect("destination folder");
                    for index in 0..150 {
                        std::fs::write(
                            fixture.path().join(format!("folder/item-{index:03}")),
                            "old",
                        )
                        .expect("existing destination item");
                    }
                    let names = vec!["zz-first".to_owned(), "zz-second".to_owned()];
                    for name in &names {
                        std::fs::write(fixture.path().join(name), name).expect("source file");
                    }
                    let view = BrowserView::new(
                        Rc::new(crate::adapters::LocalFileSource),
                        PeekBehavior::default(),
                    );
                    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
                    view.set_view_mode(mode);
                    let browser = view.browser();
                    let window = gtk::Window::builder()
                        .child(&view.widget())
                        .default_width(1000)
                        .default_height(650)
                        .build();
                    window.present();
                    browser.navigate(root.clone());
                    wait_until(|| {
                        browser
                            .column_snapshot(0)
                            .is_some_and(|s| !s.loading && s.count == 3)
                    });
                    view.keyboard_navigation();
                    browser.select_entries_by_name(&names);
                    assert!(if moving {
                        view.cut_selection()
                    } else {
                        view.copy_selection()
                    });
                    browser.select_entries_by_name(&["folder".to_owned()]);
                    if already_open {
                        if mode == BrowserMode::Columns {
                            browser.descend(0, destination.clone());
                        } else {
                            browser.navigate(destination.clone());
                        }
                        wait_until(|| {
                            browser
                                .active_depth()
                                .and_then(|d| browser.column_snapshot(d))
                                .is_some_and(|s| !s.loading && s.count == 150)
                        });
                    }
                    view.paste();
                    wait_until(|| {
                        browser.active_location() == Some(destination.clone())
                            && browser
                                .selected_entries()
                                .iter()
                                .map(|e| e.display_name.clone())
                                .collect::<Vec<_>>()
                                == names
                    });
                    assert_eq!(
                        browser
                            .focused_entry()
                            .expect("first pasted item")
                            .display_name,
                        "zz-first"
                    );
                    wait_until(|| has_scrolled_items(&view.widget().upcast()));
                    if mode == BrowserMode::Columns {
                        assert_eq!(browser.location_at(0), Some(root.clone()));
                        assert_eq!(browser.location_at(1), Some(destination.clone()));
                        assert!(browser.location_at(2).is_none());
                    }
                    for name in &names {
                        assert_eq!(fixture.path().join(name).exists(), !moving);
                        assert_eq!(
                            std::fs::read_to_string(fixture.path().join("folder").join(name))
                                .expect("pasted contents"),
                            *name
                        );
                    }
                    window.destroy();
                    browser.clear_observer();
                }
            }
        },
    );
}
