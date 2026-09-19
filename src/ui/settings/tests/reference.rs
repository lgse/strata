// SPDX-License-Identifier: MIT

use super::super::*;
use crate::test_support::gtk_test;
use crate::ui::preferences::PreferenceManager;

fn descendants<T: IsA<gtk::Widget> + Clone>(root: &gtk::Widget) -> Vec<T> {
    let mut result = Vec::new();
    if let Ok(widget) = root.clone().downcast::<T>() {
        result.push(widget);
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.extend(descendants::<T>(&widget));
    }
    result
}

#[test]
fn shortcut_search_filters_actions_keys_and_categories_and_recovers_from_no_matches() {
    gtk_test(
        "ui::settings::tests::reference::shortcut_search_filters_actions_keys_and_categories_and_recovers_from_no_matches",
        || {
            let page = keybindings_page(PreferenceManager::shared());
            let search = descendants::<gtk::Entry>(&page).remove(0);
            let rows = descendants::<gtk::Box>(&page)
                .into_iter()
                .filter(|row| row.has_css_class("keybinding-row"))
                .collect::<Vec<_>>();
            let visible_actions = || {
                rows.iter()
                    .filter(|row| row.is_visible())
                    .map(|row| {
                        row.first_child()
                            .and_downcast::<gtk::Label>()
                            .expect("shortcut action label")
                            .text()
                            .to_string()
                    })
                    .collect::<Vec<_>>()
            };
            let all = visible_actions();
            search.set_text("  rEnAmE  ");
            assert_eq!(visible_actions(), ["Rename"]);
            search.set_text("F2");
            assert_eq!(visible_actions(), ["Rename"]);
            search.set_text("Selection");
            assert!(visible_actions().contains(&"Select all".to_owned()));
            assert!(!visible_actions().contains(&"Rename".to_owned()));
            search.set_text("not-a-shortcut");
            assert!(visible_actions().is_empty());
            assert!(descendants::<gtk::Label>(&page).iter().any(
                |label| label.is_visible() && label.text() == "No shortcuts match your search."
            ));
            search.set_text("");
            assert_eq!(visible_actions(), all);
        },
    );
}

fn visible_keybinding_actions(page: &gtk::Widget) -> Vec<String> {
    descendants::<gtk::Box>(page)
        .into_iter()
        .filter(|row| row.has_css_class("keybinding-row") && row.is_visible())
        .map(|row| {
            row.first_child()
                .and_downcast::<gtk::Label>()
                .expect("shortcut action label")
                .text()
                .to_string()
        })
        .collect()
}

fn binding_count(page: &gtk::Widget) -> String {
    descendants::<gtk::Label>(page)
        .into_iter()
        .find(|label| {
            label.has_css_class("settings-control-label") && label.text().ends_with("bindings")
        })
        .expect("binding count")
        .text()
        .to_string()
}

fn unused_in_minimal_note(page: &gtk::Widget, id: &str) -> bool {
    let row = descendants::<gtk::Box>(page)
        .into_iter()
        .find(|widget| widget.widget_name() == format!("settings-search-{id}"))
        .expect("settings option");
    descendants::<gtk::Label>(row.upcast_ref())
        .iter()
        .any(|label| label.is_visible() && label.text() == "Not used in minimal mode.")
}

fn option_switch_sensitive(page: &gtk::Widget, id: &str) -> bool {
    let row = descendants::<gtk::Box>(page)
        .into_iter()
        .find(|widget| widget.widget_name() == format!("settings-search-{id}"))
        .expect("settings option");
    descendants::<gtk::Switch>(row.upcast_ref())
        .first()
        .expect("option switch")
        .is_sensitive()
}

#[test]
fn settings_copy_follows_the_active_keymap() {
    gtk_test(
        "ui::settings::tests::reference::settings_copy_follows_the_active_keymap",
        || {
            let manager = PreferenceManager::shared();
            assert!(!manager.minimal_mode());
            let first_keys = keybindings_page(manager.clone());
            let second_keys = keybindings_page(manager.clone());
            let (first_general, _, _) = general_page(manager.clone());
            let (second_general, _, _) = general_page(manager.clone());
            for page in [&first_keys, &second_keys] {
                let actions = visible_keybinding_actions(page);
                assert!(actions.contains(&"Move through items".to_owned()));
                assert!(actions.contains(&"Duplicate".to_owned()));
                assert!(!actions.contains(&"Parent / leave preview".to_owned()));
                assert_eq!(binding_count(page), format!("{} bindings", actions.len()));
            }
            for page in [&first_general, &second_general] {
                assert!(!unused_in_minimal_note(page, "arrow-scope"));
                assert!(!unused_in_minimal_note(page, "type-search"));
                assert!(option_switch_sensitive(page, "arrow-scope"));
                assert!(option_switch_sensitive(page, "type-search"));
            }
            manager.set_minimal_mode(true);
            for page in [&first_keys, &second_keys] {
                let actions = visible_keybinding_actions(page);
                assert!(actions.contains(&"Parent / leave preview".to_owned()));
                assert!(actions.contains(&"Open directory / enter preview".to_owned()));
                assert!(actions.contains(&"Copy / cut / paste".to_owned()));
                assert!(actions.contains(&"Refresh / location / search".to_owned()));
                assert!(actions.contains(&"Rename / create".to_owned()));
                assert!(!actions.contains(&"Move through items".to_owned()));
                assert!(!actions.contains(&"Duplicate".to_owned()));
                assert_eq!(binding_count(page), format!("{} bindings", actions.len()));
            }
            for page in [&first_general, &second_general] {
                assert!(unused_in_minimal_note(page, "arrow-scope"));
                assert!(unused_in_minimal_note(page, "type-search"));
                assert!(option_switch_sensitive(page, "arrow-scope"));
                assert!(option_switch_sensitive(page, "type-search"));
            }
            let search = descendants::<gtk::Entry>(&first_keys)
                .into_iter()
                .find(|entry| entry.has_css_class("shortcut-search"))
                .expect("shortcut search");
            search.set_text("F5");
            assert_eq!(
                visible_keybinding_actions(&first_keys),
                ["Refresh / location / search"]
            );
            manager.set_minimal_mode(false);
            assert_eq!(visible_keybinding_actions(&first_keys), ["Refresh"]);
            for page in [&first_general, &second_general] {
                assert!(!unused_in_minimal_note(page, "arrow-scope"));
                assert!(!unused_in_minimal_note(page, "type-search"));
            }
        },
    );
}

fn option_labels(page: &gtk::Widget, id: &str) -> Vec<String> {
    let row = descendants::<gtk::Box>(page)
        .into_iter()
        .find(|widget| widget.widget_name() == format!("settings-search-{id}"))
        .expect("settings option");
    descendants::<gtk::Label>(row.upcast_ref())
        .into_iter()
        .map(|label| label.text().to_string())
        .collect()
}

#[test]
fn minimal_mode_labels_include_the_experimental_note() {
    gtk_test(
        "ui::settings::tests::reference::minimal_mode_labels_include_the_experimental_note",
        || {
            let manager = PreferenceManager::shared();
            manager.set_minimal_mode(false);
            let (general, _, _) = general_page(manager.clone());
            let labels = option_labels(&general, "minimal-mode");
            assert!(
                labels.iter().any(|text| text == "Minimal mode"),
                "title stays Minimal mode, got {labels:?}"
            );
            assert_eq!(
                crate::ui::minimal_mode::LABELED_TITLE,
                format!(
                    "Minimal mode {}",
                    crate::ui::minimal_mode::EXPERIMENTAL_NOTE
                )
            );
            assert!(
                labels
                    .iter()
                    .any(|text| text == crate::ui::minimal_mode::EXPERIMENTAL_NOTE),
                "experimental note sits next to the title, got {labels:?}"
            );
            assert!(
                labels
                    .iter()
                    .any(|text| text.contains("Hide pane chrome and use Yazi-style keys")),
                "description remains visible, got {labels:?}"
            );

            manager.set_minimal_mode(true);
            let keys = keybindings_page(manager);
            let categories: Vec<String> = descendants::<gtk::Label>(&keys)
                .into_iter()
                .filter(|label| label.has_css_class("shortcut-category") && label.is_visible())
                .map(|label| label.text().to_string())
                .collect();
            assert!(
                categories
                    .iter()
                    .any(|text| text == crate::ui::minimal_mode::LABELED_TITLE),
                "keybindings heading includes the experimental note, got {categories:?}"
            );
        },
    );
}

#[test]
fn about_copies_the_running_build_details_to_the_clipboard() {
    gtk_test(
        "ui::settings::tests::reference::about_copies_the_running_build_details_to_the_clipboard",
        || {
            let page = about_page();
            let button = descendants::<gtk::Button>(&page)
                .into_iter()
                .find(|button| button.label().as_deref() == Some("Copy version info"))
                .expect("copy version info action");
            button.emit_clicked();
            let text = glib::MainContext::default()
                .block_on(button.clipboard().read_text_future())
                .expect("clipboard read")
                .expect("version text");
            assert!(text.starts_with(&format!(
                "Strata {}\n",
                crate::build_info::installed_version()
            )));
            assert!(text.contains(&format!("Commit: {}", crate::build_info::COMMIT)));
            assert!(text.contains(&format!(
                "Toolkit: GTK {}.{}.",
                gtk::major_version(),
                gtk::minor_version()
            )));
            assert!(text.ends_with("License: MIT"));
        },
    );
}

#[test]
fn managed_channel_cannot_be_changed_from_the_settings_menu() {
    gtk_test(
        "ui::settings::tests::reference::managed_channel_cannot_be_changed_from_the_settings_menu",
        || {
            let source = super::packaged();
            let manager = PreferenceManager::shared();
            let row = channel_option(manager, source.managed());
            let menu = descendants::<gtk::MenuButton>(row.upcast_ref()).remove(0);
            assert!(!menu.is_sensitive());
            assert!(
                descendants::<gtk::Label>(row.upcast_ref())
                    .iter()
                    .any(|label| label.text().contains("strata-rc-bin"))
            );
        },
    );
}
