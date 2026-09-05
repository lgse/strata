// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::browser_modes::BrowserMode;
use std::{
    process::Command,
    time::{Duration, Instant},
};

pub(super) fn settle() {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn key(key: &str) {
    let tool = std::env::var_os("STRATA_TEST_XDOTOOL").unwrap_or_else(|| "xdotool".into());
    let result = Command::new(tool)
        .args(["key", "--clearmodifiers", key])
        .output()
        .expect("xdotool (or STRATA_TEST_XDOTOOL) is required");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    settle();
}

pub(super) fn focus_window() {
    let tool = std::env::var_os("STRATA_TEST_XDOTOOL").unwrap_or_else(|| "xdotool".into());
    assert!(
        Command::new(tool)
            .args([
                "search",
                "--onlyvisible",
                "--name",
                "^Strata keyboard regression$",
                "windowfocus",
                "--sync"
            ])
            .status()
            .expect("focus chooser")
            .success()
    );
    settle();
}

pub(super) fn focused(state: &ChooserState) -> gtk::Widget {
    gtk::prelude::RootExt::focus(&state.window).expect("keyboard focus")
}

pub(super) fn selected(state: &ChooserState) -> String {
    state
        .view
        .browser()
        .focused_entry()
        .expect("focused entry")
        .display_name
}

fn collections(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    if !widget.is_mapped() {
        return Vec::new();
    }
    if widget.is::<gtk::GridView>() || widget.is::<gtk::ListView>() {
        return vec![widget.clone()];
    }
    let mut result = Vec::new();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.extend(collections(&widget));
    }
    result
}

fn sidebar_round_trip(state: &ChooserState) {
    let before = selected(state);
    key("Left");
    assert!(
        !state.view.item_view_has_focus(),
        "Left from the outer file edge reaches the sidebar"
    );
    key("Up");
    assert!(
        focused(state).has_css_class("sidebar-toggle"),
        "Up from Home reaches the top-bar toggle"
    );
    key("Right");
    assert!(
        focused(state)
            .ancestor(gtk::HeaderBar::static_type())
            .is_some(),
        "Right continues across the top bar"
    );
    key("Left");
    assert!(focused(state).has_css_class("sidebar-toggle"));
    key("Down");
    assert!(!focused(state).has_css_class("sidebar-toggle"));
    key("Right");
    assert!(
        state.view.item_view_has_focus(),
        "Right restores file focus"
    );
    assert_eq!(
        selected(state),
        before,
        "focus transitions preserve file selection"
    );
}

fn save_modal(mode: BrowserMode, root: &Path) {
    let request = ChooserRequest {
        token: format!("save-keyboard-{mode:?}"),
        title: "Strata keyboard regression".into(),
        accept_label: "Save".into(),
        modal: false,
        parent: None,
        initial_directory: root.into(),
        kind: ChooserKind::SaveFile {
            current_name: Some("00.txt".into()),
        },
        filters: Vec::new(),
        current_filter: None,
        choices: Vec::new(),
    };
    ThemeManager::shared().set_browser_mode(mode);
    let state =
        build_chooser(request, Arc::new(AtomicBool::new(false)), |_| {}).expect("save chooser");
    state.view.set_view_mode(mode);
    settle();
    focus_window();
    let filename = state.filename.as_ref().expect("filename");
    assert!(
        focused(&state).is_ancestor(filename),
        "Save initially focuses its filename"
    );
    assert!(
        filename.selection_bounds().is_some(),
        "the suggested filename starts selected"
    );
    gtk::prelude::GtkWindowExt::set_focus(&state.window, None::<&gtk::Widget>);
    key("Right");
    assert!(
        focused(&state).is_ancestor(filename),
        "an arrow restores missing Save focus to the filename"
    );
    key("ctrl+a");
    assert!(filename.selection_bounds().is_some());
    key("Left");
    assert_eq!(filename.position(), 0);
    key("Right");
    assert_eq!(filename.position(), 1);
    assert_eq!(filename.text(), "00.txt");
    key("Return");
    let deadline = Instant::now() + Duration::from_secs(5);
    while visible_modal_layer(&state.window).is_none() {
        settle();
        assert!(Instant::now() < deadline, "overwrite confirmation appears");
    }
    assert!(focused(&state).has_css_class("action-dialog-cancel"));
    key("Right");
    assert!(focused(&state).has_css_class("action-dialog-confirm"));
    key("Left");
    key("Return");
    let deadline = Instant::now() + Duration::from_secs(5);
    while visible_modal_layer(&state.window).is_some() {
        settle();
        assert!(
            Instant::now() < deadline,
            "Enter activates Cancel, not the default confirmation"
        );
    }
    assert!(state.completion.borrow().is_some());
    assert_eq!(
        std::fs::read_to_string(root.join("00.txt")).expect("original file"),
        "text"
    );
    key("Escape");
    assert!(state.completion.borrow().is_none());
    settle();
}

#[test]
#[ignore = "requires X11, xdotool, and isolated XDG directories; run this test alone"]
fn keyboard_only_controls_and_file_navigation_work_in_every_chooser_view() {
    gtk::init().expect("GTK display");
    crate::ui::prepare_portal_ui();
    let root = tempfile::tempdir().expect("files");
    for index in 0..20 {
        std::fs::write(root.path().join(format!("{index:02}.txt")), "text").expect("text file");
    }
    for index in 0..2 {
        std::fs::write(root.path().join(format!("code-{index}.json")), "{}").expect("JSON file");
    }
    for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
        for grouped in [false, true] {
            let request = ChooserRequest {
                token: format!("keyboard-{mode:?}-{grouped}"),
                title: "Strata keyboard regression".into(),
                accept_label: "Open".into(),
                modal: false,
                parent: None,
                initial_directory: root.path().into(),
                kind: ChooserKind::Open {
                    directory: false,
                    multiple: true,
                },
                filters: vec![FileFilter::new("All files").glob("*")],
                current_filter: None,
                choices: vec![
                    Choice::new("encoding", "Encoding", "utf8")
                        .insert("utf8", "UTF-8")
                        .insert("latin1", "Latin-1"),
                    Choice::boolean("compress", "Compress files", false),
                ],
            };
            ThemeManager::shared().set_browser_mode(mode);
            ThemeManager::shared().set_group_by_type(grouped);
            let state =
                build_chooser(request, Arc::new(AtomicBool::new(false)), |_| {}).expect("chooser");
            state.view.set_view_mode(mode);
            state.view.set_group_by_type(grouped);
            settle();
            focus_window();
            assert!(
                focused(&state).has_css_class("sidebar-toggle") || state.view.item_view_has_focus(),
                "Open starts with a focused control or file: {mode:?}, focus={:?}",
                focused(&state)
            );
            assert!(
                state.window.gets_focus_visible(),
                "startup focus is visible"
            );
            for arrow in ["Left", "Right", "Up", "Down"] {
                gtk::prelude::GtkWindowExt::set_focus(&state.window, None::<&gtk::Widget>);
                state.window.set_focus_visible(false);
                key(arrow);
                assert!(
                    focused(&state).has_css_class("sidebar-toggle"),
                    "{arrow} establishes missing focus"
                );
                assert!(state.window.gets_focus_visible());
            }
            let browser = state.view.browser();
            let deadline = Instant::now() + Duration::from_secs(5);
            while browser.entry_at(0, 21).is_none() {
                settle();
                assert!(Instant::now() < deadline, "files load: {mode:?}");
            }
            browser.set_sort(
                0,
                crate::model::SortKey::Name,
                crate::model::SortDirection::Ascending,
            );
            settle();
            browser.select(0, 0);
            browser.focus_active();
            settle();
            if grouped && mode != BrowserMode::Columns {
                let first = collections(&state.view.widget())
                    .into_iter()
                    .find(|widget| {
                        if let Some(grid) = widget.downcast_ref::<gtk::GridView>() {
                            grid.model().is_some_and(|model| model.n_items() > 0)
                        } else if let Some(list) = widget.downcast_ref::<gtk::ListView>() {
                            list.model().is_some_and(|model| model.n_items() > 0)
                        } else {
                            false
                        }
                    })
                    .expect("first visible collection");
                first.grab_focus();
                if let Some(grid) = first.downcast_ref::<gtk::GridView>() {
                    grid.model().expect("model").select_item(0, true);
                    grid.scroll_to(0, gtk::ListScrollFlags::FOCUS, None);
                } else if let Some(list) = first.downcast_ref::<gtk::ListView>() {
                    list.model().expect("model").select_item(0, true);
                    list.scroll_to(0, gtk::ListScrollFlags::FOCUS, None);
                }
                settle();
                assert!(
                    focused(&state).is_ancestor(&first),
                    "first visual group takes focus"
                );
            }
            let retained = browser.selected_entries();
            let tool = std::env::var_os("STRATA_TEST_XDOTOOL").unwrap_or_else(|| "xdotool".into());
            for (x, y) in [(500, 300), (510, 310)] {
                assert!(
                    Command::new(&tool)
                        .args([
                            "search",
                            "--onlyvisible",
                            "--name",
                            "^Strata keyboard regression$",
                            "mousemove",
                            "--window",
                            "%1",
                            &x.to_string(),
                            &y.to_string(),
                        ])
                        .status()
                        .expect("pointer movement")
                        .success()
                );
                settle();
            }
            assert!(
                !state.view.widget().has_css_class("keyboard-navigation"),
                "pointer input uses shared pointer styling"
            );
            assert_eq!(
                browser.selected_entries(),
                retained,
                "pointer movement preserves selection fills"
            );
            key("Down");
            assert!(
                state.view.widget().has_css_class("keyboard-navigation"),
                "arrow navigation restores the shared keyboard cursor styling"
            );
            key("Up");
            let before_toolbar = selected(&state);
            key("Up");
            assert!(
                state.view.header_actions_have_focus(),
                "{mode:?}, grouped={grouped}: Up reaches toolbar (focus={:?}, before={before_toolbar}, after={})",
                focused(&state),
                selected(&state)
            );
            assert!(
                focused(&state).has_css_class("chooser-new-folder"),
                "first toolbar action"
            );
            key("Right");
            assert!(
                focused(&state)
                    .tooltip_text()
                    .is_some_and(|text| text.contains("Refresh")),
                "Right reaches Refresh"
            );
            for _ in 0..8 {
                if focused(&state)
                    .tooltip_text()
                    .is_some_and(|text| text.starts_with("Filter"))
                {
                    break;
                }
                key("Right");
            }
            assert!(
                focused(&state)
                    .tooltip_text()
                    .is_some_and(|text| text.starts_with("Filter")),
                "arrows reach the Filter icon"
            );
            key("space");
            let deadline = Instant::now() + Duration::from_secs(5);
            while !state.view.filter_has_focus() {
                settle();
                assert!(
                    Instant::now() < deadline,
                    "Space on Filter opens its text field"
                );
            }
            key("Left");
            assert!(
                state.view.filter_has_focus(),
                "filter cursor keys stay in the entry"
            );
            key("Escape");
            assert!(!state.view.filter_has_focus());
            key("Up");
            key("Return");
            assert!(
                state.view.new_entry_is_active(),
                "Enter on New Folder does not submit the chooser"
            );
            key("Escape");
            assert!(!state.view.new_entry_is_active());
            assert!(state.completion.borrow().is_some());
            browser.select(0, 0);
            browser.focus_active();
            settle();
            sidebar_round_trip(&state);
            if mode == BrowserMode::Icons {
                key("Right");
                assert_eq!(
                    selected(&state),
                    "01.txt",
                    "Icons Right moves a cell, not into a folder"
                );
                key("Left");
                assert!(
                    state.view.item_view_has_focus(),
                    "Left inside an Icons row stays in the view"
                );
                assert_eq!(selected(&state), "00.txt");
                key("shift+Left");
                assert!(
                    state.view.item_view_has_focus(),
                    "Shift+Left never moves focus to the sidebar"
                );
                browser.select(0, 0);
                browser.focus_active();
                settle();
                key("Down");
                sidebar_round_trip(&state);
                key("ctrl+b");
                key("Left");
                assert!(
                    state.view.item_view_has_focus(),
                    "Left cannot focus a hidden sidebar"
                );
                key("ctrl+b");
                browser.select(0, 1);
                browser.focus_active();
                settle();
                let grid = focused(&state)
                    .ancestor(gtk::GridView::static_type())
                    .and_downcast::<gtk::GridView>()
                    .expect("focused grid");
                let before = focused(&state).compute_bounds(&grid).expect("cell bounds");
                key("Down");
                let after = focused(&state)
                    .compute_bounds(&grid)
                    .expect("next row bounds");
                assert!(
                    (before.x() - after.x()).abs() < 1.0 && after.y() > before.y(),
                    "Icons Down keeps its visual column"
                );
                key("Up");
                assert_eq!(selected(&state), "01.txt");
            } else {
                key("Down");
                assert_eq!(selected(&state), "01.txt");
                sidebar_round_trip(&state);
            }
            let anchor = selected(&state);
            key("shift+Down");
            assert!(
                browser.selected_entries().len() > 1,
                "Shift+Down extends selection: {mode:?}"
            );
            key("shift+Up");
            assert_eq!(
                selected(&state),
                anchor,
                "Shift+Up restores the focused range endpoint"
            );

            if mode == BrowserMode::Icons && grouped {
                let grids = collections(&state.view.widget())
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::GridView>().ok())
                    .filter(|grid| grid.model().is_some_and(|model| model.n_items() > 0))
                    .collect::<Vec<_>>();
                assert!(grids.len() > 1);
                let first = &grids[0];
                let model = first.model().expect("group selection");
                for grid in &grids {
                    grid.model().expect("selection").unselect_all();
                }
                let last = model.n_items() - 1;
                model.select_item(last, true);
                first.grab_focus();
                first.scroll_to(last, gtk::ListScrollFlags::FOCUS, None);
                settle();
                let previous = selected(&state);
                key("Down");
                assert_ne!(
                    selected(&state),
                    previous,
                    "Down crosses type-group boundaries"
                );
                assert_eq!(
                    browser.selected_entries().len(),
                    1,
                    "plain arrows collapse selections across groups"
                );
                key("shift+Up");
                assert!(
                    browser.selected_entries().len() > 1,
                    "Shift extends selection across groups: selected={}, sizes={:?}",
                    selected(&state),
                    grids
                        .iter()
                        .map(|grid| grid.model().expect("selection").selection().size())
                        .collect::<Vec<_>>()
                );
            }

            let filter = state.filter_dropdown.as_ref().expect("filter");
            filter.button.grab_focus();
            key("Right");
            let ChoiceControl::Select { dropdown, .. } = &state.choices[0] else {
                panic!("encoding")
            };
            assert!(focused(&state).is_ancestor(&dropdown.button));
            key("Down");
            assert!(
                dropdown.popover.is_mapped(),
                "Down opens the focused dropdown"
            );
            key("Down");
            key("Return");
            assert_eq!(state.choices[0].value().1, "latin1");
            key("Right");
            key("Return");
            assert_eq!(
                state.choices[1].value().1,
                "true",
                "Enter toggles without submitting"
            );
            key("space");
            assert_eq!(state.choices[1].value().1, "false");
            key("Right");
            key("space");
            assert!(
                state
                    .read_only
                    .as_ref()
                    .expect("read-only choice")
                    .is_active()
            );
            key("Left");
            key("shift+Tab");
            assert!(
                focused(&state).is_ancestor(&dropdown.button),
                "Shift+Tab reaches the previous option"
            );
            key("Tab");
            key("Up");
            assert!(
                state.view.item_view_has_focus(),
                "Up returns from options to files"
            );
            assert!(state.completion.borrow().is_some());
            key("ctrl+shift+b");
            key("Down");
            assert!(!state.view.item_view_has_focus());
            key("Right");
            assert!(
                state.view.item_view_has_focus(),
                "Right returns from sidebar to files"
            );
            key("ctrl+l");
            assert!(state.view.location_has_focus());
            key("Left");
            assert!(state.view.location_has_focus(), "Left edits the path");
            key("Escape");
            assert!(state.completion.borrow().is_some());
            state.cancel();
            settle();
        }
        save_modal(mode, root.path());
    }
}
