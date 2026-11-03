// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use super::*;

/// Pins come from the isolated GTK bookmarks file, as they do at startup, so
/// the bookmark watch does not later replace rows seeded only in memory.
fn seed_bookmarks(names: &[&str]) {
    let home = PathBuf::from(std::env::var_os("HOME").expect("isolated HOME"));
    let config_home =
        PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("isolated config home"));
    std::fs::create_dir_all(config_home.join("gtk-3.0")).expect("bookmark directory");
    let bookmarks = names
        .iter()
        .map(|name| {
            let path = home
                .join("pins")
                .join(name.to_lowercase().replace(' ', "-"));
            std::fs::create_dir_all(&path).expect("pin directory");
            format!("{} {name}\n", gtk::gio::File::for_path(&path).uri())
        })
        .collect::<String>();
    std::fs::write(config_home.join("gtk-3.0/bookmarks"), bookmarks).expect("bookmarks");
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

/// The Pinned section's children in display order: the upper revealer's rows,
/// whatever sits between the revealers, then the lower revealer's rows. Rows
/// are reported by pin name and drop gaps as `-`.
fn section_sequence(state: &Rc<SidebarState>) -> Vec<String> {
    let sidebar = state.places_for_test();
    let revealers: Vec<gtk::Widget> =
        std::iter::successors(sidebar.first_child(), gtk::Widget::next_sibling)
            .filter(|widget| widget.has_css_class("sidebar-section-revealer"))
            .take(2)
            .collect();
    let [upper, lower] = revealers.as_slice() else {
        panic!("two pinned revealers, found {}", revealers.len());
    };
    let children = |container: &gtk::Widget| {
        std::iter::successors(container.first_child(), gtk::Widget::next_sibling)
            .collect::<Vec<_>>()
    };
    let between: Vec<gtk::Widget> =
        std::iter::successors(upper.next_sibling(), gtk::Widget::next_sibling)
            .take_while(|widget| widget != lower)
            .collect();
    let place_rows = state.place_rows.borrow();
    let pinned_places = state.pinned_places.borrow();
    [
        children(&upper.first_child().expect("upper rows")),
        between,
        children(&lower.first_child().expect("lower rows")),
    ]
    .concat()
    .into_iter()
    .map(|widget| {
        if widget.has_css_class("sidebar-row-gap") {
            return "-".to_owned();
        }
        place_rows
            .iter()
            .find(|(_, row)| *row == widget)
            .and_then(|(location, _)| {
                pinned_places
                    .iter()
                    .find_map(|(entry, label)| (entry == location).then(|| label.clone()))
            })
            .unwrap_or_else(|| format!("{widget:?}"))
    })
    .collect()
}

#[test]
fn pinned_section_toggle_hides_rows_and_persists() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::pinned_section_toggle_hides_rows_and_persists",
        || {
            seed_bookmarks(&["Pins One", "Pins Two"]);
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            let state = &fixture.sidebar.state;
            let toggle = section_toggle(state);
            let revealer = section_revealer(state);
            assert!(revealer.reveals_child());
            assert!(toggle.has_css_class("expanded"));
            assert!(!badge(&toggle).is_visible());
            assert!(!preferences.sidebar_pinned_collapsed());

            toggle.emit_clicked();
            assert!(preferences.sidebar_pinned_collapsed());
            assert!(!revealer.reveals_child());
            assert!(!toggle.has_css_class("expanded"));
            let count = badge(&toggle);
            assert!(count.is_visible());
            assert_eq!(count.label(), "2");
            assert_eq!(
                state.visible_pins().len(),
                2,
                "g chords keep reaching collapsed pins"
            );
            for row in pinned_rows(state) {
                wait_until(|| !row.is_mapped());
            }

            toggle.emit_clicked();
            assert!(!preferences.sidebar_pinned_collapsed());
            assert!(revealer.reveals_child());
            assert!(toggle.has_css_class("expanded"));
            assert!(!badge(&toggle).is_visible());
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
            seed_bookmarks(&["Pins One", "Pins Two"]);
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            let second = build_sidebar(fixture.view.clone(), preferences.clone(), true);
            for state in [&fixture.sidebar.state, &second.state] {
                assert!(section_revealer(state).reveals_child());
            }

            preferences.set_sidebar_pinned_collapsed(true);
            for state in [&fixture.sidebar.state, &second.state] {
                assert!(!section_revealer(state).reveals_child());
            }

            fixture.sidebar.state.rebuild();
            assert!(!section_revealer(&fixture.sidebar.state).reveals_child());
            assert!(!section_toggle(&fixture.sidebar.state).has_css_class("expanded"));

            preferences.set_sidebar_pinned_collapsed(false);
            for state in [&fixture.sidebar.state, &second.state] {
                assert!(section_revealer(state).reveals_child());
            }
            second.disconnect();
        },
    );
}

#[test]
fn the_icon_rail_shows_collapsed_sections() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::the_icon_rail_shows_collapsed_sections",
        || {
            seed_bookmarks(&["Pins One", "Pins Two"]);
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            let state = &fixture.sidebar.state;
            preferences.set_sidebar_pinned_collapsed(true);
            for row in pinned_rows(state) {
                wait_until(|| !row.is_mapped());
            }

            state.set_rail(true);
            assert!(!section_toggle(state).is_visible());
            for row in pinned_rows(state) {
                wait_until(|| row.is_mapped());
            }

            state.set_rail(false);
            assert!(section_toggle(state).is_visible());
            assert!(preferences.sidebar_pinned_collapsed());
            for row in pinned_rows(state) {
                wait_until(|| !row.is_mapped());
            }
        },
    );
}

#[test]
fn navigating_to_a_pin_while_collapsed_shows_only_that_row() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::navigating_to_a_pin_while_collapsed_shows_only_that_row",
        || {
            seed_bookmarks(&["Pins One", "Pins Two"]);
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            let state = &fixture.sidebar.state;
            let browser = fixture.view.browser();
            let sidebar_box = state.places_for_test();

            section_toggle(state).emit_clicked();
            for row in pinned_rows(state) {
                wait_until(|| !row.is_mapped());
            }

            browser.navigate(pin_location(state, "Pins Two"));
            let pins_two = pin_row(state, "Pins Two");
            wait_until(|| pins_two.is_mapped());
            assert_eq!(pins_two.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            assert!(!pin_row(state, "Pins One").is_mapped());

            browser.navigate(Location::local(fixture._directory.path()));
            wait_until(|| !pins_two.is_mapped());
        },
    );
}

#[test]
fn collapsed_section_keeps_active_pin_visible() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::collapsed_section_keeps_active_pin_visible",
        || {
            seed_bookmarks(&["Pins One", "Pins Two"]);
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            let state = &fixture.sidebar.state;
            let browser = fixture.view.browser();
            let sidebar_box = state.places_for_test();

            let pins_two = pin_row(state, "Pins Two");
            browser.navigate(pin_location(state, "Pins Two"));
            wait_until(|| pins_two.has_css_class("active"));

            section_toggle(state).emit_clicked();
            wait_until(|| pins_two.is_mapped());
            assert_eq!(pins_two.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            let pins_one = pin_row(state, "Pins One");
            wait_until(|| !pins_one.is_mapped());

            browser.navigate(pin_location(state, "Pins One"));
            wait_until(|| pins_one.is_mapped());
            assert_eq!(pins_one.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            wait_until(|| !pins_two.is_mapped());

            state.rebuild();
            let pins_one = pin_row(state, "Pins One");
            wait_until(|| pins_one.is_mapped());
            assert_eq!(pins_one.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            wait_until(|| !pin_row(state, "Pins Two").is_mapped());

            browser.navigate(Location::local(fixture._directory.path()));
            wait_until(|| !pins_one.is_mapped());

            section_toggle(state).emit_clicked();
            for row in pinned_rows(state) {
                wait_until(|| row.is_mapped());
            }
            assert_eq!(
                section_sequence(state),
                ["-", "Pins One", "-", "Pins Two", "-"]
            );
        },
    );
}

#[test]
fn selected_rides_the_block_both_directions() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::sidebar_sections::selected_rides_the_block_both_directions",
        || {
            seed_bookmarks(&["Pins One", "Pins Two"]);
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(false);
            let interactive = build_sidebar(fixture.view.clone(), preferences.clone(), false);
            let window = gtk::Window::new();
            window.set_child(Some(&interactive.widget));
            window.present();
            wait_until(|| interactive.widget.is_mapped());
            let state = &interactive.state;
            let browser = fixture.view.browser();
            let sidebar_box = state.places_for_test();

            browser.navigate(pin_location(state, "Pins Two"));
            let pins_two = pin_row(state, "Pins Two");
            wait_until(|| pins_two.has_css_class("active") && pins_two.is_mapped());
            assert_eq!(pins_two.parent().as_ref(), Some(sidebar_box.upcast_ref()));
            let revealer = section_revealer(state);

            section_toggle(state).emit_clicked();
            while revealer.is_child_revealed() {
                assert!(pins_two.is_mapped(), "selected pin stays while collapsing");
                pump(5);
            }
            assert!(pins_two.is_mapped());
            wait_until(|| !pin_row(state, "Pins One").is_mapped());

            section_toggle(state).emit_clicked();
            while !revealer.is_child_revealed() {
                assert!(pins_two.is_mapped(), "selected pin stays while expanding");
                pump(5);
            }
            wait_until(|| pin_row(state, "Pins One").is_mapped());
            assert_eq!(
                section_sequence(state),
                ["-", "Pins One", "-", "Pins Two", "-"]
            );
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
            seed_bookmarks(&["Pin A", "Pin B", "Pin C"]);
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            let state = &fixture.sidebar.state;
            let browser = fixture.view.browser();
            let sidebar_box = state.places_for_test();

            for (active, hidden) in [("Pin B", ["Pin A", "Pin C"]), ("Pin C", ["Pin A", "Pin B"])] {
                browser.navigate(pin_location(state, active));
                wait_until(|| pin_row(state, active).has_css_class("active"));
                section_toggle(state).emit_clicked();
                let row = pin_row(state, active);
                wait_until(|| row.is_mapped());
                assert_eq!(row.parent().as_ref(), Some(sidebar_box.upcast_ref()));
                assert_eq!(
                    section_revealer(state).next_sibling().as_ref(),
                    Some(row.upcast_ref()),
                    "{active} sits directly under the header"
                );
                for name in hidden {
                    wait_until(|| !pin_row(state, name).is_mapped());
                }
                assert_eq!(badge(&section_toggle(state)).label(), "3");

                section_toggle(state).emit_clicked();
                for row in pinned_rows(state) {
                    wait_until(|| row.is_mapped());
                }
                assert_eq!(
                    section_sequence(state),
                    ["-", "Pin A", "-", "Pin B", "-", "Pin C", "-"]
                );
            }
        },
    );
}
