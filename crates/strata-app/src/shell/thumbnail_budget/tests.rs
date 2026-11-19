// SPDX-License-Identifier: MIT

use strata_components::thumbnails::Settings;

use super::budget;
use crate::{
    host::offscreen::Offscreen,
    shell,
    startup::{self, UiThread},
    test_support::in_own_process,
};

/// Lets each window apply what its queue holds.
fn settle(hosts: &mut [&mut Offscreen]) {
    for host in hosts {
        while host.turn() {}
    }
}

/// GTK's one cache of 64 MiB per process: the open windows split it, and a window that goes
/// gives its share back to the others.
#[test]
fn the_windows_split_the_process_s_thumbnail_budget() {
    in_own_process(
        "shell::thumbnail_budget::tests::the_windows_split_the_process_s_thumbnail_budget",
        &[],
        || {
            let whole = Settings::default().budget_bytes;
            let ui = UiThread::claim().expect("the UI thread");
            let mut first = Offscreen::new(ui, startup::load_settings());
            assert_eq!(budget(first.app()), whole);

            let mut second = Offscreen::new(ui, shell::share_settings(first.app()));
            let mut third = Offscreen::new(ui, shell::share_settings(first.app()));
            settle(&mut [&mut first, &mut second, &mut third]);
            let budgets = [first.app(), second.app(), third.app()].map(budget);
            assert_eq!(budgets, [whole / 3; 3]);

            drop(third);
            settle(&mut [&mut first, &mut second]);
            assert_eq!([first.app(), second.app()].map(budget), [whole / 2; 2]);

            drop(second);
            settle(&mut [&mut first]);
            assert_eq!(budget(first.app()), whole);
        },
    );
}
