// SPDX-License-Identifier: MIT

use super::super::*;
use crate::test_support::gtk_test;

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

#[test]
fn keybindings_follow_the_active_map_across_windows() {
    gtk_test(
        "ui::settings::tests::reference::keybindings_follow_the_active_map_across_windows",
        || {
            let manager = PreferenceManager::shared();
            manager.set_omastrata_mode(false);
            let first = keybindings_page(manager.clone());
            let second = keybindings_page(manager.clone());
            let (general, _, _) = super::super::general::general_page(manager.clone());
            let phrase = crate::ui::shortcut_reference::EXPERIMENTAL_LABEL;
            assert!(!page_text(general.upcast_ref()).contains(phrase));
            let actions = |page: &gtk::Widget| visible_actions(page);
            assert!(actions(&first).contains(&"Quick preview".to_owned()));
            assert!(actions(&second).contains(&"Rename".to_owned()));
            assert!(keys_for(&first, "Rename").contains("Ctrl"));
            assert!(keys_for(&first, "Rename").contains("R"));
            assert!(page_text(&first).contains("Columns view"));
            assert!(page_text(&first).contains("← / → in Icons view"));
            assert!(!page_text(&first).contains(phrase));

            manager.set_omastrata_mode(true);
            for page in [&first, &second] {
                let shown = actions(page);
                assert!(
                    !shown.contains(&"Quick preview".to_owned()),
                    "the active map drops Space preview"
                );
                assert!(shown.contains(&"Leave Omastrata mode".to_owned()));
                assert!(shown.contains(&"Move between icons".to_owned()));
                assert!(shown.contains(&"Move between column panes".to_owned()));
                let text = page_text(page);
                assert!(text.contains(phrase));
                assert!(keys_for(page, "Shortcut reference").contains('~'));
                assert!(!keys_for(page, "Rename").contains('R'));
            }
            assert!(page_text(general.upcast_ref()).contains(phrase));

            manager.set_omastrata_mode(false);
            assert!(actions(&first).contains(&"Quick preview".to_owned()));
            assert!(actions(&second).contains(&"Quick preview".to_owned()));
            assert!(!page_text(&first).contains(phrase));
            assert!(!page_text(&second).contains(phrase));
            assert!(!page_text(general.upcast_ref()).contains(phrase));
        },
    );
}

fn visible_actions(page: &gtk::Widget) -> Vec<String> {
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

fn keys_for(page: &gtk::Widget, action: &str) -> String {
    let row = descendants::<gtk::Box>(page)
        .into_iter()
        .find(|row| {
            row.has_css_class("keybinding-row")
                && row
                    .first_child()
                    .and_downcast::<gtk::Label>()
                    .is_some_and(|label| label.text() == action)
        })
        .unwrap_or_else(|| panic!("{action} row"));
    descendants::<gtk::Label>(row.upcast_ref())
        .iter()
        .filter(|label| label.has_css_class("settings-keycap"))
        .map(|label| label.text().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn page_text(page: &gtk::Widget) -> String {
    descendants::<gtk::Label>(page)
        .into_iter()
        .map(|label| label.text().to_string())
        .collect::<Vec<_>>()
        .join("\n")
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
