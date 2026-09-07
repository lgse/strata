// SPDX-License-Identifier: GPL-3.0-or-later

use super::super::*;
use crate::services::{
    LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
};
use crate::ui::{shortcut_footer::ShortcutFooter, top_bar_navigation::TopBarNavigation};

pub(super) struct TextPreview;

impl PreviewProvider for TextPreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        glib::idle_add_local_once(move || {
            emit(PreviewEvent::Ready(Preview {
                request_id: request.id,
                entry: request.entry,
                content_type: "text/plain".into(),
                content: PreviewContent::Text {
                    content: "Space opens quick preview.\n".into(),
                    truncated: false,
                },
            }))
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn type_to_search_shortcuts_work_in_all_view_modes() {
    crate::test_support::gtk_test(
        "ui::window::tests::type_to_search::type_to_search_shortcuts_work_in_all_view_modes",
        exercise_type_to_search,
    );
}

fn exercise_type_to_search() {
    ThemeManager::seed_saved_preferences_for_test();
    load_styles();
    let preferences = ThemeManager::shared();
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::write(fixture.path().join("notes.txt"), b"preview fixture").expect("fixture file");
    std::fs::create_dir(fixture.path().join("folder")).expect("fixture directory");
    std::fs::write(fixture.path().join("archive.zip"), b"unsupported fixture")
        .expect("unsupported file");
    let view = BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default());
    view.set_single_click_previews(false);
    let browser = view.browser();
    let sidebar = build_sidebar(view.clone(), preferences.clone(), true);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let toggle = gtk::ToggleButton::new();
    let top_bar = TopBarNavigation::new(&header, &sidebar.widget, &toggle);
    let preview = PreviewDrawer::new(Rc::new(TextPreview), false);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&view.widget());
    content.append(&preview.widget());
    let window = gtk::ApplicationWindow::builder()
        .child(&content)
        .default_width(1000)
        .default_height(600)
        .build();
    let type_to_search = TypeToSearch {
        view: view.clone(),
        preferences: preferences.clone(),
    };
    keyboard::install(
        &window,
        &sidebar,
        keyboard::Bindings {
            view: view.clone(),
            top_bar,
            preview: preview.clone(),
            type_to_search,
            shortcuts: ShortcutFooter::new(BrowserMode::Columns),
        },
    );
    let keys = window
        .observe_controllers()
        .item(0)
        .and_downcast::<gtk::EventControllerKey>()
        .expect("keyboard controller");
    window.present();
    browser.navigate(Location::local(fixture.path()));
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });

    assert!(!preferences.type_to_search());
    for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
        view.set_view_mode(mode);
        browser.focus_active();
        press(&keys, gtk::gdk::Key::n);
        assert!(
            !view.filter_has_focus(),
            "saved disabled type-to-search: {mode:?}"
        );
    }

    for enabled in [true, false] {
        preferences.set_type_to_search(enabled);
        for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
            view.set_view_mode(mode);
            select_entry(&browser, "notes.txt");
            browser.focus_active();
            wait_until(|| view.item_view_has_focus());
            press(&keys, gtk::gdk::Key::space);
            assert!(
                preview.is_open(),
                "Space opens preview: {mode:?}, type-to-search={enabled}"
            );
            assert!(!view.filter_has_focus());
            press(&keys, gtk::gdk::Key::space);
            assert!(!preview.is_open(), "Space closes preview: {mode:?}");
            for name in ["folder", "archive.zip"] {
                select_entry(&browser, name);
                browser.focus_active();
                wait_until(|| view.item_view_has_focus());
                press(&keys, gtk::gdk::Key::space);
                assert!(
                    !preview.is_open(),
                    "Space must not preview {name}: {mode:?}, type-to-search={enabled}"
                );
                assert!(!view.filter_has_focus());
            }
        }
    }

    preferences.set_type_to_search(false);
    for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
        view.set_view_mode(mode);
        for (letter, arrow) in [
            (gtk::gdk::Key::h, gtk::gdk::Key::Left),
            (gtk::gdk::Key::j, gtk::gdk::Key::Down),
            (gtk::gdk::Key::k, gtk::gdk::Key::Up),
            (gtk::gdk::Key::l, gtk::gdk::Key::Right),
        ] {
            let mut results = Vec::new();
            for key in [arrow, letter] {
                browser.navigate(Location::local(fixture.path()));
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
                });
                select_entry(&browser, "folder");
                browser.focus_active();
                wait_until(|| view.item_view_has_focus());
                if !press(&keys, key) {
                    assert_eq!(key, arrow, "letter must be consumed: {mode:?}");
                    assert!(crate::ui::focus_navigation::activate_native_arrow(
                        &window, arrow
                    ));
                }
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                results.push((
                    browser.active_location(),
                    browser.focused_entry().map(|entry| entry.location),
                    view.header_actions_have_focus(),
                ));
                assert!(!view.filter_has_focus());
            }
            assert_eq!(
                results[0], results[1],
                "{letter:?} must mirror {arrow:?}: {mode:?}"
            );
        }
    }
    browser.navigate(Location::local(fixture.path()));
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    preferences.set_type_to_search(true);
    for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
        view.set_view_mode(mode);
        browser.focus_active();
        wait_until(|| view.item_view_has_focus());
        assert!(press(&keys, gtk::gdk::Key::slash));
        assert!(view.filter_has_focus(), "slash opens filter: {mode:?}");
        let focused = gtk::prelude::RootExt::focus(&window)
            .expect("filter focus")
            .downcast::<gtk::Text>()
            .expect("entry text");
        assert_eq!(focused.text(), "", "slash does not seed query: {mode:?}");
        assert!(press(&keys, gtk::gdk::Key::Escape));
    }

    browser.focus_active();
    wait_until(|| view.item_view_has_focus());
    press(&keys, gtk::gdk::Key::n);
    assert!(
        view.filter_has_focus(),
        "filename typing still starts filtering"
    );
    assert!(!preview.is_open());
    assert!(
        !press(&keys, gtk::gdk::Key::space),
        "Space in the filter must reach the text entry"
    );
    let focused = gtk::prelude::RootExt::focus(&window)
        .expect("filter focus")
        .downcast::<gtk::Text>()
        .expect("entry text");
    focused.emit_by_name::<()>("insert-at-cursor", &[&" "]);
    assert_eq!(focused.text(), "n ");
    browser.clear_observer();
    sidebar.disconnect();
    window.destroy();
}

fn select_entry(browser: &Browser, name: &str) {
    let count = browser.column_snapshot(0).expect("loaded column").count;
    let position = browser
        .with_entries(0, 0..count, |entries| {
            entries.iter().position(|entry| entry.display_name == name)
        })
        .flatten()
        .expect("fixture entry");
    browser.select(0, position);
}

fn press(keys: &gtk::EventControllerKey, key: gtk::gdk::Key) -> bool {
    keys.emit_by_name::<bool>(
        "key-pressed",
        &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
    )
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "browser did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}
