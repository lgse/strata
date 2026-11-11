use std::collections::BTreeSet;

use super::*;
use crate::theme::color::contrast;
use crate::theme::colors::Key;

fn theme(id: &str) -> &'static Theme {
    bundled_theme(id).unwrap_or_else(|| panic!("{id} is bundled"))
}

/// settings/themes: "Settings → Appearance lists 95 bundled themes in alphabetical order".
#[test]
fn ninety_five_themes_load_with_unique_ids_in_alphabetical_order() {
    let themes = bundled();
    assert_eq!(themes.len(), 95);
    let ids: BTreeSet<&str> = themes.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids.len(), 95, "ids are unique");
    let names: Vec<String> = themes.iter().map(|t| t.name.to_lowercase()).collect();
    assert!(names.windows(2).all(|w| w[0] <= w[1]), "{names:?}");
    assert!(themes.iter().all(|t| t.source == Source::Bundled));
    assert!(bundled_theme(DEFAULT_ID).is_some());
    assert!(bundled_theme(UNKNOWN_ID_FALLBACK).is_some());
}

/// settings/themes: "Code previews color keywords, strings, constants, types, and preprocessor
/// directives from the active theme; all 95 bundled themes define these five colors."
#[test]
fn every_bundled_theme_sets_every_key() {
    for t in bundled() {
        for key in Key::ALL {
            assert!(t.colors.get(key).is_some(), "{} sets {}", t.id, key.name());
        }
    }
}

/// settings/themes: "Every bundled theme's text has at least 4.5:1 contrast on its background
/// and 3:1 on its surface", and "Every bundled theme's dim text, as loaded, has" the same.
#[test]
fn every_bundled_theme_meets_the_text_and_dim_text_floors() {
    for t in bundled() {
        let c = &t.colors;
        for (what, fg) in [("text", c.text), ("dim text", c.dim_text)] {
            assert!(
                contrast(fg, c.background) >= 4.5,
                "{} {what} on background",
                t.id
            );
            assert!(contrast(fg, c.surface) >= 3.0, "{} {what} on surface", t.id);
        }
    }
}

/// settings/themes: "Loading a bundled theme whose catalog dim text misses those floors
/// blends it toward its text color in 1/255 steps, stopping at the first that passes. Tokyo
/// Night's #737aa2 loads as #767da6", and "Material Lighter draws text in #455A64, 6.94:1 on
/// its background, and dim text in #5c7785, lifted from its catalog #607D8B".
#[test]
fn dim_text_lifts_only_as_far_as_the_floors() {
    assert_eq!(theme("tokyo-night").colors.dim_text.hex(), "#767da6");
    let lighter = theme("material-lighter").colors;
    assert_eq!(lighter.text.hex(), "#455a64");
    assert!((contrast(lighter.text, lighter.background) - 6.94).abs() < 0.01);
    assert_eq!(lighter.dim_text.hex(), "#5c7785");
    // Each lifted dim text still reads dimmer than text. (A catalogue entry may write it
    // brighter, as Spacemacs does; that one isn't lifted.)
    let doc = parse_document(CATALOG).expect("parses");
    let raw = doc.get_ref()["themes"]
        .get_ref()
        .as_array()
        .expect("themes");
    let mut lifted = 0;
    for entry in raw {
        let DeValue::Table(table) = entry.get_ref() else {
            panic!("a table")
        };
        let (name, before) = read_table(CATALOG, table).expect("reads");
        let after = bundled()
            .iter()
            .find(|t| t.name == name)
            .expect("loaded")
            .colors;
        if before.dim_text != after.dim_text {
            lifted += 1;
            assert!(
                contrast(after.dim_text, after.background)
                    <= contrast(after.text, after.background),
                "{name}"
            );
        }
    }
    assert!(lifted > 0);
}

/// settings/themes: "Material Vivid draws text in #9e9e9e, 6.01:1 on its background."
#[test]
fn readability_overrides_in_the_catalogue_load_as_written() {
    let vivid = theme("material-vivid").colors;
    assert_eq!(vivid.text.hex(), "#9e9e9e");
    assert!((contrast(vivid.text, vivid.background) - 6.01).abs() < 0.01);
}

#[test]
fn a_catalogue_entry_without_an_id_is_an_error() {
    let bad = "[[themes]]\nname = \"x\"\n";
    assert!(parse(bad).is_err_and(|e| e.contains("line 1")));
}
