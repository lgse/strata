// SPDX-License-Identifier: MIT

use std::{path::PathBuf, rc::Rc};

use rune::{Key, Mods};
use serde_json::Value;
use strata_core::i18n::tr;

use crate::{
    error_dialog, launch,
    settings_dialog::tests::{click, has_state, press},
    shell,
    test_support::{PATIENCE, hosted, view},
};

const MODULE: &str = "settings_dialog::general_more::tests";
const CHOOSE_KEY: &str = "settings-default-dir-choose";
const RESET_KEY: &str = "settings-default-dir-reset";

fn find_by_key<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    view::all(tree, &|node| node["key"] == key)
        .into_iter()
        .next()
}

#[test]
fn default_directory_chooser_picks_abbreviates_cancels_errors_and_resets() {
    hosted(
        MODULE,
        "default_directory_chooser_picks_abbreviates_cancels_errors_and_resets",
        |mut host| {
            click(&mut host, "header-settings");

            let tree = host.tree();
            let choose = find_by_key(&tree, CHOOSE_KEY).expect("choose button");
            assert_eq!(view::text(choose, "name"), tr("Home directory"));
            assert_eq!(
                view::text(choose, "description"),
                tr("Select default directory")
            );
            let reset = find_by_key(&tree, RESET_KEY).expect("reset button");
            assert!(has_state(reset, "disabled"));

            // Picking a folder under $HOME saves it and shows ~/ abbreviation.
            let target = launch::home_directory().join("Projects");
            let target_clone = target.clone();
            host.app_mut().settings_dialog.folder_picker =
                Some(Rc::new(move |_| Ok(Some(target_clone.clone()))));
            click(&mut host, CHOOSE_KEY);
            assert!(
                host.run_until(PATIENCE, |h| {
                    shell::preferences(h.app()).default_directory.as_ref() == Some(&target)
                }),
                "picked path saved to preferences"
            );
            let tree = host.tree();
            let choose = find_by_key(&tree, CHOOSE_KEY).expect("choose button");
            assert_eq!(view::text(choose, "name"), "~/Projects");
            let reset = find_by_key(&tree, RESET_KEY).expect("reset button");
            assert!(!has_state(reset, "disabled"));

            // Cancelling leaves the saved directory unchanged.
            host.app_mut().settings_dialog.folder_picker = Some(Rc::new(|_| Ok(None)));
            click(&mut host, CHOOSE_KEY);
            host.runtime().frame(0.05);
            assert_eq!(
                shell::preferences(host.app()).default_directory.as_ref(),
                Some(&target)
            );

            // Picker error surfaces error_dialog without clearing the saved directory.
            host.app_mut().settings_dialog.folder_picker =
                Some(Rc::new(|_| Err("portal unavailable".to_owned())));
            click(&mut host, CHOOSE_KEY);
            assert!(
                host.run_until(PATIENCE, |h| error_dialog::shown_details(h.app()).is_some()),
                "error dialog opens on picker failure"
            );
            let (_, _, title, _, message) =
                error_dialog::shown_details(host.app()).expect("error details");
            assert_eq!(title, tr("Default directory"));
            assert_eq!(message, "portal unavailable");
            assert_eq!(
                shell::preferences(host.app()).default_directory.as_ref(),
                Some(&target)
            );
            press(&mut host, Key::Escape, Mods::default());
            assert!(error_dialog::shown_details(host.app()).is_none());

            // Picking a non-home path shows full path, and Reset clears back to Home directory.
            let external = PathBuf::from("/tmp/strata-default-dir");
            let external_clone = external.clone();
            host.app_mut().settings_dialog.folder_picker =
                Some(Rc::new(move |_| Ok(Some(external_clone.clone()))));
            click(&mut host, CHOOSE_KEY);
            assert!(host.run_until(PATIENCE, |h| {
                shell::preferences(h.app()).default_directory.as_ref() == Some(&external)
            }));
            assert_eq!(
                view::text(
                    find_by_key(&host.tree(), CHOOSE_KEY).expect("choose"),
                    "name"
                ),
                "/tmp/strata-default-dir"
            );

            click(&mut host, RESET_KEY);
            assert!(shell::preferences(host.app()).default_directory.is_none());
            let tree = host.tree();
            assert_eq!(
                view::text(find_by_key(&tree, CHOOSE_KEY).expect("choose"), "name"),
                tr("Home directory")
            );
            assert!(has_state(
                find_by_key(&tree, RESET_KEY).expect("reset"),
                "disabled"
            ));
        },
    );
}
