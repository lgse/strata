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

fn return_to(fixture: &KeyboardFixture, location: &Location) {
    let browser = fixture.view.browser();
    if browser.active_location().as_ref() != Some(location) {
        browser.navigate(location.clone());
        wait_until(|| browser.active_location().as_ref() == Some(location));
    }
    wait_loaded(&browser, 0);
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
