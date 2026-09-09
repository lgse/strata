// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser_modes::BrowserMode;
use std::time::{Duration, Instant};

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "filtered chooser did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn find(widget: &gtk::Widget, predicate: &impl Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if predicate(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = find(&widget, predicate) {
            return Some(found);
        }
    }
    None
}

fn keys(widget: &impl IsA<gtk::Widget>) -> gtk::EventControllerKey {
    let controllers = widget.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("key controller")
}

fn press(
    keys: &gtk::EventControllerKey,
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
) -> bool {
    keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
}

#[test]
fn space_toggles_the_selected_search_result_in_open_and_save_choosers() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::filtered_preview::space_toggles_the_selected_search_result_in_open_and_save_choosers",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(root.path().join("folder")).expect("nested directory");
            std::fs::write(root.path().join("stale.txt"), "Wrong preview").expect("stale file");
            std::fs::write(root.path().join("folder/nested.txt"), "Nested preview")
                .expect("nested file");
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                ThemeManager::shared().set_browser_mode(mode);
                for save in [false, true] {
                    let request = ChooserRequest {
                        token: format!("preview-{mode:?}-{save}"),
                        title: "Filtered preview regression".into(),
                        accept_label: if save { "Save" } else { "Open" }.into(),
                        modal: false,
                        parent: None,
                        parent_size_hint: None,
                        initial_directory: root.path().into(),
                        kind: if save {
                            ChooserKind::SaveFile {
                                current_name: Some("untitled.txt".into()),
                            }
                        } else {
                            ChooserKind::Open {
                                directory: false,
                                multiple: false,
                            }
                        },
                        filters: Vec::new(),
                        current_filter: None,
                        choices: Vec::new(),
                    };
                    let state = build_chooser(request, Arc::new(AtomicBool::new(false)), |_| {})
                        .expect("chooser");
                    let browser = state.view.browser();
                    wait_until(|| {
                        browser
                            .column_snapshot(0)
                            .is_some_and(|column| !column.loading)
                    });
                    browser.select(0, 1);
                    assert!(state.view.show_filter_with_query("nested"));
                    let field = gtk::prelude::RootExt::focus(&state.window)
                        .expect("filter focus")
                        .ancestor(gtk::Entry::static_type())
                        .and_downcast::<gtk::Entry>()
                        .expect("filter entry");
                    let window_keys = keys(&state.window);
                    let filter_keys = keys(&field);
                    wait_until(|| {
                        find(state.window.upcast_ref(), &|widget| {
                            widget.is_mapped()
                                && widget
                                    .downcast_ref::<gtk::Label>()
                                    .is_some_and(|label| label.text() == "nested.txt")
                        })
                        .is_some()
                    });
                    field.grab_focus_without_selecting();
                    let results = find(state.window.upcast_ref(), &|widget| {
                        widget.is_mapped()
                            && widget.has_css_class("file-list")
                            && (widget.is::<gtk::ListView>() || widget.is::<gtk::ListBox>())
                    })
                    .expect("search listing");
                    if let Some(list) = results.downcast_ref::<gtk::ListView>() {
                        list.model().expect("selection model").unselect_all();
                    } else {
                        results
                            .downcast_ref::<gtk::ListBox>()
                            .expect("results list")
                            .unselect_all();
                    }
                    assert!(state.view.selected_search_result().is_none());
                    assert!(
                        !press(
                            &window_keys,
                            gtk::gdk::Key::space,
                            gtk::gdk::ModifierType::empty()
                        ),
                        "unselected query accepts spaces"
                    );
                    assert!(press(
                        &filter_keys,
                        gtk::gdk::Key::Down,
                        gtk::gdk::ModifierType::empty()
                    ));
                    assert_eq!(
                        state
                            .view
                            .selected_search_result()
                            .expect("search selection")
                            .location,
                        Location::local(root.path().join("folder/nested.txt"))
                    );
                    assert!(!press(
                        &window_keys,
                        gtk::gdk::Key::space,
                        gtk::gdk::ModifierType::SHIFT_MASK
                    ));
                    let capture = |name| {
                        if mode == BrowserMode::Columns
                            && !save
                            && let Some(output) =
                                std::env::var_os("STRATA_FILTER_PREVIEW_SCREENSHOTS")
                        {
                            super::sizing::capture(&state.window, Path::new(&output), name);
                        }
                    };
                    capture("before");
                    for open in [true, false] {
                        assert!(press(
                            &window_keys,
                            gtk::gdk::Key::space,
                            gtk::gdk::ModifierType::empty()
                        ));
                        wait_until(|| {
                            find(state.window.upcast_ref(), &|widget| {
                                widget.has_css_class("preview-pane")
                            })
                            .is_some()
                                == open
                        });
                        assert_eq!(field.text(), "nested");
                        assert_eq!(
                            state
                                .view
                                .selected_search_result()
                                .expect("preserved search selection")
                                .location,
                            Location::local(root.path().join("folder/nested.txt"))
                        );
                        assert!(state.view.filter_has_focus());
                        assert_eq!(
                            browser.active_location(),
                            Some(Location::local(root.path()))
                        );
                        assert!(state.completion.borrow().is_some());
                        if open {
                            let pane = find(state.window.upcast_ref(), &|widget| {
                                widget.has_css_class("preview-pane")
                            })
                            .expect("open preview");
                            wait_until(|| {
                                find(&pane, &|widget| {
                                    widget
                                        .downcast_ref::<gtk::Label>()
                                        .is_some_and(|label| label.text() == "nested.txt")
                                })
                                .is_some()
                            });
                            capture("after");
                        }
                    }
                    state.cancel();
                }
            }
        },
    );
}
