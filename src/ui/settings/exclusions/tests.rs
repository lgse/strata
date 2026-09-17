// SPDX-License-Identifier: MIT

use std::path::Path;

use super::*;
use crate::test_support::gtk_test;

#[test]
fn abbreviate_home_shortens_user_home_path() {
    let home = glib::home_dir();
    let inside = home.join("Downloads/stuff");
    assert_eq!(
        crate::ui::settings::general::abbreviate_home(&inside),
        "~/Downloads/stuff"
    );

    let outside = Path::new("/var/log");
    assert_eq!(
        crate::ui::settings::general::abbreviate_home(outside),
        "/var/log"
    );
}

#[test]
fn search_exclusions_persist_and_update_behavior() {
    gtk_test(
        "ui::settings::exclusions::tests::search_exclusions_persist_and_update_behavior",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            manager.set_search_exclusions(vec![".venv".to_owned(), "~/Secret".to_owned()]);
            assert_eq!(
                manager.search_exclusions(),
                vec![".venv".to_owned(), "~/Secret".to_owned()]
            );

            // Verify case-insensitive removal matching exclusions dialog behavior
            let mut current = manager.search_exclusions();
            current.retain(|item| !item.eq_ignore_ascii_case(".VENV"));
            manager.set_search_exclusions(current);
            assert_eq!(manager.search_exclusions(), vec!["~/Secret".to_owned()]);
        },
    );
}

#[test]
fn validate_exclusion_input_handles_all_cases() {
    let current = vec![".venv".to_owned(), "/var/log".to_owned()];

    // Empty input returns Err(None) to clear errors
    assert_eq!(validate_exclusion_input("   ", &current), Err(None));

    // Bare ~ or / is rejected
    assert_eq!(
        validate_exclusion_input("~", &current),
        Err(Some("Cannot exclude root or entire home directory."))
    );
    assert_eq!(
        validate_exclusion_input("/", &current),
        Err(Some("Cannot exclude root or entire home directory."))
    );

    // Relative path with slash or ~user is rejected
    assert_eq!(
        validate_exclusion_input("project/build", &current),
        Err(Some("Directory paths must start with / or ~/"))
    );
    assert_eq!(
        validate_exclusion_input("~user/dir", &current),
        Err(Some("Directory paths must start with / or ~/"))
    );

    // Folder names are rejected case-insensitively
    assert_eq!(
        validate_exclusion_input(".VENV", &current),
        Err(Some("This exclusion has already been added."))
    );

    // Directory paths match case-sensitively on Linux
    assert_eq!(
        validate_exclusion_input("/Var/Log", &current),
        Ok("/Var/Log".to_owned())
    );
    assert_eq!(
        validate_exclusion_input("/var/log", &current),
        Err(Some("This exclusion has already been added."))
    );

    // Valid inputs
    assert_eq!(
        validate_exclusion_input("node_modules/", &current),
        Ok("node_modules".to_owned())
    );
    assert_eq!(
        validate_exclusion_input("~/Downloads/", &current),
        Ok("~/Downloads".to_owned())
    );
}
