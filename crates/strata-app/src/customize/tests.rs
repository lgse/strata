// SPDX-License-Identifier: MIT

//! The Customize dialog on the window the crate registers: each choice is saved as it is
//! made, the custom colour's Apply saves it, and Clear forgets both.

use std::path::Path;

use rune::Tx;
use strata_core::model::{FolderColor, FolderColorValue, Location};

use crate::{
    App,
    commands::{self, On},
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, fixture, hosted, view::idle},
};

const MODULE: &str = "customize::tests";

/// Presses the element keyed `key` once it is mounted.
fn click(host: &mut Offscreen, key: &str) {
    let mounted = |host: &Offscreen| {
        host.elements()
            .iter()
            .any(|element| !element.exiting && element.key.as_deref() == Some(key))
    };
    assert!(host.run_until(PATIENCE, mounted), "{key} is mounted");
    let elements = host.runtime().elements();
    let element = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key))
        .expect("the element");
    let [x, y, w, h] = element.visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn saved(host: &Offscreen, path: &Path) -> (Option<FolderColorValue>, Option<String>) {
    let preferences = shell::preferences(host.app());
    (
        preferences.item_color(path),
        preferences.item_icon(path).map(str::to_owned),
    )
}

#[test]
fn each_choice_is_saved_at_once_and_clear_forgets_them() {
    hosted(
        MODULE,
        "each_choice_is_saved_at_once_and_clear_forgets_them",
        |mut host| {
            let folder = fixture();
            let target = folder.path().join("pictures");
            let place = Location::local(&target);
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                commands::run(app, "customize", On::Place(place), tx);
            });
            idle(&mut host);
            assert!(
                host.app().customize.open,
                "the sidebar row's command opens the dialog"
            );

            click(&mut host, "customize-color-red");
            click(&mut host, "customize-icon-2");
            let red = FolderColorValue::Preset(FolderColor::Red);
            assert_eq!(
                saved(&host, &target),
                (Some(red.clone()), Some("strata-file-code".to_owned()))
            );

            // The custom colour opens on the item's, and Apply saves it as a custom one.
            click(&mut host, "customize-color-custom");
            click(&mut host, "customize-custom-1-primary");
            assert_eq!(
                saved(&host, &target).0,
                Some(FolderColorValue::Custom(red.hex().to_owned()))
            );

            click(&mut host, "customize-1-secondary");
            assert_eq!(saved(&host, &target), (None, None));
        },
    );
}
