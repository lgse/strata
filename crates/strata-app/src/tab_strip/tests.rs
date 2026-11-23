// SPDX-License-Identifier: MIT

use std::{path::Path, time::Duration};

use rune::{Key, KeyEvent, Mods, Tx};
use strata_components::tabs::TabIcon;
use strata_core::model::{FolderColor, FolderColorValue, Location};
use strata_desktop::settings::Preferences;
use strata_kit::icons::IconName;

use strata_platform::csd::Outcome as Sent;

use super::{named, select, tab_icon};
use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    shell::{
        self,
        tabs::{shown_tab, tab_browser, tabs},
    },
    slots::{TabKey, color_of},
    test_support::{
        PATIENCE, fixture, held, hosted, send_key,
        view::{all, field, hosted_in, hosted_with, idle, press, text},
    },
};

mod badges;
mod drops;

const MODULE: &str = "tab_strip::tests";

pub(super) fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

/// The window opened at `folder`, listed.
pub(super) fn opened(host: &mut Offscreen, folder: &Path) {
    host.app().open(Location::local(folder));
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder's listing");
    idle(host);
}

/// The tabs' locations, in the strip's order, once each has opened.
pub(super) fn located(host: &mut Offscreen, count: usize) -> Vec<Option<Location>> {
    let all_there = |host: &Offscreen| {
        let order = tabs(host.app());
        order.len() == count
            && order.iter().all(|&key| {
                tab_browser(host.app(), key).is_some_and(|tab| tab.active_location().is_some())
            })
    };
    assert!(
        host.run_until(PATIENCE, all_there),
        "{count} tabs, each open"
    );
    let app = host.app();
    let order = tabs(app);
    let at = |key: &TabKey| tab_browser(app, *key).and_then(|tab| tab.active_location());
    order.iter().map(at).collect()
}

pub(super) fn act<T>(host: &mut Offscreen, act: impl FnOnce(&mut App, &mut Tx) -> T) -> T {
    let mut said = None;
    host.runtime().update(|app, tx| said = Some(act(app, tx)));
    idle(host);
    said.expect("the update ran")
}

pub(super) fn two_tabs_at(host: &mut Offscreen, folder: &Path) -> TabKey {
    let at = Location::local(folder);
    host.app().open(at.clone());
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder's listing");
    idle(host);
    let first = shown_tab(host.app());
    let ran = act(host, |app, tx| run(app, "tab-new", On::Focus, tx));
    assert_eq!(ran, Outcome::Ran);
    assert_eq!(tabs(host.app()).len(), 2, "New tab opened a tab");
    let opened = |host: &Offscreen| {
        host.app().browser().active_location() == Some(at.clone())
            && host.app().browser().column_entry_counts(0).is_some()
    };
    assert!(
        host.run_until(PATIENCE, opened),
        "the new tab at the same folder"
    );
    idle(host);
    first
}

fn typed(host: &mut Offscreen, text: &str) {
    for character in text.chars() {
        host.runtime().key(KeyEvent {
            key: Key::Text(character.to_string()),
            mods: Mods::default(),
        });
    }
    idle(host);
}

#[test]
fn a_rename_open_when_its_tab_is_hidden_is_committed() {
    hosted(
        MODULE,
        "a_rename_open_when_its_tab_is_hidden_is_committed",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            let second = shown_tab(host.app());
            let todo = Location::local(folder.path().join("todo.txt"));
            act(&mut host, |app, tx| {
                let row = On::Row((0, shell::id_of(app, &todo)));
                assert_eq!(run(app, "rename", row, tx), Outcome::Ran);
            });
            assert!(field(&host).is_some(), "the editor opened");
            typed(&mut host, "zz");

            assert!(act(&mut host, |app, tx| select(app, first, tx)));
            let renamed = folder.path().join("zz.txt");
            let committed = host.run_until(PATIENCE, |_| renamed.exists());
            assert!(committed, "the name typed was committed");
            assert!(!folder.path().join("todo.txt").exists());

            assert!(act(&mut host, |app, tx| select(app, second, tx)));
            assert_eq!(field(&host), None, "the tab shown again has no edit open");
        },
    );
}

#[test]
fn a_mirror_due_in_a_hidden_tab_opens_its_column_in_that_tab_alone() {
    hosted(
        MODULE,
        "a_mirror_due_in_a_hidden_tab_opens_its_column_in_that_tab_alone",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            let second = shown_tab(host.app());
            let documents = Location::local(folder.path().join("documents"));
            assert_eq!(
                host.app().browser().location_at(1),
                None,
                "no child column yet"
            );

            let focus = shell::view_focus(host.app()).expect("the view's key");
            act(&mut host, |_, tx| tx.focus(focus, true));
            // The switch comes before the mirror's wait is over, or just after it.
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Down,
                    mods: Mods::default(),
                },
            );
            // One frame hands the key to the view and its cursor to the browser.
            host.turn();
            let moved = host
                .app()
                .browser()
                .focused_item()
                .map(|(depth, at, _)| (depth, at));
            assert_eq!(moved, Some((0, 1)), "Down moved the cursor onto documents");
            assert!(act(&mut host, |app, tx| select(app, first, tx)));
            host.run_until(Duration::from_millis(400), |_| false);
            let shown = host.app().browser().location_at(1);
            assert_eq!(shown, None, "the tab shown opened no column");

            assert!(act(&mut host, |app, tx| select(app, second, tx)));
            let opened =
                |host: &Offscreen| host.app().browser().location_at(1) == Some(documents.clone());
            let mirrored = host.run_until(PATIENCE, opened);
            assert!(mirrored, "the mirror opened documents in its own tab");
        },
    );
}

#[test]
fn a_menu_row_opens_each_folder_it_acts_on_in_a_tab() {
    hosted(
        MODULE,
        "a_menu_row_opens_each_folder_it_acts_on_in_a_tab",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let at = |name: &str| Location::local(folder.path().join(name));
            let names = ["documents", "pictures", "readme.md"].map(str::to_owned);
            act(&mut host, |app, _| {
                app.browser.select_entries_by_name_at(0, &names);
            });
            let ran = act(&mut host, |app, tx| {
                let row = On::Row((0, shell::id_of(app, &at("documents"))));
                run(app, "open-in-tab", row, tx)
            });
            assert_eq!(ran, Outcome::Ran);
            let first = Location::local(folder.path());
            assert_eq!(
                located(&mut host, 3),
                [Some(first), Some(at("documents")), Some(at("pictures"))]
            );
            assert_eq!(Some(&shown_tab(host.app())), tabs(host.app()).last());
        },
    );
}

#[test]
fn ctrl_enter_opens_the_cursors_folder_in_a_new_tab() {
    hosted(
        MODULE,
        "ctrl_enter_opens_the_cursors_folder_in_a_new_tab",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let view = shell::view_focus(host.app()).expect("the view's key");
            act(&mut host, |_, tx| tx.focus(view, true));
            press(&mut host, Key::Down, Mods::default());
            press(&mut host, Key::Enter, ctrl());
            let documents = Location::local(folder.path().join("documents"));
            let first = Location::local(folder.path());
            assert_eq!(located(&mut host, 2), [Some(first), Some(documents)]);
            assert_eq!(Some(&shown_tab(host.app())), tabs(host.app()).last());
        },
    );
}

/// The name of the dialog shown, if one is.
pub(super) fn dialog(host: &Offscreen) -> Option<String> {
    let tree = host.tree();
    let dialogs = all(&tree, &|node| node["role"] == "dialog");
    dialogs.first().map(|dialog| text(dialog, "name"))
}

pub(super) const OPERATIONS_ACTIVE: &str = "File operations are still active";

#[test]
fn a_tab_with_an_operation_running_stays_open_and_says_so() {
    hosted(
        MODULE,
        "a_tab_with_an_operation_running_stays_open_and_says_so",
        |mut host| {
            let folder = fixture();
            two_tabs_at(&mut host, folder.path());
            act(&mut host, |app, _| held::busy(&app.browser, "todo.txt"));
            act(&mut host, |app, tx| run(app, "tab-close", On::Focus, tx));
            let said = host.run_until(PATIENCE, |host| dialog(host).is_some());
            assert!(said, "a notice");
            assert_eq!(dialog(&host).as_deref(), Some(OPERATIONS_ACTIVE));
            assert_eq!(tabs(host.app()).len(), 2, "the tab stays");
        },
    );
}

#[test]
fn the_window_is_not_closed_while_a_hidden_tab_has_an_operation_running() {
    hosted(
        MODULE,
        "the_window_is_not_closed_while_a_hidden_tab_has_an_operation_running",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            act(&mut host, |app, _| {
                let browser = tab_browser(app, first).expect("the first tab");
                held::busy(browser, "todo.txt");
            });
            click_key(&mut host, "window-close");
            let said = host.run_until(PATIENCE, |host| dialog(host).is_some());
            assert!(said, "a notice");
            assert_eq!(dialog(&host).as_deref(), Some(OPERATIONS_ACTIVE));
            let sent = act(&mut host, |app, _| {
                shell::csd_of(app).last_outcome().cloned()
            });
            assert!(
                !matches!(sent, Some(Sent::Sent { .. })),
                "no close was sent: {sent:?}"
            );
        },
    );
}

/// Presses the element keyed `key` with the pointer.
fn click_key(host: &mut Offscreen, key: &str) {
    let elements = host.runtime().elements();
    let element = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = element.expect("the element is mounted").visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    let runtime = host.runtime();
    runtime.pointer_move(x, y);
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    idle(host);
}

fn filter_across_a_switch(host: &mut Offscreen, folder: &Path, back: Option<(String, bool)>) {
    opened(host, folder);
    let view = shell::view_focus(host.app()).expect("the view's key");
    act(host, |_, tx| tx.focus(view, true));
    press(host, Key::Text("f".to_owned()), ctrl());
    typed(host, "photo");
    let typing = |host: &Offscreen| field(host) == Some(("photo".to_owned(), true));
    assert!(host.run_until(PATIENCE, typing), "the query in the field");
    let first = shown_tab(host.app());
    act(host, |app, tx| run(app, "tab-new", On::Focus, tx));
    assert_eq!(tabs(host.app()).len(), 2);
    assert!(act(host, |app, tx| select(app, first, tx)));
    let reached = host.run_until(PATIENCE, |host| field(host) == back);
    assert!(reached, "the field is {:?}, not {back:?}", field(host));
}

#[test]
fn a_tab_left_from_a_panes_filter_field_gets_the_field_back() {
    hosted_in(
        MODULE,
        "list",
        "a_tab_left_from_a_panes_filter_field_gets_the_field_back",
        |mut host| {
            let folder = fixture();
            let back = Some(("photo".to_owned(), true));
            filter_across_a_switch(&mut host, folder.path(), back);
        },
    );
}

#[test]
fn a_tab_left_from_a_columns_filter_comes_back_with_it_closed() {
    hosted_in(
        MODULE,
        "columns",
        "a_tab_left_from_a_columns_filter_comes_back_with_it_closed",
        |mut host| {
            let folder = fixture();
            filter_across_a_switch(&mut host, folder.path(), None);
        },
    );
}

/// Whether the element keyed `key` is drawn inside the shown tab's content (`tab-<n>`).
fn drawn_in_shown_tab(host: &mut Offscreen, key: &str) -> bool {
    let tab = format!("tab-{}", shown_tab(host.app()).0);
    let elements = host.runtime().elements();
    let in_tab = |mut at: Option<usize>| {
        while let Some(index) = at {
            let element = &elements[index];
            if element.key.as_deref() == Some(tab.as_str()) {
                return !element.exiting;
            }
            at = element.parent;
        }
        false
    };
    let drawn =
        |element: &&rune::ElementInfo| !element.exiting && element.key.as_deref() == Some(key);
    elements
        .iter()
        .filter(drawn)
        .any(|element| in_tab(element.parent))
}

#[test]
fn a_new_tabs_first_frame_has_its_sidebar() {
    hosted(
        MODULE,
        "a_new_tabs_first_frame_has_its_sidebar",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            assert!(drawn_in_shown_tab(&mut host, "sidebar"), "the first tab's");
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Text("t".to_owned()),
                    mods: ctrl(),
                },
            );
            host.turn();
            assert_eq!(tabs(host.app()).len(), 2, "Ctrl+T opened a tab");
            assert!(drawn_in_shown_tab(&mut host, "sidebar"), "the new tab's");
        },
    );
}

/// What has the keyboard, as an update reads it.
fn keyboard(host: &mut Offscreen) -> Option<String> {
    act(host, |_, tx| tx.focused().map(str::to_owned))
}

/// Presses the strip's tab at `index` with the pointer.
fn click_tab(host: &mut Offscreen, index: usize) {
    let tree = host.tree();
    let tabs = crate::test_support::view::all(&tree, &|node| node["role"] == "tab");
    let bounds = &tabs.get(index).expect("the tab")["bounds"];
    let at = |i: usize| bounds[i].as_f64().expect("a bound");
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let (x, y) = ((at(0) + at(2) / 2.0) as f32, (at(1) + at(3) / 2.0) as f32);
    let runtime = host.runtime();
    runtime.pointer_move(x, y);
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    idle(host);
}

#[test]
fn a_tab_left_by_a_press_on_the_strip_gets_the_keyboard_back_in_its_view() {
    hosted(
        MODULE,
        "a_tab_left_by_a_press_on_the_strip_gets_the_keyboard_back_in_its_view",
        |mut host| {
            let folder = fixture();
            let first = two_tabs_at(&mut host, folder.path());
            let second = shown_tab(host.app());
            let view = shell::view_focus(host.app()).expect("the view's key");
            assert_eq!(keyboard(&mut host).as_deref(), Some(view));

            click_tab(&mut host, 0);
            assert_eq!(
                shown_tab(host.app()),
                first,
                "the press showed the first tab"
            );
            assert!(act(&mut host, |app, tx| select(app, second, tx)));
            assert_eq!(keyboard(&mut host).as_deref(), Some(view));
            assert!(act(&mut host, |app, tx| select(app, first, tx)));
            assert_eq!(
                keyboard(&mut host).as_deref(),
                Some(view),
                "the first tab's view"
            );
        },
    );
}

#[test]
fn the_close_buttons_speak_the_users_language() {
    hosted_with(
        MODULE,
        "columns",
        "language = \"fr\"\n".to_owned(),
        "the_close_buttons_speak_the_users_language",
        |mut host| {
            let folder = fixture();
            two_tabs_at(&mut host, folder.path());
            let tree = host.tree();
            let lists = all(&tree, &|node| node["role"] == "tab-list");
            let list = lists.first().expect("the tab list");
            assert_eq!(
                text(list, "name"),
                rust_i18n::t!("Tabs", locale = "fr").as_ref()
            );
            let close_buttons: Vec<_> = all(list, &|node| node["role"] == "button")
                .into_iter()
                .map(|node| text(node, "name"))
                .collect();
            let expected = rust_i18n::t!("Close tab", locale = "fr").into_owned();
            assert_eq!(close_buttons, [expected.clone(), expected]);
        },
    );
}

#[test]
fn tab_icon_resolves_folder_customization_and_falls_back_when_uncustomized() {
    let home_path = Path::new("/home/tester");
    let home = Location::local(home_path);
    let project_path = Path::new("/home/tester/project");
    let project = Location::local(project_path);
    let docs_path = Path::new("/home/tester/docs");
    let docs = Location::local(docs_path);

    let mut preferences = Preferences::default();
    assert_eq!(
        tab_icon(None, &home, &preferences),
        (TabIcon::Name(IconName::Home), None)
    );
    assert_eq!(
        tab_icon(Some(&home), &home, &preferences),
        (TabIcon::Name(IconName::Home), None)
    );
    assert_eq!(
        tab_icon(Some(&Location::local(Path::new("/"))), &home, &preferences),
        (TabIcon::Name(IconName::Drive), None)
    );
    assert_eq!(
        tab_icon(Some(&Location::uri("trash:///")), &home, &preferences),
        (TabIcon::Name(IconName::Trash), None)
    );
    assert_eq!(
        tab_icon(
            Some(&Location::uri("sftp://example/share")),
            &home,
            &preferences
        ),
        (TabIcon::Name(IconName::Network), None)
    );
    assert_eq!(
        tab_icon(Some(&project), &home, &preferences),
        (TabIcon::Name(IconName::Folder), None)
    );

    let red = FolderColorValue::Preset(FolderColor::Red);
    let custom = FolderColorValue::Custom("#34d399".to_owned());
    preferences.set_item_colors(&[home_path.to_path_buf()], Some(&red));
    preferences.set_item_colors(&[project_path.to_path_buf()], Some(&custom));
    preferences.set_item_icon(project_path, Some("strata-terminal"));
    preferences.set_item_icon(docs_path, Some("emoji:🚀"));

    assert_eq!(
        tab_icon(Some(&home), &home, &preferences),
        (TabIcon::Name(IconName::Home), color_of(&red)),
        "color-only customization keeps the semantic place icon"
    );
    assert_eq!(
        tab_icon(Some(&project), &home, &preferences),
        (
            TabIcon::Catalog("strata-terminal".into()),
            color_of(&custom)
        ),
        "custom icon and color both apply"
    );
    assert_eq!(
        tab_icon(Some(&docs), &home, &preferences),
        (TabIcon::Emoji("🚀".into()), None),
        "emoji customization without color leaves tint unset"
    );

    preferences.clear_item_customization(project_path);
    assert_eq!(
        tab_icon(Some(&project), &home, &preferences),
        (TabIcon::Name(IconName::Folder), None),
        "clearing customization restores the default folder icon"
    );
}

fn open_pictures_column_in_second_tab(host: &mut Offscreen, folder: &Path) {
    let documents = folder.join("documents");
    let pictures = folder.join("pictures");
    opened(host, &documents);
    act(host, |app, tx| {
        super::open_in_new_tab(app, Location::local(folder), tx);
    });
    let ready = |h: &Offscreen| {
        tabs(h.app()).len() == 2
            && h.app().browser().active_location() == Some(Location::local(folder))
            && h.app().browser().column_entry_counts(0).is_some()
    };
    assert!(host.run_until(PATIENCE, ready), "second tab listed");
    idle(host);

    act(host, |app, _| {
        app.browser
            .select_entries_by_name_at(0, &["pictures".to_owned()]);
        app.browser.activate_focused();
    });
    let opened_pictures = |h: &Offscreen| {
        h.app().browser().active_location() == Some(Location::local(&pictures))
            && h.app().browser().column_entry_counts(1).is_some()
    };
    assert!(
        host.run_until(PATIENCE, opened_pictures),
        "pictures column opened in active tab"
    );
    idle(host);
}

fn rename_folder_and_focus_relocated(
    host: &mut Offscreen,
    old_path: &Path,
    new_name: &str,
    expected_icon: &str,
) {
    let old_loc = Location::local(old_path);
    act(host, |app, tx| {
        let row = On::Row((0, shell::id_of(app, &old_loc)));
        assert_eq!(run(app, "rename", row, tx), Outcome::Ran);
    });
    assert!(field(host).is_some(), "rename editor opened");
    typed(host, new_name);
    press(host, Key::Enter, Mods::default());

    let new_path = old_path.parent().expect("parent folder").join(new_name);
    let relocated = |h: &Offscreen| {
        new_path.exists()
            && h.app().browser().location_at(1) == Some(Location::local(&new_path))
            && shell::preferences(h.app()).item_icon(&new_path) == Some(expected_icon)
    };
    assert!(
        host.run_until(PATIENCE, relocated),
        "renamed folder and its customization relocated"
    );
    idle(host);

    if host.app().browser().active_location() != Some(Location::local(&new_path)) {
        act(host, |app, _| {
            app.browser
                .select_entries_by_name_at(0, &[new_name.to_owned()]);
            app.browser.activate_focused();
        });
        let active =
            |h: &Offscreen| h.app().browser().active_location() == Some(Location::local(&new_path));
        assert!(host.run_until(PATIENCE, active));
        idle(host);
    }
}

#[test]
fn open_tabs_update_live_on_customization_clear_and_folder_rename() {
    hosted(
        MODULE,
        "open_tabs_update_live_on_customization_clear_and_folder_rename",
        |mut host| {
            let folder = fixture();
            let documents = folder.path().join("documents");
            let pictures = folder.path().join("pictures");
            open_pictures_column_in_second_tab(&mut host, folder.path());

            let initial = named(host.app());
            assert_eq!(initial.len(), 2);
            assert_eq!(initial[0].icon, TabIcon::Name(IconName::Folder));
            assert_eq!(initial[0].tint, None);
            assert_eq!(initial[1].icon, TabIcon::Name(IconName::Folder));
            assert_eq!(initial[1].tint, None);

            let blue = FolderColorValue::Preset(FolderColor::Blue);
            let green = FolderColorValue::Preset(FolderColor::Green);
            act(&mut host, |app, _| {
                shell::change_customization(app, |settings| {
                    settings.set_item_colors(std::slice::from_ref(&documents), Some(&blue));
                    settings.set_item_icon(&documents, Some("strata-terminal"));
                    settings.set_item_colors(std::slice::from_ref(&pictures), Some(&green));
                    settings.set_item_icon(&pictures, Some("emoji:🚀"));
                });
            });

            let customized = named(host.app());
            assert_eq!(
                customized[0].icon,
                TabIcon::Catalog("strata-terminal".into())
            );
            assert_eq!(customized[0].tint, color_of(&blue));
            assert_eq!(customized[1].icon, TabIcon::Emoji("🚀".into()));
            assert_eq!(customized[1].tint, color_of(&green));

            act(&mut host, |app, _| {
                shell::change_customization(app, |settings| {
                    settings.clear_item_customization(&documents);
                });
            });
            let cleared = named(host.app());
            assert_eq!(cleared[0].icon, TabIcon::Name(IconName::Folder));
            assert_eq!(cleared[0].tint, None);

            rename_folder_and_focus_relocated(&mut host, &pictures, "photos", "emoji:🚀");
            let after_rename = named(host.app());
            assert_eq!(after_rename[1].name, "photos");
            assert_eq!(after_rename[1].icon, TabIcon::Emoji("🚀".into()));
            assert_eq!(after_rename[1].tint, color_of(&green));
        },
    );
}
