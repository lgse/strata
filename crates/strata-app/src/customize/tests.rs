// SPDX-License-Identifier: MIT

//! The Customize dialog on the window the crate registers: each choice is saved as it is
//! made, the custom colour's Apply saves it, and Clear forgets both.

use std::path::Path;

use rune::{Color, Key, KeyEvent, Mods, Tx};
use strata_components::{
    contract::{EntryId, EntrySource, OwnedGlyph},
    controls::icon_picker::{Choice, IconPickerState, Page, drawn_text},
    sidebar::RowIcon,
};
use strata_core::model::{FolderColor, FolderColorValue, Location};
use strata_desktop::keys::{Binding, Key as CatalogKey};

use super::{customizing, emoji};
use crate::{
    App,
    commands::{self, On, Outcome, Situation},
    host::offscreen::Offscreen,
    shell,
    slots::color_of,
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

fn label_of(host: &mut Offscreen, key: &str) -> Option<String> {
    host.runtime()
        .elements()
        .into_iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key))
        .and_then(|element| element.label)
}

#[test]
fn emoji_picker_saves_choice_tracks_recent_and_resets_with_icons_and_clear() {
    hosted(
        MODULE,
        "emoji_picker_saves_choice_tracks_recent_and_resets_with_icons_and_clear",
        |mut host| {
            let folder = fixture();
            let target = folder.path().join("pictures");
            let place = Location::local(&target);
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                commands::run(app, "customize", On::Place(place), tx);
            });
            idle(&mut host);
            assert_eq!(
                label_of(&mut host, "customize-emoji-button").as_deref(),
                Some("Choose Emoji…")
            );

            click(&mut host, "customize-emoji-button");
            assert!(host.app().customize.emoji.is_open());
            press(
                &mut host,
                Key::Text("astronaut".to_owned()),
                Mods::default(),
            );
            press(&mut host, Key::Enter, Mods::default());

            let astronaut = Choice::emoji("🧑\u{200d}🚀");
            let astronaut_label = format!("Emoji  {}", drawn_text(&astronaut));
            assert!(!host.app().customize.emoji.is_open());
            assert_eq!(
                saved(&host, &target).1.as_deref(),
                Some("emoji:🧑\u{200d}🚀")
            );
            assert_eq!(
                label_of(&mut host, "customize-emoji-button").as_deref(),
                Some(astronaut_label.as_str())
            );

            // Reopening starts on Recently used and keeps the saved emoji marked current.
            click(&mut host, "customize-emoji-button");
            assert_eq!(host.app().customize.emoji.picker_page(), Page::Recent);
            assert_eq!(host.app().customize.emoji.current(), Some(&astronaut));
            press(&mut host, Key::Escape, Mods::default());

            // Choosing a bundled icon or pressing Clear resets the emoji button and current mark.
            click(&mut host, "customize-icon-2");
            assert_eq!(saved(&host, &target).1.as_deref(), Some("strata-file-code"));
            assert_eq!(host.app().customize.emoji.current(), None);
            assert_eq!(
                label_of(&mut host, "customize-emoji-button").as_deref(),
                Some("Choose Emoji…")
            );

            click(&mut host, "customize-emoji-button");
            press(&mut host, Key::Enter, Mods::default());
            assert_eq!(
                saved(&host, &target).1.as_deref(),
                Some("emoji:🧑\u{200d}🚀")
            );
            click(&mut host, "customize-1-secondary");
            assert_eq!(saved(&host, &target), (None, None));
            assert_eq!(host.app().customize.emoji.current(), None);
        },
    );
}

#[test]
fn emoji_picker_escape_closes_only_popover_from_field_and_grid() {
    hosted(
        MODULE,
        "emoji_picker_escape_closes_only_popover_from_field_and_grid",
        |mut host| {
            let folder = fixture();
            let place = Location::local(folder.path().join("pictures"));
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                commands::run(app, "customize", On::Place(place), tx);
            });
            idle(&mut host);

            click(&mut host, "customize-emoji-button");
            assert!(host.app().customize.emoji.field_focused());
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().customize.emoji.is_open());
            assert!(
                host.app().customize.open,
                "first Escape from search closes only the emoji popover"
            );

            click(&mut host, "customize-emoji-button");
            press(&mut host, Key::Down, Mods::default());
            assert!(!host.app().customize.emoji.field_focused());
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().customize.emoji.is_open());
            assert!(
                host.app().customize.open,
                "first Escape from the emoji grid closes only the popover"
            );

            press(&mut host, Key::Escape, Mods::default());
            assert!(
                !host.app().customize.open,
                "second Escape closes the Customize dialog"
            );
        },
    );
}

#[test]
fn emoji_picker_reports_typing_only_while_search_field_has_focus() {
    hosted(
        MODULE,
        "emoji_picker_reports_typing_only_while_search_field_has_focus",
        |mut host| {
            let folder = fixture();
            let place = Location::local(folder.path().join("pictures"));
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                shell::saved(app).change(|settings| settings.tenxer_mode = true);
                commands::run(app, "customize", On::Place(place), tx);
            });
            idle(&mut host);

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

            click(&mut host, "customize-emoji-button");
            assert!(
                !down_alias(&mut host),
                "the search field takes focus first, so plain letters type into search"
            );

            press(&mut host, Key::Down, Mods::default());
            assert!(
                down_alias(&mut host),
                "once Down moves focus to the emoji grid, 10xer's structural alias applies"
            );
        },
    );
}

#[test]
fn emoji_picker_translates_chrome_and_categories_while_keeping_english_search() {
    for locale in rust_i18n::available_locales!() {
        let words = emoji::words_for(&locale);
        assert!(!words.grid.is_empty(), "{locale}: grid");
        assert!(!words.popover.is_empty(), "{locale}: popover");
        assert!(!words.tabs.is_empty(), "{locale}: tabs");
        assert!(!words.recent.is_empty(), "{locale}: recent");
        assert!(!words.current.is_empty(), "{locale}: current");
        let source = emoji::source_for(&locale);
        assert_eq!(source.categories().len(), 9, "{locale}: categories");
    }

    let french_words = emoji::words_for("fr");
    assert_eq!(french_words.popover, "Choisir un émoji");
    assert_eq!(french_words.recent, "Utilisés récemment");
    let french_source = emoji::source_for("fr");
    assert_eq!(french_source.categories()[0].name, "Smileys et émotions");

    let mut picker = IconPickerState::new(french_source);
    picker.set_query("smile");
    assert!(
        picker.results().is_some_and(|count| count > 0),
        "English CLDR keywords still match under a non-English UI locale"
    );
}

fn entry_worn(source: &dyn EntrySource, id: EntryId) -> (Option<Color>, Option<OwnedGlyph>, bool) {
    let index = source.index_of(id).expect("the row is listed");
    let entry = source.entry(index);
    (
        entry.color,
        entry.glyph.map(OwnedGlyph::from),
        entry.thumbnail.is_some(),
    )
}

struct WornCase {
    first_tab: crate::slots::TabKey,
    shot_id: EntryId,
    pictures_id: EntryId,
    blue: Option<Color>,
    green: Option<Color>,
    red: Option<Color>,
    code_icon: Option<OwnedGlyph>,
    astro: Option<OwnedGlyph>,
}

#[test]
fn customizations_reach_rows_filter_hits_switched_views_hidden_tabs_and_sidebar() {
    hosted(
        MODULE,
        "customizations_reach_rows_filter_hits_switched_views_hidden_tabs_and_sidebar",
        |mut host| {
            let folder = fixture();
            let shot = folder.path().join("shot.png");
            std::fs::write(&shot, b"png").expect("image file");
            let pictures = folder.path().join("pictures");
            let root = Location::local(folder.path());

            host.app().open(root.clone());
            let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, listed), "first tab listed");
            idle(&mut host);

            let first_tab = shell::tabs::shown_tab(host.app());
            host.runtime().update(|app, tx| {
                shell::tabs::open(app, Some(root), 1, tx);
            });
            assert!(host.run_until(PATIENCE, listed), "second tab listed");
            idle(&mut host);

            let red = customize_first_place(&mut host);
            let shot_id = shell::id_of(host.app(), &Location::local(&shot));
            let pictures_id = shell::id_of(host.app(), &Location::local(&pictures));
            let (blue, code_icon) = customize_shot_row(&mut host, shot_id);
            let (green, astro) = customize_pictures_row(&mut host, pictures_id);

            let case = WornCase {
                first_tab,
                shot_id,
                pictures_id,
                blue,
                green,
                red,
                code_icon,
                astro,
            };
            assert_switched_views(&mut host, &case);
            assert_filter_clear_and_hidden_tab(&mut host, &case);
        },
    );
}

fn customize_first_place(host: &mut Offscreen) -> Option<Color> {
    let home_location = host.app().sidebar.first_place_location();
    host.runtime().update(|app, tx| {
        commands::run(app, "customize", On::Place(home_location), tx);
    });
    idle(host);
    click(host, "customize-color-red");
    click(host, "customize-icon-6");
    let red = color_of(&FolderColorValue::Preset(FolderColor::Red));
    assert_eq!(host.app().sidebar.places()[0].tint, red);
    assert_eq!(
        host.app().sidebar.places()[0].icon,
        RowIcon::Catalog("strata-terminal".to_owned())
    );
    red
}

fn customize_shot_row(
    host: &mut Offscreen,
    shot_id: EntryId,
) -> (Option<Color>, Option<OwnedGlyph>) {
    assert_eq!(
        entry_worn(host.app().columns.listing(0).unwrap(), shot_id),
        (None, None, true)
    );
    host.runtime().update(|app, tx| {
        commands::run(app, "customize", On::Row((0, shot_id)), tx);
    });
    idle(host);
    click(host, "customize-color-blue");
    let blue = color_of(&FolderColorValue::Preset(FolderColor::Blue));
    assert_eq!(
        entry_worn(host.app().columns.listing(0).unwrap(), shot_id),
        (blue, None, true),
        "colour alone keeps a file's thumbnail"
    );
    click(host, "customize-icon-2");
    let code_icon = Some(OwnedGlyph::Icon("strata-file-code".into()));
    assert_eq!(
        entry_worn(host.app().columns.listing(0).unwrap(), shot_id),
        (blue, code_icon.clone(), false),
        "a custom icon suppresses a file's thumbnail"
    );
    (blue, code_icon)
}

fn customize_pictures_row(
    host: &mut Offscreen,
    pictures_id: EntryId,
) -> (Option<Color>, Option<OwnedGlyph>) {
    host.runtime().update(|app, tx| {
        commands::run(app, "customize", On::Row((0, pictures_id)), tx);
    });
    idle(host);
    click(host, "customize-color-green");
    click(host, "customize-emoji-button");
    press(host, Key::Text("astronaut".to_owned()), Mods::default());
    press(host, Key::Enter, Mods::default());
    press(host, Key::Escape, Mods::default());
    (
        color_of(&FolderColorValue::Preset(FolderColor::Green)),
        Some(OwnedGlyph::Emoji("🧑\u{200d}🚀".into())),
    )
}

fn assert_switched_views(host: &mut Offscreen, case: &WornCase) {
    for mode in ["icons", "list"] {
        host.runtime().update(|app, tx| {
            assert!(shell::show_view(app, mode, tx));
        });
        idle(host);
    }
    assert_eq!(
        entry_worn(host.app().list.source(), case.pictures_id),
        (case.green, case.astro.clone(), false)
    );
    assert_eq!(
        entry_worn(host.app().list.source(), case.shot_id),
        (case.blue, case.code_icon.clone(), false)
    );
    host.runtime().update(|app, tx| {
        assert!(shell::show_view(app, "icons", tx));
    });
    idle(host);
    assert_eq!(
        entry_worn(host.app().icons.source(), case.pictures_id),
        (case.green, case.astro.clone(), false)
    );
}

fn assert_filter_clear_and_hidden_tab(host: &mut Offscreen, case: &WornCase) {
    host.runtime().update(|app, tx| {
        assert_eq!(commands::run(app, "filter", On::Focus, tx), Outcome::Ran);
    });
    idle(host);
    press(host, Key::Text("shot".to_owned()), Mods::default());
    let shot_id = case.shot_id;
    let hit = |host: &Offscreen| crate::filter::targets::at(host.app(), 0, shot_id).is_some();
    assert!(host.run_until(PATIENCE, hit), "shot filter hit");
    assert_eq!(
        entry_worn(host.app().filter.pane.source(), shot_id),
        (case.blue, case.code_icon.clone(), false)
    );

    host.runtime().update(|app, tx| {
        commands::run(app, "customize", On::Row((0, shot_id)), tx);
    });
    idle(host);
    let clear_key = format!("customize-{}-secondary", host.app().customize.opened);
    click(host, &clear_key);
    assert_eq!(
        entry_worn(host.app().filter.pane.source(), shot_id),
        (None, None, true),
        "Clear restores the file's thumbnail across filter results and panes"
    );
    assert_eq!(
        entry_worn(host.app().icons.source(), shot_id),
        (None, None, true)
    );

    let first_tab = case.first_tab;
    host.runtime().update(|app, tx| {
        assert!(shell::tabs::show(app, first_tab, tx));
    });
    idle(host);
    assert_eq!(
        entry_worn(host.app().columns.listing(0).unwrap(), case.pictures_id),
        (case.green, case.astro.clone(), false)
    );
    assert_eq!(host.app().sidebar.places()[0].tint, case.red);
}
