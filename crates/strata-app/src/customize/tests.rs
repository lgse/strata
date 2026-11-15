// SPDX-License-Identifier: MIT

//! The Customize dialog on the window the crate registers: each choice is saved as it is
//! made, the custom colour's Apply saves it, and Clear forgets both.

use std::path::Path;

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::{FolderColor, FolderColorValue, Location};
use strata_desktop::keys::{Binding, Key as CatalogKey};

use super::customizing;
use crate::{
    App,
    commands::{self, On, Outcome, Situation},
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, fixture, hosted, send_key, view::idle},
};

const MODULE: &str = "customize::tests";

fn center_of(host: &mut Offscreen, key: &str) -> (f32, f32) {
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
    (x + w / 2.0, y + h / 2.0)
}

/// Presses the element keyed `key` once it is mounted.
fn click(host: &mut Offscreen, key: &str) {
    let (x, y) = center_of(host, key);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

/// Presses and releases `key` in one turn, as a tap burst or assistive activation does.
fn tap(host: &mut Offscreen, key: &str) {
    let (x, y) = center_of(host, key);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
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

#[test]
fn customize_on_a_subfolder_filter_hit_opens_that_result() {
    hosted(
        MODULE,
        "customize_on_a_subfolder_filter_hit_opens_that_result",
        |mut host| {
            let folder = fixture();
            host.app().open(Location::local(folder.path()));
            let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, listed), "the folder's listing");
            while host.turn() {}

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                assert_eq!(commands::run(app, "filter", On::Focus, tx), Outcome::Ran);
            });
            idle(&mut host);
            press(&mut host, Key::Text("photo".to_owned()), Mods::default());

            let photo = folder.path().join("pictures/photo.txt");
            let id = shell::id_of(host.app(), &Location::local(&photo));
            let hit = |host: &Offscreen| crate::filter::targets::at(host.app(), 0, id).is_some();
            assert!(host.run_until(PATIENCE, hit), "subfolder filter hit");

            let mut outcome = None;
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                outcome = Some(commands::run(app, "customize", On::Row((0, id)), tx));
            });
            idle(&mut host);
            assert_eq!(outcome, Some(Outcome::Ran));
            assert_eq!(customizing(host.app()), Some(photo.as_path()));

            click(&mut host, "customize-color-blue");
            assert_eq!(
                saved(&host, &photo).0,
                Some(FolderColorValue::Preset(FolderColor::Blue))
            );
        },
    );
}

#[test]
fn apply_commits_typed_hex_without_an_intervening_blur_frame() {
    hosted(
        MODULE,
        "apply_commits_typed_hex_without_an_intervening_blur_frame",
        |mut host| {
            let folder = fixture();
            let target = folder.path().join("pictures");
            let other = folder.path().join("documents");
            let place = Location::local(&target);
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                commands::run(app, "customize", On::Place(place), tx);
            });
            idle(&mut host);

            click(&mut host, "customize-color-custom");
            click(&mut host, "customize-picker-hex");
            let select_all = Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            };
            press(&mut host, Key::Command('a'), select_all);
            press(&mut host, Key::Text("#ff8800".to_owned()), Mods::default());
            assert!(host.app().customize.custom.field_focused());

            tap(&mut host, "customize-custom-1-primary");
            assert_eq!(
                saved(&host, &target).0,
                Some(FolderColorValue::Custom("#ff8800".to_owned()))
            );

            // Re-entering `customize` while the custom-colour dialog is open closes it.
            click(&mut host, "customize-color-custom");
            assert!(host.app().customize.custom.is_open());
            let other_place = Location::local(&other);
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                commands::run(app, "customize", On::Place(other_place), tx);
            });
            idle(&mut host);
            assert!(!host.app().customize.custom.is_open());
            assert_eq!(customizing(host.app()), Some(other.as_path()));
        },
    );
}

#[test]
fn custom_color_reports_typing_only_while_its_hex_field_has_focus() {
    hosted(
        MODULE,
        "custom_color_reports_typing_only_while_its_hex_field_has_focus",
        |mut host| {
            let folder = fixture();
            let place = Location::local(folder.path().join("pictures"));
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                shell::saved(app).change(|settings| settings.tenxer_mode = true);
                commands::run(app, "customize", On::Place(place), tx);
            });
            idle(&mut host);

            click(&mut host, "customize-color-custom");
            let down_alias = |host: &mut Offscreen| {
                let mut matched = false;
                host.runtime().update(|app: &mut App, tx: &mut Tx| {
                    let event = KeyEvent {
                        key: Key::Text("j".to_owned()),
                        mods: Mods::default(),
                    };
                    let press = commands::read(app, &event, &Situation::of(app, tx), tx);
                    matched = press.is(Binding::plain(CatalogKey::Down));
                });
                matched
            };
            assert!(
                down_alias(&mut host),
                "the plane slider takes focus first, so 10xer's structural alias applies"
            );

            click(&mut host, "customize-picker-hex");
            assert!(
                !down_alias(&mut host),
                "while the hex field has focus, plain letters stay text"
            );
        },
    );
}
