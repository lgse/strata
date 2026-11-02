// SPDX-License-Identifier: MIT

use super::*;

fn german_rows_pair_keys_with_actions(labels: &[String], preview: &str) {
    assert!(reference_pairs(labels, "l / → auf einer Datei", preview));
    assert!(has_label(labels, "g g"));
    assert!(has_label(
        labels,
        "Tab in Gehe zu › / Verschieben nach › / Kopieren nach ›"
    ));
    for (keys, action) in [
        (
            "d / Entf",
            "Nach Bestätigung in den Papierkorb verschieben; d d bestätigt",
        ),
        (
            "Strg+C / Strg+X",
            "Ausgewählte Elemente kopieren / ausschneiden",
        ),
        (
            "Strg+Umschalt+1–9 / 0",
            "Tab auswählen (Strg+Umschalt für Zahlen gedrückt halten)",
        ),
    ] {
        assert!(reference_pairs(labels, keys, action), "{keys}: {labels:?}");
    }
}

fn search_matches(footer: &ShortcutFooter, query: &str) -> Vec<String> {
    footer.search.set_text(query);
    settle();
    reference_labels(footer)
}

fn english_key_names_find_german_rows(footer: &ShortcutFooter, preview: &str) {
    let matches = search_matches(footer, "auf einer Datei");
    assert!(reference_pairs(&matches, "l / → auf einer Datei", preview));
    assert!(!has_label(&matches, "g g"));
    let matches = search_matches(footer, "ctrl+c");
    assert!(
        has_label(&matches, "Strg+C / Strg+X"),
        "English key names find their German labels: {matches:?}"
    );
    assert!(!has_label(&matches, "d / Entf"));
    let matches = search_matches(footer, "shift");
    assert!(
        reference_pairs(
            &matches,
            ", a / , m / , s / , e",
            "Nach Name / Änderung / Größe / Typ sortieren; Umschalt kehrt die Reihenfolge um"
        ),
        "English key names find keys named in German action text: {matches:?}"
    );
}

fn typed_symbols_find_alternatives(footer: &ShortcutFooter) {
    for query in ["strg+0", "ctrl+0"] {
        let matches = search_matches(footer, query);
        assert!(
            reference_pairs(
                &matches,
                "Strg++ / − / 0",
                "Textgröße erhöhen / verringern / zurücksetzen"
            ),
            "{query} finds an alternative that shares its modifiers: {matches:?}"
        );
    }
    for (query, keys) in [
        ("ctrl+-", "Strg++ / − / 0"),
        ("g -", "g + / g −"),
        ("1-9", "Strg+Umschalt+1–9 / 0"),
    ] {
        let matches = search_matches(footer, query);
        assert!(
            has_label(&matches, keys),
            "a typed - finds {keys}: {matches:?}"
        );
    }
}

#[test]
fn localized_reference_names_keys_in_the_interface_language() {
    crate::test_support::gtk_test(
        "ui::shortcut_footer::tests::localized::localized_reference_names_keys_in_the_interface_language",
        || {
            let manager = crate::ui::preferences::PreferenceManager::shared();
            // The shared manager applies the saved startup language first.
            rust_i18n::set_locale("de");
            manager.set_tenxer_mode(true);
            let footer = ShortcutFooter::new(BrowserMode::Columns);
            footer.bind_preferences(&manager);
            settle();
            let preview = crate::i18n::tr("Enter its preview; keys move into the drawer");
            german_rows_pair_keys_with_actions(&reference_labels(&footer), &preview);
            english_key_names_find_german_rows(&footer, &preview);
            typed_symbols_find_alternatives(&footer);
        },
    );
}
