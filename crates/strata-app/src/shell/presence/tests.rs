// SPDX-License-Identifier: MIT

use strata_core::model::Location;
use strata_fs::app::Presence;

use crate::{
    App,
    host::offscreen::Offscreen,
    shell::tabs::{tab_browser, tabs},
    tab_strip,
    test_support::{PATIENCE, folder_of, hosted},
};

const MODULE: &str = "shell::presence::tests";

/// Every tab's browser's presence, shown tab first.
fn presences(app: &App) -> Vec<Presence> {
    tabs(app)
        .iter()
        .filter_map(|&key| tab_browser(app, key))
        .map(|browser| browser.freshness_input().presence)
        .collect()
}

fn settle(host: &mut Offscreen) {
    host.run_until(PATIENCE, |_| true);
    while host.turn() {}
}

#[test]
fn every_tab_follows_the_window_s_focus_and_visibility() {
    hosted(
        MODULE,
        "every_tab_follows_the_window_s_focus_and_visibility",
        |mut host| {
            let folder = folder_of(&["one.txt"]);
            let location = Location::local(folder.path());
            host.app().open(location.clone());
            let opened = location.clone();
            host.runtime()
                .update(move |app, tx| tab_strip::open_in_new_tab(app, opened, tx));
            settle(&mut host);
            assert_eq!(presences(host.app()), vec![Presence::Focused; 2]);

            host.runtime().window_focus(false);
            settle(&mut host);
            assert_eq!(presences(host.app()), vec![Presence::Visible; 2]);

            let opened = location.clone();
            host.runtime()
                .update(move |app, tx| tab_strip::open_in_new_tab(app, opened, tx));
            settle(&mut host);
            assert_eq!(presences(host.app()), vec![Presence::Visible; 3]);

            host.runtime()
                .update(|app, tx| super::window_shown(app, false, tx));
            settle(&mut host);
            assert_eq!(presences(host.app()), vec![Presence::Hidden; 3]);

            host.runtime()
                .update(|app, tx| super::window_shown(app, true, tx));
            host.runtime().window_focus(true);
            settle(&mut host);
            assert_eq!(presences(host.app()), vec![Presence::Focused; 3]);
        },
    );
}

#[test]
fn a_reopened_window_has_the_keyboard_only_once_rune_gives_it() {
    use crate::startup::{self, UiThread};
    use strata_platform::csd::{Backend, Csd, Policy};

    crate::test_support::in_own_process(
        &format!("{MODULE}::a_reopened_window_has_the_keyboard_only_once_rune_gives_it"),
        &[],
        || {
            let folder = folder_of(&["one.txt"]);
            let ui = UiThread::claim().expect("the UI thread");
            let mut host = Offscreen::new(ui, startup::load_settings());
            host.app().open(Location::local(folder.path()));
            host.runtime().window_focus(true);
            settle(&mut host);
            assert_eq!(presences(host.app()), vec![Presence::Focused]);

            let csd = Csd::new(Policy::Auto.frame(), Backend::from_env());
            let stand_in = App::new(&ui, csd, crate::shell::share_settings(host.app()));
            let mut kept = std::mem::replace(host.app_mut(), stand_in);
            drop(host);
            crate::shell::reopened(&mut kept);
            assert_eq!(presences(&kept), vec![Presence::Visible]);

            let mut reopened = Offscreen::from_app(ui, kept, (1200.0, 760.0));
            settle(&mut reopened);
            reopened.runtime().window_focus(true);
            settle(&mut reopened);
            assert_eq!(presences(reopened.app()), vec![Presence::Focused]);
        },
    );
}
