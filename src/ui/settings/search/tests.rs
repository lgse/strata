// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{blur::BlurBin, theme::ThemeManager};

#[test]
fn ranks_exact_labels_aliases_and_small_typing_errors() {
    for (query, page, id) in [
        ("Folder peeking", "general", "peeking"),
        ("tezt size", "theme", "text"),
        ("font size", "theme", "text"),
        ("nightly", "updates", "channel"),
        ("relase chanel", "updates", "channel"),
        ("copyright", "about", "license"),
        ("rename", "keybindings", "shortcuts"),
    ] {
        let matches = find_matches(&normalized(query));
        assert_eq!(matches.best_page, Some(page), "{query}");
        assert!(matches.ids.contains(id), "{query}");
    }
    assert!(find_matches("unfindablequantumsetting").ids.is_empty());
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    for child in children(widget) {
        widgets.extend(descendants(&child));
    }
    widgets
}

fn item(layer: &gtk::Widget, id: &str) -> gtk::Widget {
    descendants(layer)
        .into_iter()
        .find(|widget| widget.widget_name() == format!("settings-search-{id}"))
        .expect("searchable setting")
}

#[test]
fn global_search_navigates_filters_lazy_pages_and_restores_without_editing_preferences() {
    crate::test_support::gtk_test(
        "ui::settings::search::tests::global_search_navigates_filters_lazy_pages_and_restores_without_editing_preferences",
        || {
            crate::ui::prepare_portal_ui();
            let manager = ThemeManager::shared();
            let original = manager.folder_peeking();
            let button = gtk::Button::with_label("Settings");
            let root = BlurBin::new(&button);
            let layer = super::super::build_layer(
                &button,
                &root,
                manager.clone(),
                Rc::new(|_| {}),
                super::super::install_guard(),
            );
            layer.set_visible(true);
            let entry = descendants(layer.upcast_ref())
                .into_iter()
                .find(|widget| widget.has_css_class("settings-global-search-entry"))
                .and_then(|widget| widget.downcast::<gtk::Entry>().ok())
                .expect("global search");
            let stack = descendants(layer.upcast_ref())
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::Stack>().ok())
                .expect("settings pages");
            assert!(stack.child_by_name("theme").is_none());
            entry.set_text("folder peeking");
            assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
            assert!(item(layer.upcast_ref(), "peeking").is_visible());
            assert!(!item(layer.upcast_ref(), "previews").is_visible());
            entry.set_text("tezt size");
            assert_eq!(stack.visible_child_name().as_deref(), Some("theme"));
            assert!(item(layer.upcast_ref(), "text").is_visible());
            assert!(!item(layer.upcast_ref(), "motion").is_visible());
            assert!(!item(layer.upcast_ref(), "themes").is_visible());
            entry.set_text("unfindablequantumsetting");
            assert_eq!(
                stack.visible_child_name().as_deref(),
                Some("settings-search-empty")
            );
            entry.set_text("");
            assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
            assert!(item(layer.upcast_ref(), "previews").is_visible());
            assert!(item(layer.upcast_ref(), "themes").is_visible());
            assert!(item(layer.upcast_ref(), "motion").is_visible());
            assert_eq!(manager.folder_peeking(), original);
        },
    );
}

#[test]
fn late_page_filter_uses_the_current_query_and_recovers_its_sections() {
    crate::test_support::gtk_test(
        "ui::settings::search::tests::late_page_filter_uses_the_current_query_and_recovers_its_sections",
        || {
            crate::ui::prepare_portal_ui();
            for (method, channel_available) in [
                (crate::services::UpdateMethod::InPlace, true),
                (crate::services::UpdateMethod::Pacman, false),
            ] {
                let state = Rc::new(RefCell::new(Some(find_matches("nightly"))));
                let (page, _) = super::super::updates_page(
                    ThemeManager::shared(),
                    Rc::new(|_| {}),
                    super::super::install_guard(),
                    method,
                );
                apply(&page, &state);
                assert_eq!(item(&page, "channel").is_visible(), channel_available);
                assert!(!item(&page, "check").is_visible());
                assert!(!item(&page, "auto-updates").is_visible());
                let empty = descendants(&page)
                    .into_iter()
                    .find(|widget| widget.has_css_class("settings-search-page-empty"))
                    .expect("installation-specific empty state");
                assert_eq!(empty.is_visible(), !channel_available);
                state.replace(None);
                apply(&page, &state);
                assert!(item(&page, "check").is_visible());
                assert!(item(&page, "auto-updates").is_visible());
                assert_eq!(item(&page, "channel").is_visible(), channel_available);
                assert!(!empty.is_visible());
            }
        },
    );
}
