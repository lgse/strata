// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{blur::BlurBin, preferences::PreferenceManager};

#[test]
fn localized_setting_titles_keep_stable_search_targets() {
    crate::test_support::gtk_test(
        "ui::settings::search::tests::localized_setting_titles_keep_stable_search_targets",
        || {
            for (locale, query) in [
                ("fr", "Langue"),
                ("de", "Sprache"),
                ("ja", "言語"),
                ("ko", "언어"),
                ("ru", "Язык"),
            ] {
                rust_i18n::set_locale(locale);
                for query in [query, "Language"] {
                    let matches = find_matches(&normalized(query));
                    assert_eq!(matches.best_page, Some("general"), "{locale}: {query}");
                    assert!(matches.ids.contains("language"));
                }
            }
            rust_i18n::set_locale("de");
            for (query, id) in [
                ("10xer", "tenxer"),
                ("Modus", "tenxer"),
                ("Doppelklick", "opening"),
                ("double click", "opening"),
            ] {
                assert!(
                    find_matches(&normalized(query)).ids.contains(id),
                    "de: {query}"
                );
            }
            rust_i18n::set_locale("ja");
            assert!(
                find_matches(&normalized("ダブルクリック"))
                    .ids
                    .contains("opening")
            );
        },
    );
}

#[test]
fn ranks_exact_labels_aliases_and_small_typing_errors() {
    for (query, page, id) in [
        ("Folder peeking", "general", "peeking"),
        ("Autoplay media previews", "general", "preview-autoplay"),
        ("paused", "general", "preview-autoplay"),
        ("Items shown in sidebar", "general", "sidebar-places"),
        ("sidebar downloads", "general", "sidebar-places"),
        ("show network", "general", "sidebar-places"),
        ("show recent", "general", "sidebar-places"),
        ("tezt size", "theme", "text"),
        ("font size", "theme", "text"),
        ("nightly", "updates", "channel"),
        ("relase chanel", "updates", "channel"),
        ("copyright", "about", "license"),
        ("shortcuts button", "general", "hints"),
        ("keybindings", "general", "hints"),
        ("render documents", "general", "render-documents"),
        ("markdown", "general", "render-documents"),
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
        .find(|widget| tagged(widget, id))
        .expect("searchable setting")
}

fn tagged(widget: &gtk::Widget, id: &str) -> bool {
    widget.widget_name() == format!("settings-search-{id}")
}

fn settings_layer(manager: &Rc<PreferenceManager>) -> (gtk::Widget, gtk::Entry, gtk::Stack) {
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
    let layer: gtk::Widget = layer.upcast();
    let entry = descendants(&layer)
        .into_iter()
        .find(|widget| widget.has_css_class("settings-global-search-entry"))
        .and_then(|widget| widget.downcast::<gtk::Entry>().ok())
        .expect("global search");
    let stack = descendants(&layer)
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::Stack>().ok())
        .expect("settings pages");
    (layer, entry, stack)
}

#[test]
fn global_search_navigates_filters_lazy_pages_and_restores_without_editing_preferences() {
    crate::test_support::gtk_test(
        "ui::settings::search::tests::global_search_navigates_filters_lazy_pages_and_restores_without_editing_preferences",
        || {
            crate::ui::prepare_portal_ui();
            let manager = PreferenceManager::shared();
            let original = manager.folder_peeking();
            let (layer, entry, stack) = settings_layer(&manager);
            let layer = &layer;
            assert!(stack.child_by_name("theme").is_none());
            entry.set_text("folder peeking");
            assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
            assert!(item(layer, "peeking").is_visible());
            assert!(!item(layer, "previews").is_visible());
            assert!(!item(layer, "render-documents").is_visible());
            entry.set_text("autoplay");
            assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
            assert!(item(layer, "preview-autoplay").is_visible());
            assert!(!item(layer, "peeking").is_visible());
            entry.set_text("window buttons");
            assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
            for id in ["window-minimize", "window-maximize", "window-close"] {
                assert!(item(layer, id).is_visible());
            }
            assert!(!item(layer, "previews").is_visible());
            entry.set_text("restore");
            assert!(item(layer, "window-maximize").is_visible());
            assert!(!item(layer, "window-minimize").is_visible());
            assert!(!item(layer, "window-close").is_visible());
            entry.set_text("tezt size");
            assert_eq!(stack.visible_child_name().as_deref(), Some("theme"));
            assert!(item(layer, "text").is_visible());
            assert!(!item(layer, "motion").is_visible());
            assert!(!item(layer, "themes").is_visible());
            entry.set_text("unfindablequantumsetting");
            assert_eq!(
                stack.visible_child_name().as_deref(),
                Some("settings-search-empty")
            );
            entry.set_text("");
            assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
            assert!(item(layer, "previews").is_visible());
            assert!(item(layer, "themes").is_visible());
            assert!(item(layer, "motion").is_visible());
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

fn nav(layer: &gtk::Widget, page: &str) -> gtk::Button {
    descendants(layer)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.widget_name() == page)
        .unwrap_or_else(|| panic!("navigation button for {page}"))
}

/// Whether `widget` and every ancestor up to its stack page are visible.
fn shown_on_page(widget: &gtk::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget
            .parent()
            .is_some_and(|parent| parent.is::<gtk::Stack>())
        {
            return true;
        }
        if !widget.is_visible() {
            return false;
        }
        current = widget.parent();
    }
    false
}

#[test]
fn global_search_routes_every_target_to_its_page() {
    crate::test_support::gtk_test(
        "ui::settings::search::tests::global_search_routes_every_target_to_its_page",
        || {
            crate::ui::prepare_portal_ui();
            let (layer, entry, stack) = settings_layer(&PreferenceManager::shared());
            let title = descendants(&layer)
                .into_iter()
                .find(|widget| widget.has_css_class("settings-title"))
                .and_then(|widget| widget.downcast::<gtk::Label>().ok())
                .expect("settings title");
            let mut failures = Vec::new();
            for target in TARGETS {
                entry.set_text(target.title);
                let mut fail = |problem: String| {
                    failures.push(format!("{:?} ({}): {problem}", target.title, target.id));
                };
                let shown = stack.visible_child_name();
                if shown.as_deref() != Some(target.page) {
                    fail(format!("opened page {shown:?}, expected {}", target.page));
                }
                let expected_title = page_title(target.page).map(crate::i18n::tr);
                if expected_title.as_deref() != Some(title.text().as_str()) {
                    fail(format!(
                        "title {:?}, expected {expected_title:?}",
                        title.text()
                    ));
                }
                let matches = find_matches(&normalized(target.title));
                for page in super::super::NAVIGATION.iter().map(|entry| entry.page) {
                    let visible = nav(&layer, page).is_visible();
                    if visible != matches.pages.contains(page) {
                        fail(format!("{page} navigation button visible={visible}"));
                    }
                }
                // Updates builds asynchronously; every_page_row_is_registered_for_search
                // builds it directly.
                if target.page == "updates" {
                    continue;
                }
                match descendants(&layer)
                    .into_iter()
                    .find(|widget| tagged(widget, target.id))
                {
                    None => fail("no tagged row in the built pages".into()),
                    Some(row)
                        if !shown_on_page(&row)
                            && !row.has_css_class("settings-search-unavailable") =>
                    {
                        fail("matching row is hidden".into());
                    }
                    Some(_) => {}
                }
            }
            assert!(
                failures.is_empty(),
                "{} routing failures:\n{}",
                failures.len(),
                failures.join("\n")
            );
        },
    );
}
