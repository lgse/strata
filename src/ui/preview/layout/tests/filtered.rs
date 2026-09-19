// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser_modes::BrowserMode;

fn results(widget: &gtk::Widget) -> Option<gtk::Widget> {
    if !widget.is_mapped() {
        return None;
    }
    if widget.has_css_class("file-list")
        && (widget.is::<gtk::ListView>() || widget.is::<gtk::ListBox>())
    {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(list) = results(&widget) {
            return Some(list);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn filtered_selection_follows_the_preview_in_browser_and_chooser_modes() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::filtered::filtered_selection_follows_the_preview_in_browser_and_chooser_modes",
        || {
            for chooser in [false, true] {
                for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                    let fixture = Fixture::new(chooser);
                    for name in ["matched-first.txt", "matched-second.txt"] {
                        std::fs::write(fixture.root.path().join(name), name).expect("file");
                    }
                    fixture.browser.set_view_mode(mode);
                    assert!(fixture.browser.show_filter_with_query("matched"));
                    wait_until(|| {
                        fixture
                            .browser
                            .selected_search_results()
                            .is_some_and(|_| results(&fixture.browser.widget()).is_some())
                    });
                    let list = results(&fixture.browser.widget()).expect("results");
                    let select = |position: u32| {
                        if let Some(list) = list.downcast_ref::<gtk::ListView>() {
                            let selection = list.model().expect("selection");
                            wait_until(|| selection.n_items() == 2);
                            list.grab_focus();
                            selection.select_item(position, true);
                        } else {
                            let list = list.downcast_ref::<gtk::ListBox>().expect("result list");
                            wait_until(|| list.row_at_index(1).is_some());
                            let row = list.row_at_index(position as i32).expect("result");
                            row.set_focusable(true);
                            row.grab_focus();
                            list.unselect_all();
                            list.select_row(Some(&row));
                        }
                    };
                    select(0);
                    let first = fixture
                        .browser
                        .selected_search_result()
                        .expect("first result");
                    fixture.preview.show(first.clone(), Some(0));
                    wait_until(|| !fixture.requests.borrow().is_empty());
                    select(1);
                    let second = fixture
                        .browser
                        .selected_search_result()
                        .expect("second result");
                    assert_ne!(first.location, second.location);
                    wait_until(|| {
                        fixture
                            .preview
                            .state
                            .current
                            .borrow()
                            .as_ref()
                            .is_some_and(|entry| entry.location == second.location)
                    });
                    select(0);
                    wait_until(|| {
                        fixture
                            .preview
                            .state
                            .current
                            .borrow()
                            .as_ref()
                            .is_some_and(|entry| entry.location == first.location)
                    });
                    fixture.preview.close();
                    select(1);
                    assert!(!fixture.preview.is_enabled());
                    fixture.window.destroy();
                }
            }
        },
    );
}
