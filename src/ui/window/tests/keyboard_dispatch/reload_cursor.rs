// SPDX-License-Identifier: MIT

use super::*;

fn settles(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
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

/// The focused row or tile, not the bare collection view, renders `name`.
fn focus_shows(fixture: &KeyboardFixture, name: &str) -> bool {
    focus(fixture).is_some_and(|focused| {
        !focused.is::<gtk::ListView>()
            && !focused.is::<gtk::GridView>()
            && rendered_name(&focused, name)
    })
}

fn selected_names(browser: &crate::app::Browser) -> Vec<String> {
    browser
        .selected_entries()
        .into_iter()
        .map(|entry| entry.display_name)
        .collect()
}

fn report(failures: Vec<String>) {
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// How the return to the parent starts. Alt+Up in the listing, Alt+Up or Alt+Left in
/// its Ctrl+F field, and a breadcrumb click focus the restored listing; the pane
/// header's Back button keeps focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Return {
    AltUp,
    FilterAltUp,
    FilterAltLeft,
    BackButton,
    Breadcrumb,
}

fn mapped_button(widget: &gtk::Widget, matches: &dyn Fn(&gtk::Button) -> bool) -> Option<gtk::Button> {
    if widget.is_mapped()
        && let Some(button) = widget.downcast_ref::<gtk::Button>()
        && matches(button)
    {
        return Some(button.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = mapped_button(&widget, matches) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

fn icons_return_case(
    leave_in: BrowserMode,
    return_in: BrowserMode,
    route: Return,
) -> Result<(), String> {
    let fixture = KeyboardFixture::new();
    let browser = fixture.view.browser();
    // Not the first entry, which a return that restores nothing selects.
    for folder in ["alpha", "nest"] {
        let path = fixture._directory.path().join(folder);
        std::fs::create_dir(&path).expect("folder");
        std::fs::write(path.join("inner.txt"), b"x").expect("folder file");
    }
    fixture.view.refresh();
    wait_until(|| entry_count(&browser) == 5);
    fixture.view.set_view_mode(leave_in);
    select_named(&fixture, "nest");
    fixture.press(Key::Return, ModifierType::empty());
    if !settles(|| location_ends_with(browser.active_location(), "nest")) {
        return Err("Return did not open nest".to_owned());
    }
    wait_loaded(&browser, 0);
    fixture.view.set_view_mode(return_in);
    focus_files(&fixture);
    match route {
        Return::AltUp => {
            fixture.press(Key::Up, ModifierType::ALT_MASK);
        }
        Return::FilterAltUp | Return::FilterAltLeft => {
            if !fixture.press(Key::f, ModifierType::CONTROL_MASK) || !fixture.view.filter_has_focus()
            {
                return Err(format!(
                    "setup: Ctrl+F did not focus the field; focus is on {}",
                    describe_focus(&fixture)
                ));
            }
            let key = if route == Return::FilterAltUp {
                Key::Up
            } else {
                Key::Left
            };
            fixture.press(key, ModifierType::ALT_MASK);
        }
        Return::BackButton | Return::Breadcrumb => {
            let parent = fixture
                ._directory
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            let button = mapped_button(fixture.window.upcast_ref(), &|button| match route {
                // Back leads the pane header's history buttons.
                Return::BackButton => {
                    button.has_css_class("list-navigation-button")
                        && button.prev_sibling().is_none()
                        && button
                            .parent()
                            .is_some_and(|parent| parent.has_css_class("list-navigation"))
                }
                _ => button.has_css_class("breadcrumb") && button.label().as_deref() == Some(&parent),
            })
            .ok_or_else(|| format!("setup: no {route:?} to click"))?;
            button.grab_focus();
            button.emit_clicked();
        }
    }
    if !settles(|| browser.active_location() == Some(Location::local(fixture._directory.path())))
    {
        return Err(format!("{route:?} did not return to the parent"));
    }
    wait_loaded(&browser, 0);
    let mut failures = Vec::new();
    if focused_name(&browser) != "nest" {
        failures.push(format!(
            "the cursor is on {:?}, not nest",
            focused_name(&browser)
        ));
    }
    if selected_names(&browser) != ["nest"] {
        failures.push(format!(
            "the selection is {:?}, not [nest]",
            selected_names(&browser)
        ));
    }
    match route {
        Return::AltUp | Return::FilterAltUp | Return::FilterAltLeft | Return::Breadcrumb
            if !settles(|| focus_shows(&fixture, "nest")) =>
        {
            failures.push(format!(
                "focus is on {}, not the nest item",
                describe_focus(&fixture)
            ));
        }
        Return::BackButton
            if {
                // Focus that must stay put has no settle condition.
                pump(300);
                !focus(&fixture)
                    .is_some_and(|focused| focused.has_css_class("list-navigation-button"))
            } =>
        {
            failures.push(format!(
                "the restore took focus from the header to {}",
                describe_focus(&fixture)
            ));
        }
        _ => {}
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[test]
fn returning_to_a_visited_directory_restores_the_icons_cursor() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::reload_cursor::returning_to_a_visited_directory_restores_the_icons_cursor",
        || {
            use BrowserMode::{Icons, List};
            let failures = [
                (Icons, Icons, Return::AltUp),
                (List, Icons, Return::AltUp),
                (Icons, Icons, Return::BackButton),
                (Icons, Icons, Return::Breadcrumb),
                (Icons, Icons, Return::FilterAltUp),
                (List, List, Return::FilterAltUp),
                (Icons, Icons, Return::FilterAltLeft),
                (List, List, Return::FilterAltLeft),
            ]
            .into_iter()
            .filter_map(|(leave_in, return_in, route)| {
                icons_return_case(leave_in, return_in, route)
                    .err()
                    .map(|error| {
                        format!("left {leave_in:?}, returned in {return_in:?} by {route:?}: {error}")
                    })
            })
            .collect();
            report(failures);
        },
    );
}
