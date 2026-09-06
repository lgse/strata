// SPDX-License-Identifier: GPL-3.0-or-later

use super::super::*;
use crate::services::{
    LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
};
use crate::ui::{shortcut_footer::ShortcutFooter, top_bar_navigation::TopBarNavigation};

struct TextPreview;

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
fn space_toggles_preview_without_starting_type_to_search() {
    const CHILD: &str = "STRATA_SPACE_PREVIEW_TEST_CHILD";
    if env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated preferences");
        let status = std::process::Command::new(env::current_exe().expect("test executable"))
            .args(["--exact", "ui::window::tests::type_to_search::space_toggles_preview_without_starting_type_to_search", "--nocapture"])
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .env("XDG_DATA_HOME", sandbox.path().join("data"))
            .status().expect("isolated GTK test");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    crate::assets::prepare().expect("assets");
    crate::assets::register_icon_theme();
    load_styles();
    let preferences = ThemeManager::shared();
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::write(fixture.path().join("notes.txt"), b"preview fixture").expect("fixture file");
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
    install_keyboard_navigation(
        &window,
        &view,
        &sidebar,
        &top_bar,
        &preview,
        &type_to_search,
        &ShortcutFooter::new(BrowserMode::Columns),
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

    for enabled in [true, false] {
        preferences.set_type_to_search(enabled);
        for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
            view.set_view_mode(mode);
            browser.select(0, 0);
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
        }
    }

    preferences.set_type_to_search(true);
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
