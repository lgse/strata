// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy)]
enum LeftSelection<'a> {
    /// The cursor the folder's load placed on its first folder, `art`.
    LoadCursor,
    Cleared,
    Names(&'a [&'a str]),
}

struct ReturningSelection<'a> {
    case: &'a str,
    selected: LeftSelection<'a>,
    opens: Option<&'a str>,
    kept: &'a [&'a str],
}

const OPEN_PATH: &[&str] = &["/fx/docs", "/fx/docs/inner"];

fn returning_selections() -> [ReturningSelection<'static>; 6] {
    let case = |case, selected, opens, kept| ReturningSelection {
        case,
        selected,
        opens,
        kept,
    };
    [
        case(
            "load cursor on a folder",
            LeftSelection::LoadCursor,
            None,
            &[],
        ),
        case("empty selection", LeftSelection::Cleared, None, &[]),
        case(
            "open child still selected",
            LeftSelection::Names(&["docs"]),
            None,
            OPEN_PATH,
        ),
        case(
            "open child among several",
            LeftSelection::Names(&["docs", "pics"]),
            None,
            OPEN_PATH,
        ),
        case(
            "several without the open child",
            LeftSelection::Names(&["art", "pics"]),
            None,
            &[],
        ),
        case(
            "another folder selected",
            LeftSelection::Names(&["pics"]),
            Some("/fx/pics"),
            &[],
        ),
    ]
}

/// `/fx` with `docs` and its `inner` open as child columns and `/fx` active.
fn columns_with_open_child() -> (Rc<Browser>, Rc<FolderTree>) {
    let (browser, _, tree) = folder_browser(FolderTree::with(&[
        ("/fx", &["art", "docs", "pics"]),
        ("/fx/docs", &["inner"]),
    ]));
    let loaded = |depth: usize| {
        browser
            .column_snapshot(depth)
            .is_some_and(|snapshot| !snapshot.loading)
    };
    browser.navigate(Location::local("/fx"));
    pump_until(|| loaded(0));
    browser.show_child(0, Location::local("/fx/docs"));
    pump_until(|| loaded(1));
    browser.show_child(1, Location::local("/fx/docs/inner"));
    pump_until(|| loaded(2));
    browser.set_active_column(0);
    (browser, tree)
}

fn open_children(browser: &Browser) -> Vec<Location> {
    (1..)
        .map_while(|depth| browser.location_at(depth))
        .collect()
}

#[test]
fn returning_columns_reopen_only_the_child_the_selection_holds() {
    for ReturningSelection {
        case,
        selected,
        opens,
        kept,
    } in returning_selections()
    {
        let (browser, _) = columns_with_open_child();
        match selected {
            LeftSelection::LoadCursor => {
                assert!(browser.selection_is_load_cursor(), "{case}");
            }
            LeftSelection::Cleared => assert!(browser.clear_active_selection(), "{case}"),
            LeftSelection::Names(names) => {
                let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
                assert!(browser.select_entries_by_name_at(0, &names), "{case}");
            }
        }
        let history = browser.back_locations().len();

        let reopened = browser.follow_selection_on_return();

        assert_eq!(
            reopened,
            opens.map(|path| (0, Location::local(path))),
            "{case}"
        );
        let kept: Vec<_> = kept.iter().map(|path| Location::local(*path)).collect();
        assert_eq!(open_children(&browser), kept, "{case}");
        assert_eq!(
            browser.active_depth(),
            Some(0),
            "{case}: the parent stays active"
        );
        assert_eq!(
            browser.back_locations().len(),
            history,
            "{case}: closing the stale child adds no history"
        );
    }
}

#[test]
fn returning_columns_leave_a_validating_navigation_alone() {
    let (browser, tree) = columns_with_open_child();
    assert!(browser.clear_active_selection());
    tree.hold_validation.set(true);
    let share = tree_location("smb://host/share");
    browser.navigate_location(share.clone(), true);
    assert!(browser.pending_navigation_generation().is_some());

    assert_eq!(browser.follow_selection_on_return(), None);

    assert_eq!(
        open_children(&browser),
        OPEN_PATH
            .iter()
            .map(|path| Location::local(*path))
            .collect::<Vec<_>>(),
        "the stale child waits for the navigation"
    );
    tree.answer_validation(&share);
    assert_eq!(
        browser.active_location(),
        Some(share),
        "the validated navigation still opens"
    );
}

#[test]
fn returning_columns_wait_for_a_reloading_column_to_restore_its_selection() {
    let (browser, tree) = columns_with_open_child();
    assert!(browser.select_entries_by_name_at(0, &["docs".to_owned()]));
    tree.held.set(true);
    browser.reload_active();
    let loading = || {
        browser
            .column_snapshot(0)
            .is_some_and(|snapshot| snapshot.loading)
    };
    assert!(loading());

    assert_eq!(browser.follow_selection_on_return(), None);

    tree.release();
    pump_until(|| !loading());
    let selected = browser.selected_positions(0);
    assert_eq!(
        selected
            .iter()
            .filter_map(|position| browser.entry_at(0, *position))
            .map(|entry| entry.location)
            .collect::<Vec<_>>(),
        vec![Location::local("/fx/docs")],
    );
    assert_eq!(
        open_children(&browser),
        OPEN_PATH
            .iter()
            .map(|path| Location::local(*path))
            .collect::<Vec<_>>(),
        "the selected child stays open through the reload"
    );
}
