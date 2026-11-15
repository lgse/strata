// SPDX-License-Identifier: MIT

//! The filter's results in the place of a column's rows: what a command on them acts on, and
//! what holds the keyboard after the view changes.

use super::{MODULE, find, fixture, idle, open, press, seen, text, wait};
use crate::{
    commands::Holds,
    filter::Filter,
    host::offscreen::Offscreen,
    slots::Part,
    test_support::{PATIENCE, hosted},
};

/// The name and the text of each text field the tree shows.
fn fields(host: &Offscreen) -> Vec<(String, String)> {
    let tree = host.tree();
    find(&tree, &|node| node["role"] == "text-input")
        .into_iter()
        .map(|node| (text(node, "name"), text(node, "value")))
        .collect()
}

fn type_text(host: &mut Offscreen, typed: &str) {
    for character in typed.chars() {
        press(host, &character.to_string());
    }
}

/// Filters the fixture's first column down to `photo.txt`, which is in `pictures`, with
/// `todo.txt` selected before the filter.
fn filtered_to_photo(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    open(host, folder.path());
    press(host, "End");
    wait(host, "todo.txt", |seen| {
        seen.focused.as_deref() == Some("todo.txt")
    });
    press(host, "ctrl+f");
    type_text(host, "photo");
    wait(host, "the filter's one result", |seen| {
        seen.panes[0].names() == ["photo.txt"]
    });
    folder
}

/// GTK's `begin_rename` asks `selected_search_result` first: F2 on a result renames it, not
/// the entry the browser had selected before the filter, and the new name reaches the disk.
#[test]
fn f2_on_a_filter_result_renames_the_result() {
    hosted(
        MODULE,
        "filter::f2_on_a_filter_result_renames_the_result",
        |mut host| {
            let folder = filtered_to_photo(&mut host);
            press(&mut host, "Down");
            wait(&mut host, "the result under the cursor", |seen| {
                seen.focused.as_deref() == Some("photo.txt")
            });

            press(&mut host, "F2");

            let reached = host.run_until(PATIENCE, |host| {
                fields(host).iter().any(|(name, _)| name == "Rename")
            });
            let shown = fields(&host);
            assert!(reached, "no rename editor: {shown:?}");
            assert!(
                shown.contains(&("Rename".to_owned(), "photo.txt".to_owned())),
                "the editor names another entry: {shown:?}"
            );

            // The editor selects the stem, which the typing replaces.
            type_text(&mut host, "snap");
            press(&mut host, "Return");
            let renamed = folder.path().join("pictures/snap.txt");
            let committed = host.run_until(PATIENCE, |_| renamed.exists());
            assert!(committed, "the result was not renamed on disk");
            assert!(
                folder.path().join("todo.txt").exists(),
                "todo.txt was renamed"
            );
        },
    );
}

/// A view switch keeps the filter, its query and its results, and the keyboard in its field
/// (GTK's `ActivePaneFilter` and `restore_view_switch_focus`), into Icons and back.
#[test]
fn a_view_switch_keeps_the_filter_with_its_field_focused() {
    hosted(
        MODULE,
        "filter::a_view_switch_keeps_the_filter_with_its_field_focused",
        |mut host| {
            let _folder = filtered_to_photo(&mut host);

            press(&mut host, "ctrl+2");
            let field_kept = |host: &Offscreen| {
                fields(host) == [("Filter icons (Ctrl+F)".to_owned(), "photo".to_owned())]
                    && focused_field(host)
            };
            let switched = host.run_until(PATIENCE, |host| {
                seen(host).panes.is_empty() && field_kept(host)
            });
            assert!(
                switched,
                "Icons has no focused filter field: {:?}",
                fields(&host)
            );
            assert_eq!(<Filter as Part>::keyboard(host.app()), Some(Holds::Filter));
            let tree = host.tree();
            let results = find(&tree, &|node| node["name"] == "Search results");
            assert_eq!(results.len(), 1, "no results in Icons");

            press(&mut host, "ctrl+1");
            wait(&mut host, "the result back in Columns", |seen| {
                seen.panes
                    .first()
                    .is_some_and(|pane| pane.names() == ["photo.txt"])
            });
            let back = host.run_until(PATIENCE, |host| {
                fields(host) == [("Filter this pane (Ctrl+F)".to_owned(), "photo".to_owned())]
                    && focused_field(host)
            });
            assert!(
                back,
                "Columns has no focused filter field: {:?}",
                fields(&host)
            );
        },
    );
}

/// Whether a text field has the keyboard.
fn focused_field(host: &Offscreen) -> bool {
    let tree = host.tree();
    find(&tree, &|node| node["role"] == "text-input")
        .iter()
        .any(|node| {
            node["states"]
                .as_array()
                .is_some_and(|states| states.iter().any(|state| state == "focused"))
        })
}

/// After a view switch kept the filter, Escape in its field closes it, and no part holds
/// the keyboard as a filter any more.
#[test]
fn escape_after_a_view_switch_releases_the_filters_hold() {
    hosted(
        MODULE,
        "filter::escape_after_a_view_switch_releases_the_filters_hold",
        |mut host| {
            let _folder = filtered_to_photo(&mut host);
            press(&mut host, "ctrl+2");
            let switched = host.run_until(PATIENCE, |host| {
                seen(host).panes.is_empty() && focused_field(host)
            });
            assert!(switched, "Icons has no focused filter field");
            assert_eq!(<Filter as Part>::keyboard(host.app()), Some(Holds::Filter));

            press(&mut host, "Escape");

            let closed = host.run_until(PATIENCE, |host| fields(host).is_empty());
            assert!(closed, "the filter stayed: {:?}", fields(&host));
            assert_eq!(<Filter as Part>::keyboard(host.app()), None);
        },
    );
}

/// Tab in a rename on a result commits the name and opens no other editor, as GTK's: the
/// renamed result keeps the keyboard (`renamed-filter-result-keeps-focus`, its `tab` runs).
/// Here the next row of the results is a result, which a step would open an editor on.
#[test]
fn tab_in_a_rename_on_a_result_only_commits() {
    hosted(
        MODULE,
        "filter::tab_in_a_rename_on_a_result_only_commits",
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());
            press(&mut host, "ctrl+f");
            type_text(&mut host, "t");
            wait(&mut host, "the filter's results", |seen| {
                seen.panes[0].names().len() > 2
            });
            press(&mut host, "Down");
            let landed = wait(&mut host, "the top result under the cursor", |seen| {
                seen.focused.is_some()
            });
            let first = landed.focused.expect("the top result");
            press(&mut host, "F2");
            let editing = host.run_until(PATIENCE, |host| {
                fields(host).contains(&("Rename".to_owned(), first.clone()))
            });
            assert!(editing, "no editor on {first}: {:?}", fields(&host));
            press(&mut host, "End");
            type_text(&mut host, "x");

            press(&mut host, "Tab");

            let renamed = format!("{first}x");
            let committed = host.run_until(PATIENCE, |_| found_under(folder.path(), &renamed));
            assert!(committed, "{first} was not renamed");
            let after = wait(&mut host, "the renamed result in the column", |seen| {
                seen.panes[0].names().contains(&renamed.as_str())
            });
            assert_eq!(after.focused.as_deref(), Some(renamed.as_str()));
            assert_eq!(after.panes[0].selected(), [renamed.as_str()]);
            idle(&mut host);
            assert!(
                !fields(&host).iter().any(|(name, _)| name == "Rename"),
                "Tab opened another editor: {:?}",
                fields(&host)
            );
        },
    );
}

/// Whether a file named `name` is in `folder` or a folder under it.
fn found_under(folder: &std::path::Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        entry.file_name() == name || (path.is_dir() && found_under(&path, name))
    })
}

fn click_key(host: &mut Offscreen, key: &str) {
    let elements = host.runtime().elements();
    let found = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key));
    let [x, y, w, h] = found.unwrap_or_else(|| panic!("{key} is drawn")).visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

/// Leaving a filtered column for window chrome (`header-appearance`) closes its filter and
/// restores its rows and selection, while opening the column's own Sort menu keeps it open
/// (`close_filter_after_blur` in `src/ui/browser/columns/append/shell.rs`).
#[test]
fn leaving_a_filtered_column_for_chrome_closes_its_filter() {
    hosted(
        MODULE,
        "filter::leaving_a_filtered_column_for_chrome_closes_its_filter",
        |mut host| {
            let _folder = filtered_to_photo(&mut host);

            click_key(&mut host, "columns-sort-0");
            assert_eq!(
                fields(&host),
                [("Filter this pane (Ctrl+F)".to_owned(), "photo".to_owned())],
                "the column's Sort menu closed the filter"
            );
            press(&mut host, "Escape");
            assert_eq!(
                fields(&host),
                [("Filter this pane (Ctrl+F)".to_owned(), "photo".to_owned())],
                "dismissing the column's Sort menu closed the filter"
            );

            click_key(&mut host, "header-appearance");
            let closed = host.run_until(PATIENCE, |host| fields(host).is_empty());
            assert!(
                closed,
                "Appearance left the Columns filter open: {:?}",
                fields(&host)
            );
            let restored = wait(&mut host, "the restored column listing", |seen| {
                seen.panes[0].names().contains(&"todo.txt")
            });
            assert_eq!(restored.panes[0].selected(), ["todo.txt"]);
        },
    );
}
