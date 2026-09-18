// SPDX-License-Identifier: MIT

use super::*;
use crate::model::Location;

fn descendant<T: IsA<gtk::Widget> + glib::object::IsClass>(widget: &gtk::Widget) -> Option<T> {
    if let Ok(found) = widget.clone().downcast::<T>() {
        return Some(found);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = descendant(&widget) {
            return Some(found);
        }
    }
    None
}

fn press(widget: &impl IsA<gtk::Widget>, key: Key, modifiers: ModifierType) -> bool {
    let root = widget
        .as_ref()
        .root()
        .map(|root| root.upcast::<gtk::Widget>())
        .unwrap_or_else(|| widget.as_ref().clone());
    let controllers = root.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|i| {
            controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .filter(|keys| keys.propagation_phase() == gtk::PropagationPhase::Capture)
        .any(|keys| keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]))
}

fn wait_for(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "palette interaction completed"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn search(fixture: &Fixture, query: &str) -> gtk::Widget {
    fixture
        .window
        .lookup_action("command-palette")
        .expect("palette action")
        .activate(None);
    let layer = fixture
        .layer("command-palette-backdrop")
        .expect("palette layer");
    assert!(layer.is_visible());
    descendant::<gtk::Entry>(&layer)
        .expect("palette search field")
        .set_text(query);
    layer
}

#[test]
fn palette_keyboard_dismissal_modal_handoff_and_live_view_preferences() {
    gtk_test(
        "ui::window::composition::tests::palette::palette_keyboard_dismissal_modal_handoff_and_live_view_preferences",
        || {
            let fixture = Fixture::new();
            let second = Fixture::new();
            fixture.content.browser.begin_location_edit();
            let before = gtk::prelude::RootExt::focus(&fixture.window);
            assert!(press(
                &fixture.window,
                Key::P,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
            ));
            let layer = fixture
                .layer("command-palette-backdrop")
                .expect("palette layer");
            assert!(layer.is_visible());
            let field = descendant::<gtk::Entry>(&layer).expect("palette search field");
            field.set_text("no such command zzzzz");
            assert!(press(&layer, Key::Return, ModifierType::empty()));
            assert!(layer.is_visible());
            assert!(press(&layer, Key::Escape, ModifierType::empty()));
            assert_eq!(gtk::prelude::RootExt::focus(&fixture.window), before);
            fixture.content.browser.cancel_location_edit();

            for (query, mode) in [
                ("Switch to Icons", BrowserMode::Icons),
                ("Switch to List", BrowserMode::List),
                ("Switch to Columns", BrowserMode::Columns),
            ] {
                let layer = search(&fixture, query);
                press(&layer, Key::Return, ModifierType::empty());
                assert!(!layer.is_visible());
                assert_eq!(fixture.content.browser.view_mode(), mode);
                assert_eq!(second.content.browser.view_mode(), mode);
                assert_eq!(fixture.preferences.browser_mode(), mode);
            }
            let layer = search(&fixture, "preferences");
            press(&layer, Key::Return, ModifierType::empty());
            assert!(!layer.is_visible());
            assert!(
                fixture
                    .layer("settings-backdrop")
                    .expect("settings layer")
                    .is_visible()
            );
            fixture
                .window
                .lookup_action("command-palette")
                .expect("palette action")
                .activate(None);
            assert!(!layer.is_visible(), "palette does not stack above Settings");
            fixture.close();
            second.close();
        },
    );
}

#[test]
fn palette_file_commands_keep_selection_and_disabled_commands_do_not_run() {
    gtk_test(
        "ui::window::composition::tests::palette::palette_file_commands_keep_selection_and_disabled_commands_do_not_run",
        || {
            let fixture = Fixture::new();
            let directory = tempfile::tempdir().expect("fixture directory");
            std::fs::write(directory.path().join("chosen.txt"), b"chosen").expect("chosen fixture");
            std::fs::write(directory.path().join("other.txt"), b"other").expect("other fixture");
            let view = &fixture.content.browser;
            view.browser().navigate(Location::local(directory.path()));
            wait_for(|| {
                view.browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                super::super::super::apply_browser_mode(view, &fixture.preferences, mode);
                view.browser().select(0, 0);
                view.browser().focus_active();
                wait_for(|| view.item_view_has_focus());
                let selected = view.browser().selected_entries();
                let layer = search(&fixture, "copy path");
                press(&layer, Key::Return, ModifierType::empty());
                assert!(!layer.is_visible());
                let text = glib::MainContext::default()
                    .block_on(fixture.window.clipboard().read_text_future())
                    .expect("read clipboard")
                    .expect("copied path text");
                assert_eq!(
                    text.as_str(),
                    selected[0]
                        .location
                        .native_path()
                        .expect("local fixture path")
                        .to_str()
                        .expect("UTF-8 fixture path")
                );
                assert_eq!(
                    view.browser().selected_entries()[0].location,
                    selected[0].location
                );
                view.browser().clear_active_selection();
                let layer = search(&fixture, "duplicate");
                press(&layer, Key::Return, ModifierType::empty());
                assert!(layer.is_visible(), "unavailable command keeps palette open");
                press(&layer, Key::Escape, ModifierType::empty());
            }
            fixture.close();
        },
    );
}
