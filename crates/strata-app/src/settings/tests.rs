// SPDX-License-Identifier: MIT

use strata_desktop::settings::{Preferences, settings_path};

use crate::{
    host::offscreen::Offscreen,
    shell,
    startup::{self, UiThread},
    test_support::{PATIENCE, in_own_process},
};

const MODULE: &str = "settings::tests";

fn change(host: &mut Offscreen, change: impl FnOnce(&mut Preferences) + 'static) {
    host.runtime()
        .update(move |app, _| shell::saved(app).change(change));
}

fn reads(host: &mut Offscreen, wanted: impl Fn(&Preferences) -> bool) -> bool {
    host.run_until(PATIENCE, |host| wanted(shell::preferences(host.app())))
}

/// As every GTK window binds to the one `PreferenceManager`: a change made in either window is
/// the other's once its queue runs, neither undoes the other's, and the file holds both.
#[test]
fn the_windows_of_a_process_share_their_settings_and_their_saves() {
    in_own_process(
        &format!("{MODULE}::the_windows_of_a_process_share_their_settings_and_their_saves"),
        &[],
        || {
            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let mut first = Offscreen::new(ui, settings);
            let mut second = Offscreen::new(ui, shell::share_settings(first.app()));
            let peeking = !shell::preferences(first.app()).folder_peeking;
            let searching = !shell::preferences(first.app()).type_to_search;

            change(&mut first, move |saved| saved.folder_peeking = peeking);
            assert!(
                reads(&mut second, |saved| saved.folder_peeking == peeking),
                "the second window never read the first window's change"
            );
            change(&mut second, move |saved| saved.type_to_search = searching);
            assert!(
                reads(&mut first, |saved| saved.type_to_search == searching
                    && saved.folder_peeking == peeking),
                "the first window never read the second window's change"
            );

            strata_core::storage::writer::flush_at_exit();
            let text = std::fs::read_to_string(settings_path()).expect("the saved settings");
            let saved: Preferences = toml::from_str(&text).expect("settings the file holds");
            assert_eq!(
                (saved.folder_peeking, saved.type_to_search),
                (peeking, searching)
            );
        },
    );
}

/// The store holds no way into a window that has closed, and tells it nothing.
#[test]
fn a_closed_window_is_told_nothing() {
    in_own_process(
        &format!("{MODULE}::a_closed_window_is_told_nothing"),
        &[],
        || {
            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let mut first = Offscreen::new(ui, settings);
            let second = Offscreen::new(ui, shell::share_settings(first.app()));
            let mut told = 0;
            first
                .runtime()
                .update(|app, _| told = shell::saved(app).windows_told());
            assert_eq!(told, 2);
            drop(second);
            let peeking = !shell::preferences(first.app()).folder_peeking;

            change(&mut first, move |saved| saved.folder_peeking = peeking);

            assert!(reads(&mut first, |saved| saved.folder_peeking == peeking));
            first
                .runtime()
                .update(|app, _| told = shell::saved(app).windows_told());
            assert_eq!(told, 1);
        },
    );
}
