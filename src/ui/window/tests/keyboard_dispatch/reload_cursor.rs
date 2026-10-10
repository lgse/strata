// SPDX-License-Identifier: MIT

use super::*;

/// Where keyboard focus was when the reload started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    Items,
    FilterEntry,
    Sidebar,
}

/// What starts the reload: the F5 key, the pane's Refresh button, or a tick of the
/// auto-refresh timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trigger {
    F5,
    RefreshButton,
    AutoRefresh,
}

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

/// `f00`..`f11`, then `bulk` files listed after them.
fn twelve_files(fixture: &KeyboardFixture, bulk: usize) {
    for index in 0..12 {
        std::fs::write(fixture._directory.path().join(format!("f{index:02}")), b"x")
            .expect("fixture file");
    }
    for index in 0..bulk {
        std::fs::write(
            fixture._directory.path().join(format!("zz{index:04}")),
            b"x",
        )
        .expect("bulk file");
    }
    let browser = fixture.view.browser();
    fixture.view.refresh();
    if !settles(|| entry_count(&browser) == 15 + bulk) {
        panic!("setup: the folder never listed {} entries", 15 + bulk);
    }
}

/// What the user does while the reload is still loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum During {
    Nothing,
    /// Escape dismisses the focused filter, so the listing takes focus instead.
    Escape,
}

/// One refresh scenario: where focus is, whether the cursor's file survives, what
/// starts the reload, how many more files the listing holds, and what happens while
/// it loads.
#[derive(Clone, Copy, Debug)]
struct RefreshCase {
    owner: Owner,
    cursor_kept: bool,
    trigger: Trigger,
    /// Enough files that the reload spans a frame, so GTK moves focus off the hidden
    /// pane before the load finishes.
    bulk: usize,
    during: During,
}

const REFRESH_CASES: &[RefreshCase] = &[
    RefreshCase {
        owner: Owner::Items,
        cursor_kept: true,
        trigger: Trigger::F5,
        bulk: 0,
        during: During::Nothing,
    },
    RefreshCase {
        owner: Owner::Items,
        cursor_kept: false,
        trigger: Trigger::F5,
        bulk: 0,
        during: During::Nothing,
    },
    RefreshCase {
        owner: Owner::FilterEntry,
        cursor_kept: true,
        trigger: Trigger::AutoRefresh,
        bulk: 0,
        during: During::Nothing,
    },
    // The timer ticks after the monitor has already reported a deletion as a live
    // change, so only an immediate reload lists the folder without the cursor's entry.
    RefreshCase {
        owner: Owner::FilterEntry,
        cursor_kept: false,
        trigger: Trigger::RefreshButton,
        bulk: 0,
        during: During::Nothing,
    },
    RefreshCase {
        owner: Owner::FilterEntry,
        cursor_kept: true,
        trigger: Trigger::F5,
        bulk: 5000,
        during: During::Nothing,
    },
    RefreshCase {
        owner: Owner::Sidebar,
        cursor_kept: true,
        trigger: Trigger::F5,
        bulk: 0,
        during: During::Nothing,
    },
    RefreshCase {
        owner: Owner::Sidebar,
        cursor_kept: false,
        trigger: Trigger::F5,
        bulk: 0,
        during: During::Nothing,
    },
    RefreshCase {
        owner: Owner::FilterEntry,
        cursor_kept: true,
        trigger: Trigger::RefreshButton,
        bulk: 5000,
        during: During::Escape,
    },
];

/// Counts finished loads of the first column so a reload can be awaited.
fn load_counter(browser: &crate::app::Browser) -> Rc<Cell<usize>> {
    let loads = Rc::new(Cell::new(0));
    let counted = loads.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::LoadFinished { depth: 0, .. }) {
            counted.set(counted.get() + 1);
        }
    });
    loads
}

fn reload(
    fixture: &KeyboardFixture,
    trigger: Trigger,
    loads: &Cell<usize>,
    during: During,
) -> Result<(), String> {
    let before = loads.get();
    match trigger {
        Trigger::F5 => {
            fixture.press(Key::F5, ModifierType::empty());
        }
        Trigger::RefreshButton => fixture.view.refresh(),
        Trigger::AutoRefresh => fixture.view.set_auto_refresh_interval(1),
    }
    if during == During::Escape
        && (loads.get() != before || !fixture.press(Key::Escape, ModifierType::empty()))
    {
        return Err("setup: Escape did not dismiss the filter while the folder loaded".to_owned());
    }
    let reloaded = settles(|| loads.get() > before);
    if trigger == Trigger::AutoRefresh {
        fixture.view.set_auto_refresh_interval(0);
    }
    if !reloaded {
        return Err(format!("{trigger:?} did not reload the folder"));
    }
    // Focus that must stay put has no settle condition: give GTK's post-paint focus
    // move and the restore's settle frames time to run.
    pump(300);
    Ok(())
}

fn refresh_case(mode: BrowserMode, case: RefreshCase) -> Result<(), String> {
    let RefreshCase {
        owner,
        cursor_kept,
        trigger,
        bulk,
        during,
    } = case;
    let fixture = KeyboardFixture::new();
    let browser = fixture.view.browser();
    let loads = load_counter(&browser);
    twelve_files(&fixture, bulk);
    fixture.view.set_view_mode(mode);
    select_named(&fixture, "f05");
    if !settles(|| focus_shows(&fixture, "f05")) {
        return Err(format!(
            "setup: f05 never took focus; focus is on {}",
            describe_focus(&fixture)
        ));
    }
    let field = match owner {
        Owner::Items => None,
        Owner::FilterEntry => {
            if !fixture.press(Key::f, ModifierType::CONTROL_MASK)
                || !fixture.view.filter_has_focus()
            {
                return Err(format!(
                    "setup: Ctrl+F did not focus the field; focus is on {}",
                    describe_focus(&fixture)
                ));
            }
            let field = focused_entry(&fixture.window);
            field.set_text("0");
            // Past the filter's debounce.
            pump(300);
            Some(field)
        }
        Owner::Sidebar => {
            fixture.press(
                Key::b,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
            );
            if !settles(|| sidebar_has_focus(&fixture)) {
                return Err("setup: Ctrl+Shift+B did not focus the sidebar".to_owned());
            }
            None
        }
    };
    if !cursor_kept {
        std::fs::remove_file(fixture._directory.path().join("f05")).expect("remove f05");
    }
    reload(&fixture, trigger, &loads, during)?;

    let mut failures = Vec::new();
    let (cursor, selection) = if cursor_kept {
        ("f05", vec!["f05".to_owned()])
    } else {
        ("f06", Vec::new())
    };
    if focused_name(&browser) != cursor {
        failures.push(format!(
            "the cursor is on {:?}, not {cursor}",
            focused_name(&browser)
        ));
    }
    if selected_names(&browser) != selection {
        failures.push(format!(
            "the selection is {:?}, not {selection:?}",
            selected_names(&browser)
        ));
    }
    let owner = if during == During::Escape {
        Owner::Items
    } else {
        owner
    };
    match owner {
        Owner::Items => {
            if !settles(|| focus_shows(&fixture, cursor)) {
                failures.push(format!(
                    "focus is on {}, not the {cursor} row",
                    describe_focus(&fixture)
                ));
            }
        }
        Owner::FilterEntry => {
            let field = field.expect("filter field");
            if !fixture.view.filter_has_focus() {
                failures.push(format!(
                    "the filter field lost focus to {}",
                    describe_focus(&fixture)
                ));
            }
            if field.text() != "0" {
                failures.push(format!("the filter text became {:?}", field.text()));
            }
            let location = browser.active_location();
            fixture.press(Key::BackSpace, ModifierType::empty());
            pump(100);
            if browser.active_location() != location {
                failures.push("Backspace left the folder instead of editing the filter".to_owned());
            }
        }
        Owner::Sidebar => {
            if !sidebar_has_focus(&fixture) {
                failures.push(format!(
                    "the reload took focus from the sidebar to {}",
                    describe_focus(&fixture)
                ));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[test]
fn a_refresh_keeps_the_cursor_and_focus_in_list_and_icons() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::reload_cursor::a_refresh_keeps_the_cursor_and_focus_in_list_and_icons",
        || {
            let mut failures = Vec::new();
            for mode in [BrowserMode::List, BrowserMode::Icons] {
                for &case in REFRESH_CASES {
                    if let Err(error) = refresh_case(mode, case) {
                        failures.push(format!("{mode:?} {case:?}: {error}"));
                    }
                }
            }
            report(failures);
        },
    );
}

/// What starts right after a return, while its viewport is still settling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DuringSettle {
    /// The header's Forward button leaves again, and Back returns once more.
    ForwardAndBack,
    /// A reload, as an Auto-refresh tick or a monitor rescan starts one.
    Reload,
}

/// The vertical scroll position of the mapped scroller with far more content than
/// fits: the folder's rows.
fn list_viewport(widget: &gtk::Widget) -> Option<gtk::Adjustment> {
    if widget.is_mapped()
        && let Some(scroller) = widget.downcast_ref::<gtk::ScrolledWindow>()
        && scroller.vadjustment().upper() > scroller.vadjustment().page_size() * 2.0
    {
        return Some(scroller.vadjustment());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(adjustment) = list_viewport(&widget) {
            return Some(adjustment);
        }
        child = widget.next_sibling();
    }
    None
}

fn wait_for_viewport(fixture: &KeyboardFixture) -> Option<gtk::Adjustment> {
    let found = RefCell::new(None);
    settles(|| {
        *found.borrow_mut() = list_viewport(fixture.window.upcast_ref());
        found.borrow().is_some()
    });
    found.into_inner()
}

fn history_button(fixture: &KeyboardFixture, index: usize) -> Result<gtk::Button, String> {
    mapped_button(fixture.window.upcast_ref(), &|button| {
        button.has_css_class("list-navigation-button")
            && button
                .parent()
                .is_some_and(|parent| parent.has_css_class("list-navigation"))
            && std::iter::successors(button.prev_sibling(), |widget| widget.prev_sibling()).count()
                == index
    })
    .ok_or_else(|| format!("setup: no history button {index} in the pane header"))
}

fn settled_value(adjustment: &gtk::Adjustment) -> f64 {
    let samples = RefCell::new(Vec::new());
    settles(|| {
        pump(16);
        let mut samples = samples.borrow_mut();
        samples.push(adjustment.value());
        samples.len() >= 3
            && samples[samples.len() - 3..]
                .windows(2)
                .all(|pair| pair[0] == pair[1])
    });
    adjustment.value()
}

fn capture_during_settle_case(during: DuringSettle) -> Result<(), String> {
    let fixture = KeyboardFixture::new();
    let browser = fixture.view.browser();
    let loads = load_counter(&browser);
    for index in 0..160 {
        std::fs::create_dir(fixture._directory.path().join(format!("d{index:03}")))
            .expect("folder");
    }
    fixture.view.refresh();
    wait_until(|| entry_count(&browser) == 163);
    fixture.view.set_view_mode(BrowserMode::List);
    select_named(&fixture, "d080");
    let viewport = wait_for_viewport(&fixture).ok_or("setup: no scrollable list")?;
    let target = settled_value(&viewport);
    if target <= 0.0 {
        return Err("setup: selecting d080 did not scroll the listing".to_owned());
    }
    fixture.press(Key::Return, ModifierType::empty());
    if !settles(|| location_ends_with(browser.active_location(), "d080")) {
        return Err("Return did not open d080".to_owned());
    }
    wait_loaded(&browser, 0);
    let parent = Location::local(fixture._directory.path());
    let back = |fixture: &KeyboardFixture| -> Result<(), String> {
        history_button(fixture, 0)?.emit_clicked();
        if !settles(|| browser.active_location() == Some(parent.clone())) {
            return Err("Back did not return to the parent".to_owned());
        }
        wait_loaded(&browser, 0);
        Ok(())
    };
    back(&fixture)?;
    // No frame has run since the load finished: the restore has not settled yet.
    match during {
        DuringSettle::ForwardAndBack => {
            history_button(&fixture, 1)?.emit_clicked();
            if !settles(|| location_ends_with(browser.active_location(), "d080")) {
                return Err("Forward did not reopen d080".to_owned());
            }
            wait_loaded(&browser, 0);
            back(&fixture)?;
        }
        DuringSettle::Reload => reload(&fixture, Trigger::RefreshButton, &loads, During::Nothing)?,
    }
    let viewport = wait_for_viewport(&fixture).ok_or("no scrollable list after the return")?;
    let restored = settled_value(&viewport);
    if (restored - target).abs() > 1.0 {
        return Err(format!("the viewport is at {restored}, not {target}"));
    }
    Ok(())
}

#[test]
fn leaving_or_reloading_while_a_return_settles_keeps_its_viewport() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::reload_cursor::leaving_or_reloading_while_a_return_settles_keeps_its_viewport",
        || {
            let failures = [DuringSettle::ForwardAndBack, DuringSettle::Reload]
                .into_iter()
                .filter_map(|during| {
                    capture_during_settle_case(during)
                        .err()
                        .map(|error| format!("{during:?}: {error}"))
                })
                .collect();
            report(failures);
        },
    );
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

fn mapped_button(
    widget: &gtk::Widget,
    matches: &dyn Fn(&gtk::Button) -> bool,
) -> Option<gtk::Button> {
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
            if !fixture.press(Key::f, ModifierType::CONTROL_MASK)
                || !fixture.view.filter_has_focus()
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
                _ => {
                    button.has_css_class("breadcrumb") && button.label().as_deref() == Some(&parent)
                }
            })
            .ok_or_else(|| format!("setup: no {route:?} to click"))?;
            button.grab_focus();
            button.emit_clicked();
        }
    }
    if !settles(|| browser.active_location() == Some(Location::local(fixture._directory.path()))) {
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
                        format!(
                            "left {leave_in:?}, returned in {return_in:?} by {route:?}: {error}"
                        )
                    })
            })
            .collect();
            report(failures);
        },
    );
}

fn columns_reload_case(owner: Owner, trigger: Trigger) -> Result<(), String> {
    let fixture = KeyboardFixture::new();
    let browser = fixture.view.browser();
    let loads = load_counter(&browser);
    select_named(&fixture, "b.txt");
    if !settles(|| focus_shows(&fixture, "b.txt")) {
        return Err(format!(
            "setup: b.txt never took focus; focus is on {}",
            describe_focus(&fixture)
        ));
    }
    if owner == Owner::FilterEntry
        && (!fixture.press(Key::f, ModifierType::CONTROL_MASK) || !fixture.view.filter_has_focus())
    {
        return Err(format!(
            "setup: Ctrl+F did not focus the field; focus is on {}",
            describe_focus(&fixture)
        ));
    }
    reload(&fixture, trigger, &loads, During::Nothing)?;
    let mut failures = Vec::new();
    if focused_name(&browser) != "b.txt" {
        failures.push(format!(
            "the cursor is on {:?}, not b.txt",
            focused_name(&browser)
        ));
    }
    match owner {
        Owner::Items if !settles(|| focus_shows(&fixture, "b.txt")) => failures.push(format!(
            "focus is on {}, not the b.txt row",
            describe_focus(&fixture)
        )),
        Owner::FilterEntry if !fixture.view.filter_has_focus() => failures.push(format!(
            "the filter field lost focus to {}",
            describe_focus(&fixture)
        )),
        _ => {}
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[test]
fn a_reload_keeps_the_columns_list_focused() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::reload_cursor::a_reload_keeps_the_columns_list_focused",
        || {
            let mut failures = Vec::new();
            for owner in [Owner::Items, Owner::FilterEntry] {
                for trigger in [Trigger::F5, Trigger::AutoRefresh] {
                    if let Err(error) = columns_reload_case(owner, trigger) {
                        failures.push(format!("{owner:?} {trigger:?}: {error}"));
                    }
                }
            }
            report(failures);
        },
    );
}

const SHARE: &str = "sftp://share.invalid/reload";

/// A batch-loaded share whose listing tests can shrink between loads.
struct ShrinkingShare(RefCell<Vec<&'static str>>);

impl crate::services::FileSource for ShrinkingShare {
    fn validate_location(
        &self,
        _location: &crate::model::Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        request: crate::services::DirectoryRequest,
        emit: Rc<dyn Fn(crate::services::DirectoryEvent)>,
    ) -> LoadHandle {
        let entries = self
            .0
            .borrow()
            .iter()
            .map(|name| crate::model::FileEntry {
                location: crate::model::Location::uri(format!("{SHARE}/{name}")),
                thumbnail_path: None,
                native_name: (*name).into(),
                display_name: (*name).into(),
                kind: crate::model::EntryKind::File,
                size: crate::model::MetadataValue::Unknown,
                modified_unix_seconds: crate::model::MetadataValue::Unknown,
                recent_unix_seconds: crate::model::MetadataValue::Unknown,
                recent_uri: None,
                mode: crate::model::MetadataValue::Unknown,
                image_dimensions: crate::model::MetadataValue::Unknown,
                child_count: crate::model::MetadataValue::Unknown,
                duration_seconds: crate::model::MetadataValue::Unknown,
                is_hidden: false,
            })
            .collect();
        emit(crate::services::DirectoryEvent::Batch {
            request_id: request.id,
            entries,
        });
        emit(crate::services::DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

fn remote_reload_case(mode: BrowserMode) -> Result<(), String> {
    let source = Rc::new(ShrinkingShare(RefCell::new(vec![
        "a.txt", "b.txt", "c.txt",
    ])));
    let view_source = source.clone();
    let fixture = KeyboardFixture::with_parts(Rc::new(TextPreview), move || {
        BrowserView::new(view_source, crate::ui::browser::PeekBehavior::default())
    });
    fixture.view.set_view_mode(mode);
    let browser = fixture.view.browser();
    let loads = load_counter(&browser);
    let share = crate::model::Location::uri(SHARE);
    browser.navigate(share.clone());
    wait_until(|| browser.active_location().as_ref() == Some(&share));
    wait_loaded(&browser, 0);
    browser.select(0, 1);
    fixture.press(
        Key::b,
        ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
    );
    if !settles(|| sidebar_has_focus(&fixture)) {
        return Err("setup: Ctrl+Shift+B did not focus the sidebar".to_owned());
    }
    source.0.borrow_mut().retain(|name| *name != "b.txt");
    reload(&fixture, Trigger::F5, &loads, During::Nothing)?;

    let mut failures = Vec::new();
    if focused_name(&browser) != "c.txt" {
        failures.push(format!(
            "the cursor is on {:?}, not c.txt",
            focused_name(&browser)
        ));
    }
    if !sidebar_has_focus(&fixture) {
        failures.push(format!(
            "the reload took focus from the sidebar to {}",
            describe_focus(&fixture)
        ));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[test]
fn a_remote_reload_moves_a_removed_cursor_without_taking_focus() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::reload_cursor::a_remote_reload_moves_a_removed_cursor_without_taking_focus",
        || {
            let mut failures = Vec::new();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                if let Err(error) = remote_reload_case(mode) {
                    failures.push(format!("{mode:?}: {error}"));
                }
            }
            report(failures);
        },
    );
}
