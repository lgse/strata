// SPDX-License-Identifier: MIT

//! The filter's results in the place of a column's rows: what a command on them acts on, and
//! what holds the keyboard after the view changes.

use super::{MODULE, find, fixture, idle, open, press, seen, text, wait};
use crate::{
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
/// the entry the browser had selected before the filter.
#[test]
fn f2_on_a_filter_result_renames_the_result() {
    hosted(MODULE, "filter::f2_on_a_filter_result_renames_the_result", |mut host| {
        let _folder = filtered_to_photo(&mut host);
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
    });
}

/// The field leaves with the Columns view: no part says it holds the keyboard in the view
/// shown after it, so the window's commands are not held back there as in a text field.
#[test]
fn a_view_switch_with_the_field_focused_leaves_no_filter_owner() {
    hosted(
        MODULE,
        "filter::a_view_switch_with_the_field_focused_leaves_no_filter_owner",
        |mut host| {
            let _folder = filtered_to_photo(&mut host);
            assert!(<Filter as Part>::keyboard(host.app()).is_some());

            press(&mut host, "ctrl+2");
            let left = host.run_until(PATIENCE, |host| seen(host).panes.is_empty());
            idle(&mut host);

            assert!(left, "the Columns view stayed");
            assert_eq!(<Filter as Part>::keyboard(host.app()), None);
        },
    );
}

