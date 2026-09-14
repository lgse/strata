// SPDX-License-Identifier: MIT

use super::*;
use crate::services::fold_for_search;

#[test]
fn filename_patterns_anchor_stars_and_preserve_literal_substrings() {
    for (name, query, expected) in [
        ("clip.MOV", "*.MOV", true),
        ("IMG_001.MOV", "*.MOV", true),
        ("IMG_001.jpg", "*.MOV", false),
        ("clip.MOV.bak", "*.MOV", false),
        ("IMG_001.jpg", "IMG*", true),
        ("my_IMG_001.jpg", "IMG*", false),
        ("IMG_001.MOV", "IMG*.MOV", true),
        ("clip.MOV", "IMG*.MOV", false),
        ("IMG_001.jpg", "IMG*.MOV", false),
        ("IMG.MOV", "IMG*.MOV", true),
        ("IMG.MOV", "IMG**.MOV", true),
        ("a", "a*a", false),
        ("aa", "a*a", true),
        ("startart", "*start*art", true),
        ("start", "*start*art", false),
        ("one-two-one-end", "*one*end", true),
        ("one-two-end", "*two*one*", false),
        ("IMG_holiday_001.MOV", "IMG*holiday*.MOV", true),
        ("summer holiday.jpg", "*holiday*", true),
        ("clip.MOV", "*missing*", false),
        ("clip.MOV", "*", true),
        ("README", "*", true),
        ("", "*", true),
        ("anything", "***", true),
        ("anything", "", true),
        ("clip.MOV.bak", ".MOV", true),
        ("README", "read", true),
        ("report[1]?.txt", "*[1]?.txt", true),
        ("report1a.txt", "*[1]?.txt", false),
        ("report[1]?.txt", "[1]?", true),
        ("report.txt", "*.(txt|md)", false),
        ("a\\b.txt", "a\\*.txt", true),
        ("photo\nnotes.txt", "photo*.txt", true),
        ("RE\u{301}SUME\u{301}.TXT", "rés*.txt", true),
        ("RÉSUMÉ.txt", "re\u{301}s*.TXT", true),
        ("旅行写真.jpg", "旅*写真.*", true),
        ("旅行写真.jpg", "旅*文書.*", false),
    ] {
        assert_eq!(
            filter_name_matches(&fold_for_search(name), &fold_for_search(query)),
            expected,
            "name={name:?}, query={query:?}",
        );
    }
}
