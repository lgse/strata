// SPDX-License-Identifier: MIT

use super::super::*;
use crate::test_support::gtk_test;

fn page_text(root: &gtk::Widget) -> String {
    let mut text = Vec::new();
    if let Ok(label) = root.clone().downcast::<gtk::Label>() {
        text.push(label.text().to_string());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        text.push(page_text(&widget));
    }
    text.join("\n")
}

#[test]
fn tenxer_experimental_label_follows_the_mode_across_windows() {
    gtk_test(
        "ui::settings::tests::general::tenxer_experimental_label_follows_the_mode_across_windows",
        || {
            let manager = PreferenceManager::shared();
            manager.set_tenxer_mode(false);
            let (first, _, _) = general_page(manager.clone());
            let (second, _, _) = general_page(manager.clone());
            let phrase = crate::ui::shortcut_reference::EXPERIMENTAL_LABEL;
            for page in [&first, &second] {
                assert!(!page_text(page).contains(phrase));
            }

            manager.set_tenxer_mode(true);
            for page in [&first, &second] {
                assert!(page_text(page).contains(phrase));
            }

            manager.set_tenxer_mode(false);
            for page in [&first, &second] {
                assert!(!page_text(page).contains(phrase));
            }
        },
    );
}
