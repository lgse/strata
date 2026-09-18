// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{blur::BlurBin, preferences::PreferenceManager};

#[test]
fn ranks_exact_labels_aliases_and_small_typing_errors() {
    for (query, page, id) in [
        ("Folder peeking", "general", "peeking"),
        ("Items shown in sidebar", "general", "sidebar-places"),
        ("sidebar downloads", "general", "sidebar-places"),
        ("show network", "general", "sidebar-places"),
        ("show recent", "general", "sidebar-places"),
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

#[test]
fn luks_and_udiskie_queries_hit_unlock_target() {
    let luks = find_matches(&normalized("luks"));
    assert!(
        luks.ids.contains("udiskie-unlock"),
        "luks should match Unlock encrypted volumes, got {:?}",
        luks.ids
    );
    assert!(
        !luks.ids.contains("desktop"),
        "luks should not match System file manager, got {:?}",
        luks.ids
    );
    let udiskie = find_matches(&normalized("udiskie"));
    assert!(
        udiskie.ids.contains("udiskie-unlock"),
        "udiskie should match Unlock encrypted volumes, got {:?}",
        udiskie.ids
    );
    assert!(
        udiskie.ids.contains("desktop"),
        "udiskie should still surface Desktop integration, got {:?}",
        udiskie.ids
    );
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
            let manager = PreferenceManager::shared();
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
                    PreferenceManager::shared(),
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

#[test]
fn luks_keeps_desktop_heading_and_hides_portal_row() {
    crate::test_support::gtk_test(
        "ui::settings::search::tests::luks_keeps_desktop_heading_and_hides_portal_row",
        || {
            let preferences = gtk::Box::new(gtk::Orientation::Vertical, 0);
            preferences.add_css_class("settings-preferences");
            let heading = gtk::Label::new(Some("DESKTOP INTEGRATION"));
            heading.add_css_class("menu-heading");
            let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
            group.add_css_class("settings-group");
            let portal = gtk::Box::new(gtk::Orientation::Vertical, 0);
            tag(&portal, "Desktop integration");
            let udiskie = gtk::Box::new(gtk::Orientation::Vertical, 0);
            tag(&udiskie, "Unlock encrypted volumes");
            set_available(&udiskie, true);
            group.append(&portal);
            group.append(&udiskie);
            preferences.append(&heading);
            preferences.append(&group);
            let later = gtk::Label::new(Some("STARTUP"));
            later.add_css_class("menu-heading");
            let directory = gtk::Box::new(gtk::Orientation::Vertical, 0);
            tag(&directory, "Default directory");
            preferences.append(&later);
            preferences.append(&directory);
            let state = Rc::new(RefCell::new(Some(find_matches(&normalized("luks")))));
            apply(&preferences, &state);
            assert!(
                udiskie.is_visible(),
                "luks should show Unlock encrypted volumes"
            );
            assert!(!portal.is_visible(), "luks should hide System file manager");
            assert!(
                group.is_visible(),
                "luks should keep the Desktop integration group"
            );
            assert!(
                heading.is_visible(),
                "luks should keep the DESKTOP INTEGRATION heading"
            );
            assert!(
                !later.is_visible(),
                "luks should not keep a later section heading"
            );
            assert!(
                !directory.is_visible(),
                "luks should hide Default directory"
            );
        },
    );
}
