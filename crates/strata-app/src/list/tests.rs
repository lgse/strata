// SPDX-License-Identifier: MIT

//! The List view's file-type groups on the offscreen host, over a real folder, read from the
//! accessibility tree as the QA harness reads it: what only a view that groups does. What the
//! List view shares with the Icons view is tested once for both, in `single::tests`.

use std::time::Duration;

use rune::{Key, Mods};
use strata_core::model::Location;

use crate::{
    host::offscreen::Offscreen,
    test_support::{
        PATIENCE,
        view::{
            Shape, all, assert_browser_agrees, browser_holds, field, hosted_with, idle, open,
            press, seen, shift,
        },
    },
};

const MODULE: &str = "list::tests";

const LIST: Shape = Shape {
    described: "List view",
    list: "grid",
    entry: "row",
};

/// The saved setting that groups the List view by file type.
const GROUPED: &str = "group_by_type = true\n";

/// A folder whose browser order (by name) mixes three types: a folder, text and images.
fn mixed() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    std::fs::create_dir(folder.path().join("docs")).expect("folder");
    for name in ["a.png", "b.txt", "c.txt", "z.png"] {
        std::fs::write(folder.path().join(name), name).expect("file");
    }
    folder
}

/// Lets the browser's answer reach the view: a sort runs on the browser's own time.
fn settle(host: &mut Offscreen) {
    host.run_until(Duration::from_millis(300), |_| false);
    idle(host);
}

/// The index of the first row shown after the first group: where a group boundary is.
fn boundary(names: &[&str]) -> usize {
    let kind = |name: &str| name.rsplit_once('.').map_or("", |(_, ext)| ext).to_owned();
    (1..names.len())
        .find(|index| kind(names[*index]) != kind(names[index - 1]))
        .expect("two groups")
}

fn grouped() -> String {
    GROUPED.to_owned()
}

/// The first and last rows the grouped table shows are the browser's first and last rows
/// shown (`first_displayed_entry`, `last_displayed_entry`, through what is public: the cursor
/// a first visit places, and a step from no cursor), under the saved sort `saved`.
fn ends_agree(saved: &str, name: &str) {
    let more = format!("{GROUPED}{saved}");
    hosted_with(MODULE, "list", more, name, |mut host| {
        let folder = mixed();
        let listed = open(&mut host, LIST, folder.path());
        settle(&mut host);
        let shown = seen(&host, LIST);
        let names = shown.names();
        let (first, last) = (names.first().copied(), names.last().copied());
        assert_eq!(listed.names(), names, "the order shown, settled");
        let tree = host.tree();
        let headings = all(&tree, &|node| node["role"] == "heading");
        assert_eq!(headings.len(), 3, "a header for each group: {names:?}");

        assert_eq!(
            browser_holds(&host).1.as_deref(),
            first,
            "the first visit's cursor"
        );

        let browser = host.app().browser().clone();
        let missing = Location::local(folder.path().join("missing"));
        browser.reveal_locations(Location::local(folder.path()), vec![missing]);
        settle(&mut host);
        assert_eq!(browser_holds(&host).1, None, "no cursor to step from");
        browser.move_selection(-1);
        settle(&mut host);
        assert_eq!(
            browser_holds(&host).1.as_deref(),
            last,
            "a step back from no cursor"
        );
    });
}

#[test]
fn ends_agree_by_name_with_folders_first() {
    ends_agree(
        "sort_key = \"name\"\nfolders_first = true\n",
        "ends_agree_by_name_with_folders_first",
    );
}

#[test]
fn ends_agree_by_name_with_folders_mixed() {
    ends_agree(
        "sort_key = \"name\"\nfolders_first = false\n",
        "ends_agree_by_name_with_folders_mixed",
    );
}

#[test]
fn ends_agree_by_type_ascending_with_folders_first() {
    let name = "ends_agree_by_type_ascending_with_folders_first";
    ends_agree(
        "sort_key = \"type\"\nsort_direction = \"ascending\"\nfolders_first = true\n",
        name,
    );
}

#[test]
fn ends_agree_by_type_descending_with_folders_first() {
    let name = "ends_agree_by_type_descending_with_folders_first";
    ends_agree(
        "sort_key = \"type\"\nsort_direction = \"descending\"\nfolders_first = true\n",
        name,
    );
}

#[test]
fn ends_agree_by_type_ascending_with_folders_mixed() {
    let name = "ends_agree_by_type_ascending_with_folders_mixed";
    ends_agree(
        "sort_key = \"type\"\nsort_direction = \"ascending\"\nfolders_first = false\n",
        name,
    );
}

#[test]
fn ends_agree_by_type_descending_with_folders_mixed() {
    let name = "ends_agree_by_type_descending_with_folders_mixed";
    ends_agree(
        "sort_key = \"type\"\nsort_direction = \"descending\"\nfolders_first = false\n",
        name,
    );
}

/// Shift and an arrow from the last row of a group extend over its boundary to the first
/// row of the next, in the order shown, and the browser holds both.
#[test]
fn a_shift_range_crosses_a_group_boundary_in_the_order_shown() {
    let name = "a_shift_range_crosses_a_group_boundary_in_the_order_shown";
    hosted_with(MODULE, "list", grouped(), name, |mut host| {
        let folder = mixed();
        let listed = open(&mut host, LIST, folder.path());
        let names = listed.names();
        let at = boundary(&names);
        press(&mut host, Key::Home, Mods::default());
        for _ in 1..at {
            press(&mut host, Key::Down, Mods::default());
        }
        assert_eq!(seen(&host, LIST).focused.as_deref(), Some(names[at - 1]));

        press(&mut host, Key::Down, shift());

        let after = seen(&host, LIST);
        assert_eq!(after.selected, [names[at - 1], names[at]]);
        assert_eq!(after.focused.as_deref(), Some(names[at]));
        assert_browser_agrees(&host, LIST, "Shift+Down over a group boundary");
    });
}

/// Tab in a rename on the last row of a group carries the edit to the first row of the
/// next, in the order shown (`advance_rename`).
#[test]
fn tab_carries_a_rename_over_a_group_boundary_in_the_order_shown() {
    let name = "tab_carries_a_rename_over_a_group_boundary_in_the_order_shown";
    hosted_with(MODULE, "list", grouped(), name, |mut host| {
        let folder = mixed();
        let listed = open(&mut host, LIST, folder.path());
        let names: Vec<String> = listed.names().into_iter().map(str::to_owned).collect();
        let at = boundary(&names.iter().map(String::as_str).collect::<Vec<_>>());
        press(&mut host, Key::Home, Mods::default());
        for _ in 1..at {
            press(&mut host, Key::Down, Mods::default());
        }
        press(&mut host, Key::Function(2), Mods::default());
        let before = names[at - 1].clone();
        let editing = host.run_until(PATIENCE, |host| {
            field(host).is_some_and(|(text, focused)| focused && text == before)
        });
        assert!(editing, "the field: {:?}", field(&host));

        press(&mut host, Key::Tab, Mods::default());

        let next = names[at].clone();
        let moved = host.run_until(PATIENCE, |host| {
            field(host).is_some_and(|(text, focused)| focused && text == next)
        });
        assert!(moved, "the field after Tab: {:?}", field(&host));
        assert_eq!(browser_holds(&host).0, [next]);
    });
}

mod saved;
