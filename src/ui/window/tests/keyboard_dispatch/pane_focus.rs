// SPDX-License-Identifier: MIT

use super::*;

const MODES: [BrowserMode; 3] = [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons];

/// A real Tab press reaches the dispatcher first and GTK's `move-focus` binding only when
/// the dispatcher lets it through; `KeyboardFixture::press` emits just the former.
fn tab(fixture: &KeyboardFixture, direction: gtk::DirectionType) {
    let (key, modifiers) = if direction == gtk::DirectionType::TabBackward {
        (Key::ISO_Left_Tab, ModifierType::SHIFT_MASK)
    } else {
        (Key::Tab, ModifierType::empty())
    };
    if !fixture.press(key, modifiers) {
        fixture.window.emit_move_focus(direction);
    }
    pump(50);
}

fn settles(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !condition() {
        if Instant::now() >= deadline {
            return false;
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    true
}

fn focus(fixture: &KeyboardFixture) -> Option<gtk::Widget> {
    gtk::prelude::RootExt::focus(&fixture.window)
}

fn focus_shows(fixture: &KeyboardFixture, name: &str) -> bool {
    focus(fixture).is_some_and(|focused| {
        !focused.is::<gtk::ListView>()
            && !focused.is::<gtk::GridView>()
            && rendered_name(&focused, name)
    })
}

fn describe_focus(fixture: &KeyboardFixture) -> String {
    match focus(fixture) {
        None => "nothing".to_owned(),
        Some(focused) => format!(
            "{} {:?} (mapped: {})",
            focused.type_().name(),
            focused.css_classes(),
            focused.is_mapped()
        ),
    }
}

fn footer_button(fixture: &KeyboardFixture) -> gtk::Widget {
    let footer = widget_with_class(fixture.window.upcast_ref(), "shortcut-footer-button")
        .expect("shortcuts button");
    wait_until(|| footer.is_mapped());
    footer
}

fn add_subfolder(fixture: &KeyboardFixture) {
    let sub = fixture._directory.path().join("sub");
    std::fs::create_dir(&sub).expect("subfolder");
    std::fs::write(sub.join("inner.txt"), b"inner").expect("subfolder file");
    let browser = fixture.view.browser();
    fixture.view.refresh();
    wait_until(|| entry_count(&browser) == 4);
}

fn return_to(fixture: &KeyboardFixture, location: &Location) {
    let browser = fixture.view.browser();
    if browser.active_location().as_ref() != Some(location) {
        browser.navigate(location.clone());
        wait_until(|| browser.active_location().as_ref() == Some(location));
    }
    wait_loaded(&browser, 0);
}

fn mapped_with_class(widget: &gtk::Widget, class: &str) -> bool {
    if widget.has_css_class(class) && widget.is_mapped() {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if mapped_with_class(&widget, class) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

/// Records whether the pane surface took focus while its pane showed neither a status
/// nor a loading page: a fast load must keep focus on the view.
struct SurfaceFocusWatch {
    early: Rc<Cell<bool>>,
    handler: glib::SignalHandlerId,
}

impl SurfaceFocusWatch {
    fn install(fixture: &KeyboardFixture) -> Self {
        let early = Rc::new(Cell::new(false));
        let flag = early.clone();
        let handler = fixture.window.connect_focus_widget_notify(move |window| {
            let page = gtk::prelude::RootExt::focus(window)
                .filter(|focused| focused.has_css_class("directory-surface"))
                .and_downcast::<gtk::Stack>()
                .and_then(|stack| stack.visible_child_name());
            if page
                .as_deref()
                .is_some_and(|page| matches!(page, "pending" | "content"))
            {
                flag.set(true);
            }
        });
        Self { early, handler }
    }

    fn finish(self, fixture: &KeyboardFixture) -> bool {
        fixture.window.disconnect(self.handler);
        self.early.get()
    }
}

fn focused_surface(fixture: &KeyboardFixture) -> Option<gtk::Stack> {
    let mut widget = focus(fixture);
    while let Some(current) = widget {
        if current.has_css_class("directory-surface") {
            return current.downcast().ok();
        }
        widget = current.parent();
    }
    None
}

fn focused_surface_is_focusable(fixture: &KeyboardFixture) -> bool {
    focused_surface(fixture).is_some_and(|surface| surface.is_focusable())
}

#[test]
fn default_map_tab_leaves_the_listing_in_one_press() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::pane_focus::default_map_tab_leaves_the_listing_in_one_press",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let home = Location::local(fixture._directory.path());
            add_subfolder(&fixture);
            let footer = footer_button(&fixture);
            for mode in MODES {
                fixture.view.set_view_mode(mode);
                return_to(&fixture, &home);
                select_named(&fixture, "sub");
                wait_until(|| focus_shows(&fixture, "sub"));

                tab(&fixture, gtk::DirectionType::TabForward);
                assert!(
                    !fixture.view.item_view_has_focus()
                        && focus(&fixture).as_ref() == Some(&footer),
                    "{mode:?}: Tab from the cursor row moved focus to {} instead of the footer",
                    describe_focus(&fixture)
                );

                let cursor = focused_index(&browser);
                tab(&fixture, gtk::DirectionType::TabBackward);
                assert!(
                    settles(|| fixture.view.item_view_has_focus() && focus_shows(&fixture, "sub"))
                        && focused_index(&browser) == cursor,
                    "{mode:?}: Shift+Tab from the footer landed on {} instead of the cursor row",
                    describe_focus(&fixture)
                );

                let watch = SurfaceFocusWatch::install(&fixture);
                fixture.press(Key::Return, ModifierType::empty());
                assert!(
                    settles(|| location_ends_with(browser.active_location(), "sub")),
                    "{mode:?}: Enter after Shift+Tab did not open the row that showed focus"
                );
                wait_loaded(&browser, browser.active_depth().unwrap_or(0));
                pump(50);
                assert!(
                    !watch.finish(&fixture),
                    "{mode:?}: opening a folder focused its pane surface without a loading page"
                );

                return_to(&fixture, &home);
                select_named(&fixture, "a.txt");
                // GTK's Tab order skips widgets that the rebuilt pane has not laid out yet.
                wait_until(|| {
                    focus(&fixture).is_some_and(|row| row.is_mapped() && row.width() > 0)
                });
                tab(&fixture, gtk::DirectionType::TabBackward);
                let left = !fixture.view.item_view_has_focus()
                    && match mode {
                        BrowserMode::List => focus(&fixture).is_some_and(|focused| {
                            focused.is::<gtk::Button>()
                                && focused.has_css_class("list-heading-button")
                        }),
                        BrowserMode::Icons => fixture.view.header_actions_have_focus(),
                        BrowserMode::Columns
                            if mapped_with_class(
                                fixture.window.upcast_ref(),
                                "column-header-action",
                            ) =>
                        {
                            fixture.view.header_actions_have_focus()
                        }
                        BrowserMode::Columns => !file_panes_have_focus(&fixture),
                    };
                assert!(
                    left,
                    "{mode:?}: Shift+Tab from a row moved focus to {} instead of the control before the rows",
                    describe_focus(&fixture)
                );
            }
        },
    );
}

#[test]
fn columns_tab_from_outside_lands_on_the_active_column() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::pane_focus::columns_tab_from_outside_lands_on_the_active_column",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            add_subfolder(&fixture);
            select_named(&fixture, "sub");
            fixture.press(Key::Right, ModifierType::empty());
            wait_loaded(&browser, 1);
            focus_files(&fixture);
            wait_until(|| browser.active_depth() == Some(1) && focus_shows(&fixture, "inner.txt"));

            assert!(
                fixture
                    .sidebar
                    .widget
                    .child_focus(gtk::DirectionType::TabForward)
            );
            assert!(sidebar_has_focus(&fixture));
            let mut presses = 0;
            while !file_panes_have_focus(&fixture) {
                assert!(presses < 40, "Tab never left the sidebar");
                tab(&fixture, gtk::DirectionType::TabForward);
                presses += 1;
            }
            pump(100);

            assert_eq!(
                browser.active_depth(),
                Some(1),
                "Tab into the strip changed the active column; focus is on {}",
                describe_focus(&fixture)
            );
            // The cursor takes focus from the landing row on idle, after pending paints.
            assert!(
                settles(|| focus_shows(&fixture, "inner.txt")),
                "Tab into the strip should land on the active column's cursor, not {}",
                describe_focus(&fixture)
            );

            // An open filter is the stop between the rows and the header actions, in
            // both directions.
            assert!(fixture.view.show_filter_with_query("inner"));
            // The entry maps once its revealer has run a frame.
            wait_until(|| {
                fixture.view.filter_has_focus() && focus(&fixture).is_some_and(|f| f.is_mapped())
            });
            // The filter's results replace the rows asynchronously; let them land first.
            super::footer_prompt::wait_results(&fixture, &["inner.txt"]);
            focus_files(&fixture);
            tab(&fixture, gtk::DirectionType::TabBackward);
            assert!(
                settles(|| fixture.view.filter_has_focus()),
                "Shift+Tab from the rows skipped the open filter for {}",
                describe_focus(&fixture)
            );
            tab(&fixture, gtk::DirectionType::TabBackward);
            assert!(
                settles(|| fixture.view.header_actions_have_focus()),
                "Shift+Tab from the filter moved focus to {}",
                describe_focus(&fixture)
            );
            tab(&fixture, gtk::DirectionType::TabForward);
            assert!(
                settles(|| fixture.view.filter_has_focus()),
                "Tab from the header actions moved focus to {}",
                describe_focus(&fixture)
            );
        },
    );
}

#[test]
fn empty_directory_focus_is_a_named_mapped_surface_and_tabs_out() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::pane_focus::empty_directory_focus_is_a_named_mapped_surface_and_tabs_out",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_arrow_navigation_scoped(false);
            let home = Location::local(fixture._directory.path());
            let footer = footer_button(&fixture);
            for mode in MODES {
                fixture.view.set_view_mode(mode);
                return_to(&fixture, &home);
                open_empty_folder(&fixture);

                let surface = focus(&fixture)
                    .filter(|focused| {
                        focused.is_mapped()
                            && focused.is::<gtk::Stack>()
                            && focused.has_css_class("directory-surface")
                    })
                    .filter(|_| fixture.view.item_view_has_focus())
                    .unwrap_or_else(|| {
                        panic!(
                            "{mode:?}: the empty directory left focus on {} instead of a mapped pane surface",
                            describe_focus(&fixture)
                        )
                    });

                tab(&fixture, gtk::DirectionType::TabForward);
                assert_eq!(
                    focus(&fixture).as_ref(),
                    Some(&footer),
                    "{mode:?}: Tab from the empty surface moved focus to {}",
                    describe_focus(&fixture)
                );
                tab(&fixture, gtk::DirectionType::TabBackward);
                assert_eq!(
                    focus(&fixture).as_ref(),
                    Some(&surface),
                    "{mode:?}: Shift+Tab from the footer moved focus to {}",
                    describe_focus(&fixture)
                );

                if mode == BrowserMode::List {
                    fixture.press(Key::Up, ModifierType::empty());
                    assert!(
                        settles(|| fixture.view.header_actions_have_focus()),
                        "{mode:?}: Up from the empty surface moved focus to {}",
                        describe_focus(&fixture)
                    );
                }

                return_to(&fixture, &home);
                focus_files(&fixture);
                let populated = ["empty", "a.txt", "b.txt", "c.txt"];
                assert!(
                    settles(|| populated.iter().any(|name| focus_shows(&fixture, name)))
                        && !focused_surface_is_focusable(&fixture),
                    "{mode:?}: returning to a populated folder left focus on {}",
                    describe_focus(&fixture)
                );
            }
        },
    );
}

#[test]
fn tab_inside_an_inline_rename_commits_and_leaves_the_list() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::pane_focus::tab_inside_an_inline_rename_commits_and_leaves_the_list",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            for (mode, original) in [
                (BrowserMode::List, "a.txt"),
                (BrowserMode::Columns, "b.txt"),
            ] {
                fixture.view.set_view_mode(mode);
                select_named(&fixture, original);
                wait_until(|| fixture.view.rename_is_active() || fixture.view.begin_rename());
                wait_until(|| fixture.view.active_rename_field().is_some());
                let field = fixture.view.active_rename_field().expect("rename field");
                let renamed = format!("renamed-{original}");
                field.set_text(&renamed);

                tab(&fixture, gtk::DirectionType::TabForward);
                wait_until(|| directory.join(&renamed).is_file());
                assert!(!directory.join(original).exists(), "{mode:?}");
                assert!(
                    !focus(&fixture)
                        .is_some_and(|focused| crate::ui::focus_navigation::editable(&focused)),
                    "{mode:?}: Tab left focus in an editor: {}",
                    describe_focus(&fixture)
                );
                wait_until(|| !fixture.view.rename_is_active());
            }
        },
    );
}

#[derive(Clone, Copy, Debug)]
enum RowKey {
    Rename,
    ContextMenu,
}

#[test]
fn row_keys_reach_the_row_after_a_reload_closes_a_columns_rename() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::pane_focus::row_keys_reach_the_row_after_a_reload_closes_a_columns_rename",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            for key in [RowKey::Rename, RowKey::ContextMenu] {
                select_named(&fixture, "a.txt");
                assert!(fixture.press(Key::F2, ModifierType::empty()));
                let field = fixture.view.active_rename_field().expect("rename field");
                wait_until(|| focus(&fixture).is_some_and(|focused| focused.is_ancestor(&field)));
                field.set_text("typed.txt");
                // F5 waits for the edit, but a rescan of the directory reloads the
                // column regardless and takes the field's row away.
                browser.refresh_all();
                wait_until(|| !fixture.view.rename_is_active());
                wait_loaded(&browser, 0);
                select_named(&fixture, "a.txt");
                wait_until(|| {
                    focus_shows(&fixture, "a.txt") && rendered_name(&fixture.view.widget(), "a.txt")
                });

                match key {
                    RowKey::Rename => {
                        assert!(
                            fixture.press(Key::F2, ModifierType::empty()),
                            "F2 after the reload did not start a rename"
                        );
                        let field = fixture.view.active_rename_field().expect("rename field");
                        wait_until(|| {
                            focus(&fixture).is_some_and(|focused| focused.is_ancestor(&field))
                        });
                        assert_eq!(field.text(), "a.txt");
                        assert!(fixture.press(Key::Escape, ModifierType::empty()));
                        wait_until(|| !fixture.view.rename_is_active());
                    }
                    RowKey::ContextMenu => {
                        assert!(
                            fixture.press(Key::Menu, ModifierType::empty()),
                            "Menu after the reload did not open the item menu"
                        );
                        wait_until(|| visible_menu(fixture.window.upcast_ref()).is_some());
                        visible_menu(fixture.window.upcast_ref())
                            .expect("item menu")
                            .popdown();
                        wait_until(|| visible_menu(fixture.window.upcast_ref()).is_none());
                    }
                }
            }
        },
    );
}

/// A reload hides the listing behind the grace page and, when slow, the loading page.
/// GTK then moves focus off the hidden rows after the next paint; it must park on the
/// pane surface and come back to the cursor row, never escape to the surrounding chrome.
#[test]
fn reloading_keeps_focus_on_the_pane_and_returns_it_to_the_cursor() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::pane_focus::reloading_keeps_focus_on_the_pane_and_returns_it_to_the_cursor",
        || {
            let fixture = KeyboardFixture::new();
            for mode in MODES {
                fixture.view.set_view_mode(mode);
                for pages in [
                    &["pending", "content"][..],
                    &["pending", "loading", "content"][..],
                ] {
                    select_named(&fixture, "b.txt");
                    wait_until(|| focus_shows(&fixture, "b.txt"));
                    let surface = focused_surface(&fixture).expect("pane surface");
                    for page in pages {
                        surface.set_visible_child_name(page);
                        if *page != "content" {
                            assert!(
                                settles(|| focus(&fixture).as_ref() == Some(surface.upcast_ref())),
                                "{mode:?} {pages:?}: the hidden listing lost focus to {}",
                                describe_focus(&fixture)
                            );
                        }
                    }
                    assert!(
                        settles(|| focus_shows(&fixture, "b.txt")),
                        "{mode:?} {pages:?}: focus did not return to the cursor row: {}",
                        describe_focus(&fixture)
                    );
                }
            }
        },
    );
}
