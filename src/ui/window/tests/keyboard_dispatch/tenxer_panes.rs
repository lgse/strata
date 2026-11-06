// SPDX-License-Identifier: MIT

use super::*;

const NONE: ModifierType = ModifierType::empty();
const SIDEBAR_CHORD: ModifierType = ModifierType::CONTROL_MASK.union(ModifierType::SHIFT_MASK);

fn arrows_and_back_tab_stay_in_the_panes(fixture: &KeyboardFixture, mode: BrowserMode) {
    for (key, modifiers) in [
        (Key::Up, NONE),
        (Key::ISO_Left_Tab, NONE),
        (Key::Tab, ModifierType::SHIFT_MASK),
    ] {
        focus_files(fixture);
        fixture.press(key, modifiers);
        assert!(
            fixture.view.item_view_has_focus(),
            "{mode:?} {key:?} left the file panes"
        );
        assert!(
            !sidebar_has_focus(fixture),
            "{mode:?} {key:?} focused the sidebar"
        );
    }
}

fn left_stays_in_the_panes(fixture: &KeyboardFixture, mode: BrowserMode) {
    let browser = fixture.view.browser();
    let origin = browser.active_location();
    focus_files(fixture);
    fixture.press(Key::Left, NONE);
    assert!(
        file_panes_have_focus(fixture),
        "{mode:?} Left left the file panes"
    );
    assert!(!sidebar_has_focus(fixture), "{mode:?}");
    let Some(origin) = origin.filter(|origin| browser.active_location().as_ref() != Some(origin))
    else {
        return;
    };
    browser.navigate(origin);
    wait_loaded(&browser, 0);
}

fn columns_down_keeps_the_fill(fixture: &KeyboardFixture, mode: BrowserMode) {
    assert!(fixture.press(Key::Down, NONE));
    assert_eq!(fixture.selected(), [0], "{mode:?} Down keeps the fill");
    assert_eq!(focused_index(&fixture.view.browser()), 1, "{mode:?}");
    assert!(fixture.view.item_view_has_focus());
}

fn header_and_footer_return_to_the_files(fixture: &KeyboardFixture, mode: BrowserMode) {
    assert!(fixture.sidebar_toggle.grab_focus(), "{mode:?}");
    fixture.press(Key::Right, NONE);
    assert!(fixture.view.item_view_has_focus(), "{mode:?}");
    assert!(!fixture.sidebar_toggle.has_focus(), "{mode:?}");

    let shortcuts = widget_with_class(fixture.window.upcast_ref(), "shortcut-footer-button")
        .expect("shortcuts button");
    wait_until(|| shortcuts.is_mapped());
    assert!(shortcuts.grab_focus(), "{mode:?}");
    fixture.press(Key::Tab, NONE);
    assert!(
        fixture.view.item_view_has_focus(),
        "{mode:?} Tab from the footer must return to the files"
    );
}

fn panes_keep_focus_in(fixture: &KeyboardFixture, mode: BrowserMode) {
    fixture.view.set_view_mode(mode);
    fixture.window.present();
    fixture.view.browser().select(0, 0);
    focus_files(fixture);
    arrows_and_back_tab_stay_in_the_panes(fixture, mode);
    left_stays_in_the_panes(fixture, mode);
    fixture.view.browser().select(0, 0);
    focus_files(fixture);
    if mode == BrowserMode::Columns {
        columns_down_keeps_the_fill(fixture, mode);
    }
    header_and_footer_return_to_the_files(fixture, mode);
}

#[test]
fn tenxer_keeps_keyboard_navigation_inside_the_file_panes() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_panes::tenxer_keeps_keyboard_navigation_inside_the_file_panes",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_arrow_navigation_scoped(false);
            preferences.set_tenxer_mode(true);
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                panes_keep_focus_in(&fixture, mode);
            }

            focus_files(&fixture);
            assert!(fixture.press(Key::l, ModifierType::CONTROL_MASK));
            assert!(fixture.view.location_has_focus());
            assert!(fixture.press(Key::Escape, NONE));
            assert!(!fixture.view.location_has_focus());

            preferences.set_tenxer_mode(false);
            fixture.view.set_view_mode(BrowserMode::List);
            fixture.view.browser().select(0, 0);
            focus_files(&fixture);
            fixture.press(Key::Up, NONE);
            wait_until(|| fixture.view.header_actions_have_focus());
        },
    );
}

struct SidebarRoundTrip<'a> {
    fixture: &'a KeyboardFixture,
    origin: Location,
    place: gtk::Button,
    eject: gtk::Button,
    clicks: Rc<Cell<usize>>,
}

impl SidebarRoundTrip<'_> {
    fn with_device_row(fixture: &KeyboardFixture) -> SidebarRoundTrip<'_> {
        let clicks = Rc::new(Cell::new(0));
        let observed = clicks.clone();
        let place = crate::ui::window::sidebar_button(crate::assets::icons::HARD_DRIVE, "USB");
        let eject = crate::ui::window::sidebar_eject_button(
            crate::ui::window::MediaRelease::UnmountMount,
            move || observed.set(observed.get() + 1),
        );
        let row = crate::ui::window::sidebar_device_row(&place, None, Some(&eject));
        fixture.sidebar.state.places_for_test().append(&row);
        wait_until(|| eject.is_mapped());
        SidebarRoundTrip {
            fixture,
            origin: Location::local(fixture._directory.path()),
            place,
            eject,
            clicks,
        }
    }

    fn press(&self, key: Key) {
        self.fixture.press(key, NONE);
    }

    fn open_origin(&self) {
        let browser = self.fixture.view.browser();
        browser.navigate(self.origin.clone());
        wait_loaded(&browser, 0);
        browser.select(0, 0);
        focus_files(self.fixture);
    }

    fn focus_sidebar(&self) {
        self.fixture.press(Key::b, SIDEBAR_CHORD);
        wait_until(|| sidebar_has_focus(self.fixture));
    }

    fn at_origin(&self) -> bool {
        self.fixture.view.browser().active_location() == Some(self.origin.clone())
    }

    /// (files focused, selection, still at the origin, sidebar shown)
    fn files_state(&self) -> (bool, Vec<usize>, bool, bool) {
        (
            self.fixture.view.item_view_has_focus(),
            self.fixture.selected(),
            self.at_origin(),
            self.fixture.sidebar_toggle.is_active(),
        )
    }

    fn header_keys_return_to_the_files(&self, mode: BrowserMode) {
        let fixture = self.fixture;
        assert!(fixture.sidebar_toggle.is_active(), "{mode:?}");
        assert!(fixture.press(Key::Tab, NONE), "{mode:?}");
        assert!(
            fixture.sidebar_toggle.has_focus(),
            "{mode:?} Tab reaches the header"
        );
        assert_eq!(fixture.selected(), [0], "{mode:?}");
        self.press(Key::h);
        assert_eq!(
            self.files_state(),
            (true, vec![0], true, true),
            "{mode:?} h returns to the files without moving the selection, \
             the location, or toggling the sidebar"
        );

        assert!(fixture.press(Key::Tab, NONE));
        self.press(Key::j);
        assert!(fixture.view.item_view_has_focus(), "{mode:?}");
        assert_eq!(
            fixture.selected(),
            [0],
            "{mode:?} j must not move the selection"
        );
        assert!(fixture.sidebar_toggle.is_active());
    }

    fn space_activates_the_header_toggle(&self, mode: BrowserMode) -> usize {
        let fixture = self.fixture;
        assert!(fixture.press(Key::Tab, NONE));
        let clicks_before_header = self.clicks.get();
        self.press(Key::space);
        assert!(
            !fixture.sidebar_toggle.is_active(),
            "{mode:?} Space activates the header toggle"
        );
        assert_eq!(
            (self.clicks.get(), fixture.selected()),
            (clicks_before_header, vec![0])
        );
        fixture.sidebar_toggle.set_active(true);
        clicks_before_header
    }

    fn a_hidden_sidebar_stays_hidden(&self, mode: BrowserMode) {
        let fixture = self.fixture;
        focus_files(fixture);
        fixture.sidebar_toggle.set_active(false);
        fixture.press(Key::b, SIDEBAR_CHORD);
        assert!(
            !fixture.sidebar_toggle.is_active(),
            "{mode:?} hidden sidebar stays hidden"
        );
        assert_eq!(
            (
                sidebar_has_focus(fixture),
                fixture.view.item_view_has_focus(),
                fixture.selected()
            ),
            (false, true, vec![0]),
            "{mode:?} (sidebar focused, files focused, selection)"
        );
        fixture.sidebar_toggle.set_active(true);
    }

    fn sidebar_keys_walk_the_device_row(&self, mode: BrowserMode, clicks_before: usize) {
        let fixture = self.fixture;
        focus_files(fixture);
        self.focus_sidebar();
        assert_eq!(
            fixture.selected(),
            [0],
            "{mode:?} focusing the sidebar keeps the selection"
        );
        assert!(self.at_origin());
        self.press(Key::j);
        assert!(self.place.has_focus(), "{mode:?} j moves to the device row");
        self.press(Key::Down);
        assert!(
            self.eject.has_focus(),
            "{mode:?} Down moves to the device control"
        );
        self.press(Key::Up);
        assert!(self.place.has_focus(), "{mode:?}");
        self.press(Key::k);
        assert!(
            !self.place.has_focus() && !self.eject.has_focus(),
            "{mode:?} k returns to the first place"
        );
        assert_eq!(
            (fixture.selected(), self.clicks.get()),
            (vec![0], clicks_before)
        );
        self.press(Key::Right);
        assert!(
            sidebar_has_focus(fixture),
            "{mode:?} Right does not leave or activate"
        );
        assert_eq!(self.clicks.get(), clicks_before);
    }

    fn only_the_device_control_activates(&self, mode: BrowserMode, clicks_before: usize) {
        let fixture = self.fixture;
        self.press(Key::j);
        self.press(Key::l);
        assert_eq!(
            self.clicks.get(),
            clicks_before,
            "{mode:?} l on the row is not the eject control"
        );
        assert!(self.at_origin());
        self.press(Key::j);
        assert!(self.eject.has_focus(), "{mode:?}");
        self.press(Key::space);
        assert_eq!(
            self.clicks.get(),
            clicks_before + 1,
            "{mode:?} Space activates the device control"
        );
        self.press(Key::Return);
        assert_eq!(self.clicks.get(), clicks_before + 2, "{mode:?}");
        assert!(
            sidebar_has_focus(fixture),
            "{mode:?} device activation keeps the sidebar"
        );
        assert_eq!(fixture.selected(), [0]);
    }

    fn back_keys_return_to_the_files(&self, mode: BrowserMode) {
        let fixture = self.fixture;
        self.press(Key::h);
        assert!(
            fixture.view.item_view_has_focus(),
            "{mode:?} h returns to the files"
        );
        assert_eq!(fixture.selected(), [0]);
        focus_files(fixture);
        for key in [Key::Left, Key::BackSpace] {
            self.focus_sidebar();
            self.press(key);
            assert!(fixture.view.item_view_has_focus(), "{mode:?}");
            assert_eq!(fixture.selected(), [0]);
        }
        assert!(self.at_origin());
    }

    fn an_empty_folder_round_trips(&self, mode: BrowserMode) {
        let fixture = self.fixture;
        let empty = fixture._directory.path().join(format!("empty-{mode:?}"));
        std::fs::create_dir(&empty).expect("empty directory");
        let browser = fixture.view.browser();
        browser.navigate(Location::local(&empty));
        wait_loaded(&browser, 0);
        assert!(fixture.selected().is_empty(), "{mode:?}");
        focus_files(fixture);
        self.focus_sidebar();
        self.press(Key::h);
        assert!(
            fixture.view.item_view_has_focus(),
            "{mode:?} empty directory round trip"
        );
        assert!(fixture.selected().is_empty(), "{mode:?}");
    }

    fn place_activation_focuses_the_new_listing(&self, mode: BrowserMode) {
        let fixture = self.fixture;
        let browser = fixture.view.browser();
        self.open_origin();
        self.focus_sidebar();
        self.press(Key::Return);
        wait_until(|| {
            !self.at_origin()
                && browser
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
        });
        assert!(
            fixture.view.item_view_has_focus(),
            "{mode:?} place activation focuses the new listing"
        );
        assert!(!sidebar_has_focus(fixture), "{mode:?}");
    }

    fn run(&self, mode: BrowserMode) {
        self.fixture.view.set_view_mode(mode);
        self.open_origin();
        self.header_keys_return_to_the_files(mode);
        let clicks_before = self.space_activates_the_header_toggle(mode);
        self.a_hidden_sidebar_stays_hidden(mode);
        self.sidebar_keys_walk_the_device_row(mode, clicks_before);
        self.only_the_device_control_activates(mode, clicks_before);
        self.back_keys_return_to_the_files(mode);
        self.an_empty_folder_round_trips(mode);
        self.place_activation_focuses_the_new_listing(mode);
    }
}

#[test]
fn tenxer_sidebar_and_header_round_trips() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_panes::tenxer_sidebar_and_header_round_trips",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            let round_trip = SidebarRoundTrip::with_device_row(&fixture);
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                round_trip.run(mode);
            }
        },
    );
}
