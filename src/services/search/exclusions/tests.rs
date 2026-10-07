// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn folder_rules_match_ancestors_but_not_same_named_files() {
    let exclusions = SearchExclusions::from_strings(&["cache".into()]);
    for (path, directory, excluded) in [
        ("/project/cache", true, true),
        ("/other/CACHE", true, true),
        ("/project/cache/nested", true, true),
        ("/project/cache/nested/file.py", false, true),
        ("/project/cache", false, false),
        ("/project/caches", true, false),
        ("/project/file.py", false, false),
    ] {
        let path = Path::new(path);
        assert_eq!(
            exclusions.is_excluded(
                path,
                path.file_name()
                    .expect("basename")
                    .to_str()
                    .expect("UTF-8 fixture"),
                directory
            ),
            excluded,
            "{path:?}"
        );
    }
}

#[test]
fn directory_rules_expand_home_and_respect_component_boundaries_and_case() {
    let home = glib::home_dir();
    let exclusions = SearchExclusions::from_strings(&["~/Secret/./nested//".into()]);
    for (suffix, excluded) in [
        ("Secret/nested", true),
        ("Secret/nested/file.txt", true),
        ("Secret/nested-other/file.txt", false),
        ("Other/Secret/nested", false),
        ("secret/nested/file.txt", false),
    ] {
        let path = home.join(suffix);
        assert_eq!(
            exclusions.is_excluded(
                &path,
                path.file_name()
                    .expect("basename")
                    .to_str()
                    .expect("UTF-8 fixture"),
                true
            ),
            excluded,
            "{suffix}"
        );
    }
}

#[test]
fn equivalent_rules_share_a_cache_key_and_invalid_saved_rules_are_ignored() {
    let home = glib::home_dir();
    let exclusions = SearchExclusions::from_strings(&[
        " Cache/ ".into(),
        "cache".into(),
        "~/Secret//./nested".into(),
        home.join("Secret/nested").to_string_lossy().into_owned(),
        "relative/sub".into(),
        "~someone".into(),
        "/".into(),
        "~".into(),
        "/var/../".into(),
        ".".into(),
        "..".into(),
        "bad\0name".into(),
        home.to_string_lossy().into_owned(),
    ]);
    assert_eq!(
        exclusions,
        SearchExclusions::from_strings(&["cache".into(), "~/Secret/nested".into()])
    );
    assert!(!exclusions.is_excluded(Path::new("/var/other"), "other", true));
}
