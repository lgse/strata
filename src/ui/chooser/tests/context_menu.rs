// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    keyboard::{focus_window, focused, key, selected, settle},
    *,
};
use crate::ui::browser_modes::BrowserMode;
use std::{
    process::Command,
    time::{Duration, Instant},
};

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        settle();
        assert!(Instant::now() < deadline, "UI operation completes");
    }
    settle();
}

fn find_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if !widget.is_mapped() {
        return None;
    }
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = find_class(&widget, class) {
            return Some(found);
        }
    }
    None
}

fn menu_buttons(state: &ChooserState) -> Vec<gtk::Button> {
    let menu = find_class(state.window.upcast_ref(), "chooser-context-menu").expect("chooser menu");
    let mut buttons = Vec::new();
    let mut child = menu.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(button) = widget.downcast::<gtk::Button>() {
            buttons.push(button);
        }
    }
    buttons
}

fn right_click(state: &ChooserState, empty: bool) {
    let bounds = if empty {
        state
            .view
            .widget()
            .compute_bounds(&state.window)
            .expect("browser bounds")
    } else {
        focused(state)
            .compute_bounds(&state.window)
            .expect("item bounds")
    };
    let (x, y) = if empty {
        (bounds.x() + 100.0, bounds.y() + bounds.height() - 40.0)
    } else {
        (bounds.center().x(), bounds.center().y())
    };
    let tool = std::env::var_os("STRATA_TEST_XDOTOOL").unwrap_or_else(|| "xdotool".into());
    assert!(
        Command::new(tool)
            .args([
                "search",
                "--onlyvisible",
                "--name",
                "^Strata keyboard regression$",
                "mousemove",
                "--window",
                "%1",
                &format!("{x:.0}"),
                &format!("{y:.0}"),
                "click",
                "3",
            ])
            .status()
            .expect("right click")
            .success()
    );
    settle();
}

fn edit_name(state: &ChooserState, name: &str) {
    let entry = focused(state)
        .ancestor(gtk::Entry::static_type())
        .and_downcast::<gtk::Entry>()
        .unwrap_or_else(|| {
            panic!(
                "inline name field for {name}: focus={:?}, parent={:?}, mode={:?}",
                focused(state),
                focused(state).parent(),
                state.view.view_mode()
            )
        });
    entry.set_text(name);
}

#[test]
#[ignore = "requires X11, xdotool, and isolated XDG directories; run this test alone"]
fn chooser_context_menus_and_rename_work_in_every_view() {
    gtk::init().expect("GTK display");
    crate::ui::prepare_portal_ui();
    for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
        for grouped in [false, true] {
            let root = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(root.path().join("Folder")).expect("folder");
            std::fs::write(root.path().join("note.txt"), "notes").expect("file");
            ThemeManager::shared().set_browser_mode(mode);
            ThemeManager::shared().set_group_by_type(grouped);
            let request = ChooserRequest {
                token: format!("context-{mode:?}-{grouped}"),
                title: "Strata keyboard regression".into(),
                accept_label: "Open".into(),
                modal: false,
                parent: None,
                initial_directory: root.path().into(),
                kind: ChooserKind::Open {
                    directory: false,
                    multiple: true,
                },
                filters: vec![],
                current_filter: None,
                choices: vec![],
            };
            let state =
                build_chooser(request, Arc::new(AtomicBool::new(false)), |_| {}).expect("chooser");
            let browser = state.view.browser();
            wait_until(|| browser.entry_at(0, 1).is_some());
            focus_window();
            browser.set_sort(
                0,
                crate::model::SortKey::Name,
                crate::model::SortDirection::Ascending,
            );
            browser.select(0, 0);
            browser.focus_active();
            settle();
            assert_eq!(selected(&state), "Folder");
            key("F2");
            assert!(state.view.rename_is_active());
            edit_name(&state, "Cancelled");
            key("Escape");
            assert!(!state.view.rename_is_active());
            assert!(root.path().join("Folder").is_dir());
            assert!(!root.path().join("Cancelled").exists());
            assert!(state.completion.borrow().is_some());
            assert_eq!(selected(&state), "Folder");

            browser.focus_active();
            settle();
            let focus = focused(&state);
            if let Some(view) = focus
                .ancestor(gtk::ListView::static_type())
                .and_downcast::<gtk::ListView>()
            {
                view.model().expect("list selection").unselect_all();
            } else {
                focus
                    .ancestor(gtk::GridView::static_type())
                    .and_downcast::<gtk::GridView>()
                    .expect("grid")
                    .model()
                    .expect("grid selection")
                    .unselect_all();
            }
            settle();
            assert!(browser.selected_entries().is_empty());
            right_click(&state, false);
            assert!(
                crate::ui::focus_navigation::in_popover(&focused(&state)),
                "newly selected item keeps menu focus"
            );
            let buttons = menu_buttons(&state);
            assert_eq!(buttons.len(), 2);
            buttons[1].emit_clicked();
            wait_until(|| visible_modal_layer(&state.window).is_some());
            key("Escape");
            wait_until(|| visible_modal_layer(&state.window).is_none());
            assert_eq!(selected(&state), "Folder");
            assert!(state.completion.borrow().is_some());

            browser.focus_active();
            settle();
            right_click(&state, false);
            menu_buttons(&state)[0].emit_clicked();
            wait_until(|| state.view.rename_is_active());
            edit_name(&state, "Renamed");
            key("Return");
            wait_until(|| root.path().join("Renamed").is_dir() && !state.view.rename_is_active());
            assert!(!root.path().join("Folder").exists());
            assert!(state.completion.borrow().is_some());

            right_click(&state, true);
            let buttons = menu_buttons(&state);
            assert_eq!(buttons.len(), 1);
            buttons[0].emit_clicked();
            wait_until(|| state.view.new_entry_is_active());
            edit_name(&state, "Cancelled folder");
            key("Escape");
            assert!(!root.path().join("Cancelled folder").exists());
            assert!(state.completion.borrow().is_some());
            assert_eq!(selected(&state), "Renamed");

            browser.select(0, 1);
            browser.focus_active();
            settle();
            assert_eq!(selected(&state), "note.txt");
            key("alt+Return");
            wait_until(|| visible_modal_layer(&state.window).is_some());
            key("Escape");
            wait_until(|| visible_modal_layer(&state.window).is_none());
            assert_eq!(selected(&state), "note.txt");
            browser.focus_active();
            settle();
            key("F2");
            edit_name(&state, "renamed.txt");
            key("Return");
            wait_until(|| {
                root.path().join("renamed.txt").is_file() && !state.view.rename_is_active()
            });
            assert_eq!(
                std::fs::read_to_string(root.path().join("renamed.txt")).expect("renamed contents"),
                "notes"
            );
            assert!(!root.path().join("note.txt").exists());
            browser.focus_active();
            settle();
            key("ctrl+a");
            let before = browser.selected_entries();
            assert_eq!(before.len(), 2);
            key("F2");
            assert!(!state.view.rename_is_active());
            right_click(&state, false);
            let buttons = menu_buttons(&state);
            assert_eq!(buttons.len(), 2);
            assert!(!buttons[0].is_sensitive());
            buttons[1].emit_clicked();
            wait_until(|| visible_modal_layer(&state.window).is_some());
            key("Escape");
            wait_until(|| visible_modal_layer(&state.window).is_none());
            assert_eq!(browser.selected_entries(), before);
            assert!(state.completion.borrow().is_some());
            state.cancel();
            settle();
        }
    }
}
