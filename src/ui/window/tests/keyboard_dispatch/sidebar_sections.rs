// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::window::keyboard::chords::{GoTarget, go_target};

fn seed_pins(fixture: &KeyboardFixture) {
    seed_state_pins(&fixture.sidebar.state, fixture._directory.path());
}

fn seed_state_pins(state: &Rc<SidebarState>, base: &std::path::Path) {
    for (directory, name) in [("pins-one", "Pins One"), ("pins-two", "Pins Two")] {
        let path = base.join(directory);
        std::fs::create_dir_all(&path).expect("pin directory");
        state
            .pinned_places
            .borrow_mut()
            .push((Location::local(&path), name.into()));
    }
    state.rebuild();
}

fn section_toggle(state: &Rc<SidebarState>) -> gtk::Button {
    widget_with_class(
        state.places_for_test().upcast_ref(),
        "sidebar-heading-toggle",
    )
    .expect("pinned section toggle")
    .downcast::<gtk::Button>()
    .expect("toggle button")
}

fn section_revealer(state: &Rc<SidebarState>) -> gtk::Revealer {
    widget_with_class(
        state.places_for_test().upcast_ref(),
        "sidebar-section-revealer",
    )
    .expect("pinned section revealer")
    .downcast::<gtk::Revealer>()
    .expect("revealer")
}

fn badge(toggle: &gtk::Button) -> gtk::Label {
    widget_with_class(toggle.upcast_ref(), "sidebar-heading-count")
        .expect("count badge")
        .downcast::<gtk::Label>()
        .expect("badge label")
}

fn badge_text(state: &Rc<SidebarState>) -> String {
    badge(&section_toggle(state)).label().to_string()
}

fn pinned_rows(state: &Rc<SidebarState>) -> Vec<gtk::Button> {
    fn collect(widget: &gtk::Widget, rows: &mut Vec<gtk::Button>) {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>()
            && button.has_css_class("sidebar-pinned-row")
        {
            rows.push(button);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            collect(&current, rows);
            child = current.next_sibling();
        }
    }
    let mut rows = Vec::new();
    collect(state.places_for_test().upcast_ref(), &mut rows);
    rows
}

#[test]
fn pinned_section_toggle_hides_rows_and_persists() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::pinned_section_toggle_hides_rows_and_persists",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            seed_pins(&fixture);
            let state = &fixture.sidebar.state;
            let toggle = section_toggle(state);
            let revealer = section_revealer(state);
            assert!(revealer.reveals_child());
            assert!(toggle.has_css_class("expanded"));
            assert!(!badge(&toggle).is_visible());
            assert_eq!(state.visible_pins().len(), 2);
            assert!(!preferences.sidebar_pinned_collapsed());

            toggle.emit_by_name::<()>("clicked", &[]);
            assert!(preferences.sidebar_pinned_collapsed());
            assert!(!revealer.reveals_child());
            assert!(!toggle.has_css_class("expanded"));
            assert_eq!(
                toggle.tooltip_text().as_deref(),
                Some("Expand pinned places")
            );
            let count = badge(&toggle);
            assert!(count.is_visible());
            assert_eq!(count.label(), "2");
            assert!(state.visible_pins().is_empty());
            assert_eq!(
                go_target(Key::_1, &state.visible_pins()),
                Some(GoTarget::Missing("No pin 1".into()))
            );
            for row in pinned_rows(state) {
                wait_until(|| !row.is_mapped());
            }

            toggle.emit_by_name::<()>("clicked", &[]);
            assert!(!preferences.sidebar_pinned_collapsed());
            assert!(revealer.reveals_child());
            assert!(toggle.has_css_class("expanded"));
            assert_eq!(
                toggle.tooltip_text().as_deref(),
                Some("Collapse pinned places")
            );
            assert!(!badge(&toggle).is_visible());
            assert_eq!(state.visible_pins().len(), 2);
            for row in pinned_rows(state) {
                wait_until(|| row.is_mapped());
            }
        },
    );
}

#[test]
fn saved_collapsed_state_applies_on_rebuild_and_syncs_two_sidebars() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::saved_collapsed_state_applies_on_rebuild_and_syncs_two_sidebars",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            seed_pins(&fixture);
            let second = build_sidebar(fixture.view.clone(), preferences.clone(), true);
            seed_state_pins(&second.state, fixture._directory.path());
            for state in [&fixture.sidebar.state, &second.state] {
                assert!(section_revealer(state).reveals_child());
            }

            preferences.set_sidebar_pinned_collapsed(true);
            for state in [&fixture.sidebar.state, &second.state] {
                assert!(!section_revealer(state).reveals_child());
                assert!(state.visible_pins().is_empty());
            }

            fixture.sidebar.state.rebuild();
            assert!(!section_revealer(&fixture.sidebar.state).reveals_child());
            assert!(fixture.sidebar.state.visible_pins().is_empty());

            preferences.set_sidebar_pinned_collapsed(false);
            for state in [&fixture.sidebar.state, &second.state] {
                assert!(section_revealer(state).reveals_child());
                assert_eq!(state.visible_pins().len(), 2);
            }
            second.disconnect();
        },
    );
}

#[test]
fn collapsed_first_then_navigate_detaches_on_selection() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::collapsed_first_then_navigate_detaches_on_selection",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            seed_pins(&fixture);
            let state = &fixture.sidebar.state;
            let browser = fixture.view.browser();
            let sidebar_box = state.places_for_test();

            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            assert!(preferences.sidebar_pinned_collapsed());
            for row in pinned_rows(state) {
                wait_until(|| !row.is_mapped());
            }
            assert!(state.visible_pins().is_empty());

            browser.navigate(pin_location(state, "Pins Two"));
            let pins_two = pin_row(state, "Pins Two");
            wait_until(|| pins_two.is_mapped());
            assert_eq!(pins_two.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            assert_eq!(state.visible_pins(), [pin_location(state, "Pins Two")]);

            browser.navigate(Location::local(fixture._directory.path()));
            wait_until(|| !pins_two.is_mapped());
            assert!(state.visible_pins().is_empty());
            assert!(state.detached_pin.borrow().is_none());
        },
    );
}

#[test]
fn devices_section_absent_without_devices_but_pref_survives() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::devices_section_absent_without_devices_but_pref_survives",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            seed_pins(&fixture);
            preferences.set_sidebar_devices_collapsed(true);
            fixture.sidebar.state.rebuild();
            let mut toggles = Vec::new();
            let mut child = fixture.sidebar.state.places_for_test().first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                if widget.has_css_class("sidebar-heading-toggle") {
                    toggles.push(widget);
                }
            }
            assert_eq!(toggles.len(), 1, "only the pinned section toggles");
            assert!(preferences.sidebar_devices_collapsed());
            preferences.set_sidebar_devices_collapsed(false);
            fixture.sidebar.state.rebuild();
            assert!(!preferences.sidebar_devices_collapsed());
        },
    );
}

fn pin_location(state: &Rc<SidebarState>, name: &str) -> Location {
    state
        .pinned_places
        .borrow()
        .iter()
        .find(|(_, label)| label == name)
        .map(|(location, _)| location.clone())
        .expect("seeded pin")
}

fn pin_row(state: &Rc<SidebarState>, name: &str) -> gtk::Button {
    let location = pin_location(state, name);
    state
        .place_rows
        .borrow()
        .iter()
        .find(|(entry, _)| entry == &location)
        .map(|(_, row)| row.clone())
        .expect("pin row")
}

#[test]
fn collapsed_section_keeps_active_pin_visible() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::collapsed_section_keeps_active_pin_visible",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            seed_pins(&fixture);
            let state = &fixture.sidebar.state;
            let browser = fixture.view.browser();
            let sidebar_box = state.places_for_test();

            let pins_two = pin_row(state, "Pins Two");
            browser.navigate(pin_location(state, "Pins Two"));
            wait_until(|| {
                pins_two.has_css_class("active") && state.detached_pin.borrow().is_none()
            });

            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            assert!(preferences.sidebar_pinned_collapsed());
            wait_until(|| pins_two.is_mapped());
            assert_eq!(pins_two.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            let pins_one = pin_row(state, "Pins One");
            wait_until(|| !pins_one.is_mapped());
            assert_eq!(state.visible_pins().len(), 1);
            assert_eq!(
                go_target(Key::_1, &state.visible_pins()),
                Some(GoTarget::Place {
                    location: state.visible_pins()[0].clone(),
                    validate: true,
                })
            );
            assert_eq!(
                go_target(Key::_2, &state.visible_pins()),
                Some(GoTarget::Missing("No pin 2".into()))
            );

            browser.navigate(pin_location(state, "Pins One"));
            wait_until(|| pins_one.is_mapped());
            assert_eq!(pins_one.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            wait_until(|| !pins_two.is_mapped());
            assert_eq!(state.visible_pins().len(), 1);

            state.rebuild();
            let pins_one = pin_row(state, "Pins One");
            wait_until(|| pins_one.is_mapped());
            assert_eq!(pins_one.parent().as_ref(), Some(sidebar_box.upcast_ref()));

            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            assert_eq!(state.visible_pins().len(), 2);
            for row in pinned_rows(state) {
                wait_until(|| row.is_mapped());
            }
        },
    );
}

#[test]
fn interactive_sidebar_detach_cycle_keeps_every_row() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::interactive_sidebar_detach_cycle_keeps_every_row",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            let interactive = build_sidebar(fixture.view.clone(), preferences.clone(), false);
            seed_state_pins(&interactive.state, fixture._directory.path());
            let window = gtk::Window::new();
            window.set_child(Some(&interactive.widget));
            window.present();
            wait_until(|| interactive.widget.is_mapped());
            let state = &interactive.state;
            let browser = fixture.view.browser();

            browser.navigate(pin_location(state, "Pins Two"));
            wait_until(|| pin_row(state, "Pins Two").has_css_class("active"));

            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            assert!(preferences.sidebar_pinned_collapsed());
            wait_until(|| pin_row(state, "Pins Two").is_mapped());
            wait_until(|| !pin_row(state, "Pins One").is_mapped());

            browser.navigate(pin_location(state, "Pins One"));
            wait_until(|| pin_row(state, "Pins One").is_mapped());
            wait_until(|| !pin_row(state, "Pins Two").is_mapped());

            browser.navigate(Location::local(fixture._directory.path()));
            wait_until(|| !pin_row(state, "Pins One").is_mapped());
            wait_until(|| !pin_row(state, "Pins Two").is_mapped());
            assert!(state.visible_pins().is_empty());

            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            assert_eq!(pinned_rows(state).len(), 2);
            for row in pinned_rows(state) {
                wait_until(|| row.is_mapped());
            }
            interactive.disconnect();
            window.close();
        },
    );
}

fn pin_order(state: &Rc<SidebarState>) -> Vec<String> {
    pinned_rows(state)
        .into_iter()
        .filter_map(|button| {
            let location = state
                .place_rows
                .borrow()
                .iter()
                .find_map(|(entry, row)| (row == &button).then(|| entry.clone()));
            location.and_then(|loc| {
                state
                    .pinned_places
                    .borrow()
                    .iter()
                    .find_map(|(entry, label)| (entry == &loc).then(|| label.clone()))
            })
        })
        .collect()
}

#[test]
fn selected_rides_the_block_both_directions() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::selected_rides_the_block_both_directions",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(false);
            let interactive = build_sidebar(fixture.view.clone(), preferences.clone(), false);
            seed_state_pins(&interactive.state, fixture._directory.path());
            let window = gtk::Window::new();
            window.set_child(Some(&interactive.widget));
            window.present();
            wait_until(|| interactive.widget.is_mapped());
            let state = &interactive.state;
            let browser = fixture.view.browser();
            let sidebar_box = state.places_for_test();

            browser.navigate(pin_location(state, "Pins Two"));
            wait_until(|| pin_row(state, "Pins Two").has_css_class("active"));

            // Collapse: the selected row remains in sidebar_box between the revealers,
            // staying mapped the entire time as the upper revealer folds away.
            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            assert!(preferences.sidebar_pinned_collapsed());
            let revealer = section_revealer(state);
            let pins_two = pin_row(state, "Pins Two");
            assert_eq!(pins_two.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            assert!(
                pins_two.is_mapped(),
                "selected pin remains visible without popping"
            );
            wait_until(|| !revealer.is_child_revealed());
            assert!(pins_two.is_mapped());
            assert_eq!(pins_two.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            wait_until(|| !pin_row(state, "Pins One").is_mapped());

            // Expand: the selected row remains mapped and rides the opening
            // block down into its slot without disappearing.
            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            assert!(state.detached_pin.borrow().is_none());
            assert_eq!(pins_two.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            assert!(
                pins_two.is_mapped(),
                "selected pin stays visible while expanding"
            );
            wait_until(|| revealer.is_child_revealed());
            let pins_one = pin_row(state, "Pins One");
            wait_until(|| pins_one.is_mapped());
            assert_eq!(pin_order(state), ["Pins One", "Pins Two"]);
            interactive.disconnect();
            window.close();
        },
    );
}

#[test]
fn middle_and_last_pins_merge_back_in_order() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::middle_and_last_pins_merge_back_in_order",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            for (directory, name) in [("pin-a", "Pin A"), ("pin-b", "Pin B"), ("pin-c", "Pin C")] {
                let path = fixture._directory.path().join(directory);
                std::fs::create_dir_all(&path).expect("pin directory");
                fixture
                    .sidebar
                    .state
                    .pinned_places
                    .borrow_mut()
                    .push((Location::local(&path), name.into()));
            }
            fixture.sidebar.state.rebuild();
            let state = &fixture.sidebar.state;
            let browser = fixture.view.browser();
            let sidebar_box = state.places_for_test();

            browser.navigate(pin_location(state, "Pin B"));
            wait_until(|| pin_row(state, "Pin B").has_css_class("active"));
            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            assert!(preferences.sidebar_pinned_collapsed());
            let pin_b = pin_row(state, "Pin B");
            wait_until(|| pin_b.is_mapped());
            assert_eq!(pin_b.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            let upper_revealer = section_revealer(state);
            assert_eq!(
                upper_revealer.next_sibling().as_ref(),
                Some(pin_b.upcast_ref()),
                "selected row sits directly after upper revealer under the header"
            );
            wait_until(|| !pin_row(state, "Pin A").is_mapped());
            wait_until(|| !pin_row(state, "Pin C").is_mapped());
            assert_eq!(badge_text(state), "3");

            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            wait_until(|| state.detached_pin.borrow().is_none());
            for row in pinned_rows(state) {
                wait_until(|| row.is_mapped());
            }
            assert_eq!(pin_order(state), ["Pin A", "Pin B", "Pin C"]);

            browser.navigate(pin_location(state, "Pin C"));
            wait_until(|| pin_row(state, "Pin C").has_css_class("active"));
            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            let pin_c = pin_row(state, "Pin C");
            wait_until(|| pin_c.is_mapped());
            assert_eq!(pin_c.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            let upper_revealer = section_revealer(state);
            assert_eq!(
                upper_revealer.next_sibling().as_ref(),
                Some(pin_c.upcast_ref()),
                "last pin sits directly after upper revealer under the header"
            );
            wait_until(|| !pin_row(state, "Pin A").is_mapped());
            wait_until(|| !pin_row(state, "Pin B").is_mapped());

            section_toggle(state).emit_by_name::<()>("clicked", &[]);
            wait_until(|| state.detached_pin.borrow().is_none());
            for row in pinned_rows(state) {
                wait_until(|| row.is_mapped());
            }
            assert_eq!(pin_order(state), ["Pin A", "Pin B", "Pin C"]);
            assert_eq!(state.visible_pins().len(), 3);
        },
    );
}
