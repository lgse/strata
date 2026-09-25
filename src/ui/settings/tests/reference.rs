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
            let shows_default = |page: &gtk::Widget| {
                let shown = visible_actions(page);
                shown.contains(&"Quick preview".to_owned())
                    && !shown.contains(&"Leave Omastrata mode".to_owned())
                    && !page_text(page).contains(phrase)
            };
            assert!(shows_default(&first));
            assert!(shows_default(&second));
            assert!(!page_text(general.upcast_ref()).contains(phrase));

            manager.set_omastrata_mode(true);
            for page in [&first, &second] {
                let shown = visible_actions(page);
                assert!(!shown.contains(&"Quick preview".to_owned()));
                assert!(shown.contains(&"Leave Omastrata mode".to_owned()));
                assert!(page_text(page).contains(phrase));
            }
            assert!(page_text(general.upcast_ref()).contains(phrase));

            manager.set_omastrata_mode(false);
            assert!(shows_default(&first));
            assert!(shows_default(&second));
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

fn page_text(page: &gtk::Widget) -> String {
    descendants::<gtk::Label>(page)
        .into_iter()
        .map(|label| label.text().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
