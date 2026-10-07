// SPDX-License-Identifier: MIT

use super::match_ranges;

#[test]
fn find_matches_case_insensitive_substrings_by_byte_range() {
    for (name, query, expected) in [
        ("report-final.txt", "rep", vec![(0, 3)]),
        ("README.md", "read", vec![(0, 4)]),
        ("notes.txt", "T", vec![(2, 3), (6, 7), (8, 9)]),
        ("aaaa", "aa", vec![(0, 2), (2, 4)]),
        ("Café Menu", "é m", vec![(3, 7)]),
        ("ÅNGSTRÖM", "ström", vec![(4, 10)]),
        ("plain", "", vec![]),
        ("plain", "xyz", vec![]),
        ("ab", "abc", vec![]),
    ] {
        let ranges = match_ranges(name, query);
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(start, end)| start..end)
            .collect();
        assert_eq!(ranges, expected, "{query:?} in {name:?}");
        for range in ranges {
            assert!(name.is_char_boundary(range.start) && name.is_char_boundary(range.end));
        }
    }
}

#[test]
fn find_highlights_follow_the_applied_theme_colors() {
    crate::test_support::gtk_test(
        "ui::browser::find::tests::find_highlights_follow_the_applied_theme_colors",
        || {
            use gtk::prelude::*;

            fn segment_colors(label: &gtk::Label) -> Vec<(u32, u32, String)> {
                label
                    .attributes()
                    .map(|attributes| attributes.attributes())
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|attribute| {
                        let color = attribute.downcast_ref::<gtk::pango::AttrColor>()?;
                        Some((
                            attribute.start_index(),
                            attribute.end_index(),
                            color.color().to_str().to_string(),
                        ))
                    })
                    .collect()
            }

            let label = gtk::Label::new(Some("todo.txt"));
            super::apply_theme("#112233", "#f0f0f0");
            super::highlight_name(label.upcast_ref(), Some("do"));
            let mut colors = segment_colors(&label);
            colors.sort();
            assert_eq!(
                colors,
                [
                    (2, 4, "#111122223333".to_owned()),
                    (2, 4, "#f0f0f0f0f0f0".to_owned()),
                ]
            );

            super::apply_theme("#aa0000", "#000000");
            super::highlight_name(label.upcast_ref(), Some("do"));
            assert!(
                segment_colors(&label)
                    .iter()
                    .any(|(_, _, color)| color == "#aaaa00000000"),
                "a new theme recolors the highlight"
            );
            super::highlight_name(label.upcast_ref(), None);
            assert!(label.attributes().is_none());
        },
    );
}

#[test]
fn listing_filters_use_and_highlight_fuzzy_terms_only_in_tenxer_mode() {
    crate::test_support::gtk_test(
        "ui::browser::find::tests::listing_filters_use_and_highlight_fuzzy_terms_only_in_tenxer_mode",
        || {
            use crate::{
                services::fold_for_search, ui::browser::entry::entry_matches,
                ui::preferences::PreferenceManager,
            };
            use gtk::prelude::*;

            let matches =
                |query: &str| entry_matches("fv\tgamma-report.md", false, &fold_for_search(query));
            let highlighted = |query: &str| {
                let label = gtk::Label::new(Some("gamma-report.md"));
                super::highlight_listing_name(label.upcast_ref(), None, &fold_for_search(query));
                label.attributes().is_some()
            };
            let preferences = PreferenceManager::shared();

            preferences.set_tenxer_mode(false);
            assert!(matches("*.md"));
            assert!(!matches("rep md"));
            assert!(!highlighted("report"), "ordinary filters do not highlight");

            preferences.set_tenxer_mode(true);
            for (query, expected) in [
                ("rep md", true),
                ("MD Rep", true),
                ("^gamma .md$", true),
                ("rep !gamma", false),
                ("*.md", false),
                ("reports", false),
            ] {
                assert_eq!(matches(query), expected, "{query:?}");
            }
            assert!(highlighted("rep md"));
            assert!(!highlighted(""));
        },
    );
}
