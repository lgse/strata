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
