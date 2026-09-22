// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{browser_modes::BrowserMode, preferences::PreferenceManager};
use std::time::{Duration, Instant};

fn widget_with_tooltip(widget: &gtk::Widget, tooltip: &str) -> Option<gtk::Widget> {
    if widget.tooltip_text().as_deref() == Some(tooltip) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = widget_with_tooltip(&widget, tooltip) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn widget_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = widget_with_class(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn focused_has_class(window: &gtk::Window, class: &str) -> bool {
    gtk::prelude::RootExt::focus(window).is_some_and(|focus| {
        let mut widget = Some(focus);
        while let Some(current) = widget {
            if current.has_css_class(class) {
                return true;
            }
            widget = current.parent();
        }
        false
    })
}

fn capture_keys(window: &gtk::Window) -> Vec<gtk::EventControllerKey> {
    let controllers = window.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .collect()
}

fn press(
    keys: &[gtk::EventControllerKey],
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
) -> bool {
    keys.iter()
        .any(|keys| keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]))
}

fn wait_for_listing(view: &crate::ui::browser::BrowserView, mode: BrowserMode) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !view
        .browser()
        .column_snapshot(0)
        .is_some_and(|column| !column.loading && column.count >= 2)
    {
        assert!(Instant::now() < deadline, "{mode:?} chooser loads");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    let initialized = Rc::new(Cell::new(false));
    let observed = initialized.clone();
    glib::idle_add_local_once(move || observed.set(true));
    super::acceptance::wait_until(|| initialized.get());
}

#[test]
fn chooser_follows_minimal_mode_chrome_and_keys() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::minimal_chrome::chooser_follows_minimal_mode_chrome_and_keys",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            assert!(
                PreferenceManager::shared().minimal_mode(),
                "saved fixture enables minimal mode"
            );
            let directory = tempfile::tempdir().expect("fixture");
            std::fs::write(directory.path().join("a.txt"), b"chooser").expect("fixture file");
            std::fs::write(directory.path().join("b.txt"), b"chooser").expect("fixture file");
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_type_to_search(false);
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                PreferenceManager::shared().set_minimal_mode(true);
                PreferenceManager::shared().set_browser_mode(mode);
                let accepted = Rc::new(RefCell::new(None));
                let observed = accepted.clone();
                let state = build_chooser(
                    super::acceptance::request(directory.path().into()),
                    Arc::new(AtomicBool::new(false)),
                    move |result| {
                        observed.replace(Some(result));
                    },
                )
                .expect("chooser with minimal dispatcher");
                let view = &state.view;
                let window = &state.window;
                wait_for_listing(view, mode);
                let refresh = widget_with_tooltip(&view.widget(), "Refresh (F5)")
                    .expect("chooser refresh exists");
                assert!(
                    !refresh.is_visible(),
                    "{mode:?} chooser refresh hides in minimal mode"
                );
                let filter_tooltip = match mode {
                    BrowserMode::Columns => "Filter this pane (Ctrl+F)",
                    BrowserMode::Icons => "Filter icons (Ctrl+F)",
                    BrowserMode::List => "Filter list (Ctrl+F)",
                };
                let filter = widget_with_tooltip(&view.widget(), filter_tooltip)
                    .expect("chooser filter exists");
                assert!(
                    !filter.is_visible(),
                    "{mode:?} chooser filter hides in minimal mode"
                );
                let min_tag = widget_with_class(window.upcast_ref(), "minimal-mode-tag")
                    .expect("chooser footer MIN tag");
                assert!(min_tag.is_visible(), "{mode:?} chooser shows MIN tag");
                view.browser().select(0, 0);
                view.browser().focus_active();
                super::acceptance::wait_until(|| view.item_view_has_focus());
                let keys = capture_keys(window);
                let start = view
                    .browser()
                    .focused_item()
                    .map(|(_, position, _)| position)
                    .expect("focused listing row");
                // Icons aliases native tile motion: two files share a row, so
                // l/h reverse. Columns and List stay linear on j/k.
                let (forward, back) = match mode {
                    BrowserMode::Icons => (gtk::gdk::Key::l, gtk::gdk::Key::h),
                    _ => (gtk::gdk::Key::j, gtk::gdk::Key::k),
                };
                assert!(press(&keys, forward, gtk::gdk::ModifierType::empty()));
                super::acceptance::wait_until(|| {
                    view.browser()
                        .focused_item()
                        .is_some_and(|(_, position, _)| position != start)
                });
                assert!(press(&keys, back, gtk::gdk::ModifierType::empty()));
                super::acceptance::wait_until(|| {
                    view.browser()
                        .focused_item()
                        .is_some_and(|(_, position, _)| position == start)
                });
                assert!(press(
                    &keys,
                    gtk::gdk::Key::f,
                    gtk::gdk::ModifierType::CONTROL_MASK
                ));
                assert!(
                    !view.filter_has_focus(),
                    "{mode:?} chooser Ctrl+F is page motion, not the pane filter"
                );
                view.browser().select(0, 0);
                view.browser().focus_active();
                super::acceptance::wait_until(|| view.item_view_has_focus());
                assert!(press(
                    &keys,
                    gtk::gdk::Key::f,
                    gtk::gdk::ModifierType::empty()
                ));
                super::acceptance::wait_until(|| focused_has_class(window, "minimal-prompt-entry"));
                assert!(press(
                    &keys,
                    gtk::gdk::Key::Escape,
                    gtk::gdk::ModifierType::empty()
                ));
                super::acceptance::wait_until(|| {
                    !focused_has_class(window, "minimal-prompt-entry")
                });
                assert!(
                    window.is_visible(),
                    "prompt Esc does not cancel the chooser"
                );
                view.browser().select(0, 0);
                view.browser().focus_active();
                super::acceptance::wait_until(|| view.item_view_has_focus());
                assert!(press(
                    &keys,
                    gtk::gdk::Key::q,
                    gtk::gdk::ModifierType::empty()
                ));
                assert!(
                    !PreferenceManager::shared().minimal_mode(),
                    "{mode:?} q leaves minimal mode"
                );
                assert!(window.is_visible(), "q does not cancel the chooser");
                super::acceptance::wait_until(|| refresh.is_visible());
                assert!(filter.is_visible(), "{mode:?} chrome returns after leaving");
                PreferenceManager::shared().set_minimal_mode(true);
                super::acceptance::wait_until(|| !refresh.is_visible());
                view.browser().select(0, 0);
                view.browser().focus_active();
                super::acceptance::wait_until(|| view.item_view_has_focus());
                assert!(press(
                    &keys,
                    gtk::gdk::Key::Return,
                    gtk::gdk::ModifierType::empty()
                ));
                super::acceptance::wait_until(|| accepted.borrow().is_some());
                let selected = accepted
                    .borrow_mut()
                    .take()
                    .expect("completed chooser")
                    .expect("accepted file");
                assert_eq!(
                    selected
                        .uris()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                    [gio::File::for_path(directory.path().join("a.txt"))
                        .uri()
                        .to_string()]
                );
                assert!(PreferenceManager::shared().minimal_mode());
                window.destroy();
            }
        },
    );
}

fn save_chooser(token: &str) -> (Rc<ChooserState>, tempfile::TempDir) {
    PreferenceManager::shared().set_minimal_mode(true);
    PreferenceManager::shared().set_browser_mode(BrowserMode::List);
    let directory = tempfile::tempdir().expect("chooser fixture");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(directory.path().join(name), b"chooser").expect("fixture file");
    }
    let mut request = super::acceptance::request(directory.path().to_path_buf());
    request.token = token.into();
    request.kind = ChooserKind::SaveFile {
        current_name: Some("suggested.txt".into()),
    };
    let state =
        build_chooser(request, Arc::new(AtomicBool::new(false)), |_| {}).expect("save chooser");
    wait_for_listing(&state.view, BrowserMode::List);
    (state, directory)
}

fn filename_has_focus(state: &ChooserState) -> bool {
    let Some(filename) = state.filename.as_ref() else {
        return false;
    };
    gtk::prelude::RootExt::focus(&state.window).is_some_and(|focus| {
        let entry = filename.upcast_ref::<gtk::Widget>();
        &focus == entry || focus.is_ancestor(entry)
    })
}

fn focus_filename(state: &ChooserState) {
    state.filename.as_ref().expect("filename").grab_focus();
    super::acceptance::wait_until(|| filename_has_focus(state));
}

fn focus_listing(state: &ChooserState) {
    state.view.browser().select(0, 0);
    state.view.browser().focus_active();
    state.view.restore_file_view_focus();
    super::acceptance::wait_until(|| state.view.item_view_has_focus());
}

fn dialog_open(state: &ChooserState) -> bool {
    state.completion.borrow().is_some() && state.window.is_visible()
}

fn selected_names(state: &ChooserState) -> Vec<String> {
    let mut names = state
        .view
        .browser()
        .selected_entries()
        .into_iter()
        .map(|entry| entry.display_name)
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn prompt_entry(window: &gtk::Window) -> gtk::Entry {
    widget_with_class(window.upcast_ref(), "minimal-prompt-entry")
        .and_downcast::<gtk::Entry>()
        .expect("minimal prompt entry")
}

fn preview_open(window: &gtk::Window) -> bool {
    widget_with_class(window.upcast_ref(), "preview-pane").is_some_and(|pane| pane.is_mapped())
}

#[test]
fn chooser_filename_escape_dismisses_before_cancel() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::minimal_chrome::chooser_filename_escape_dismisses_before_cancel",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            crate::ui::prepare_portal_ui();
            assert!(PreferenceManager::shared().minimal_mode());

            let (state, _directory) = save_chooser("filename-esc-filter");
            let keys = capture_keys(&state.window);
            focus_listing(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::f,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                focused_has_class(&state.window, "minimal-prompt-entry")
            });
            prompt_entry(&state.window).set_text("a.txt");
            super::acceptance::wait_until(|| state.view.hidden_filter_active());
            assert!(press(
                &keys,
                gtk::gdk::Key::Return,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                state.view.hidden_filter_active()
                    && !focused_has_class(&state.window, "minimal-prompt-entry")
            });
            focus_filename(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::Escape,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                !state.view.hidden_filter_active() && dialog_open(&state)
            });
            state.window.destroy();

            let (state, _directory) = save_chooser("filename-esc-find");
            let keys = capture_keys(&state.window);
            focus_listing(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::slash,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                focused_has_class(&state.window, "minimal-prompt-entry")
            });
            prompt_entry(&state.window).set_text("txt");
            assert!(press(
                &keys,
                gtk::gdk::Key::Return,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                state.view.find_highlights_active()
                    && !focused_has_class(&state.window, "minimal-prompt-entry")
            });
            focus_filename(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::Escape,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                !state.view.find_highlights_active() && dialog_open(&state)
            });
            state.window.destroy();

            let (state, _directory) = save_chooser("filename-esc-visual");
            let keys = capture_keys(&state.window);
            focus_listing(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::v,
                gtk::gdk::ModifierType::empty()
            ));
            assert!(press(
                &keys,
                gtk::gdk::Key::j,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| selected_names(&state).len() >= 2);
            let filled = selected_names(&state);
            focus_filename(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::Escape,
                gtk::gdk::ModifierType::empty()
            ));
            assert!(
                dialog_open(&state),
                "visual Esc must not cancel the chooser"
            );
            assert_eq!(selected_names(&state), filled);
            state.view.browser().focus_active();
            state.view.restore_file_view_focus();
            super::acceptance::wait_until(|| state.view.item_view_has_focus());
            let before = state
                .view
                .browser()
                .focused_item()
                .map(|(_, position, _)| position);
            assert!(press(
                &keys,
                gtk::gdk::Key::j,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                state
                    .view
                    .browser()
                    .focused_item()
                    .map(|(_, position, _)| position)
                    != before
            });
            assert_eq!(
                selected_names(&state),
                filled,
                "filename Esc must leave visual mode so the next j keeps the fill"
            );
            assert!(dialog_open(&state));
            state.window.destroy();

            let (state, _directory) = save_chooser("filename-esc-preview");
            let keys = capture_keys(&state.window);
            focus_listing(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::i,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| preview_open(&state.window));
            focus_filename(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::Escape,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| !preview_open(&state.window) && dialog_open(&state));
            state.window.destroy();

            let (state, _directory) = save_chooser("filename-esc-location");
            let keys = capture_keys(&state.window);
            focus_listing(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::l,
                gtk::gdk::ModifierType::CONTROL_MASK
            ));
            super::acceptance::wait_until(|| state.view.location_has_focus());
            assert!(press(
                &keys,
                gtk::gdk::Key::Escape,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                !state.view.location_has_focus() && dialog_open(&state)
            });
            state.window.destroy();

            let (state, _directory) = save_chooser("filename-esc-prompt");
            let keys = capture_keys(&state.window);
            focus_listing(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::f,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                focused_has_class(&state.window, "minimal-prompt-entry")
            });
            assert!(press(
                &keys,
                gtk::gdk::Key::Escape,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| {
                !focused_has_class(&state.window, "minimal-prompt-entry")
                    && !state.view.hidden_filter_active()
                    && dialog_open(&state)
            });
            state.window.destroy();

            let (state, _directory) = save_chooser("filename-esc-cancel");
            let keys = capture_keys(&state.window);
            focus_filename(&state);
            assert!(press(
                &keys,
                gtk::gdk::Key::Escape,
                gtk::gdk::ModifierType::empty()
            ));
            super::acceptance::wait_until(|| state.completion.borrow().is_none());
            state.window.destroy();
        },
    );
}
