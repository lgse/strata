// SPDX-License-Identifier: MIT

use rune::{Key, Mods};
use strata_core::model::Location;

use crate::{
    application::{self, Opening},
    commands::{On, Outcome, run},
    shell,
    test_support::{
        PATIENCE, fixture, hosted,
        view::{idle, press, shift},
    },
};

const MODULE: &str = "new_window::tests";

/// As GTK's `open_focused_in_new` sends `open-window-at`: Shift+Enter on the cursor's folder
/// asks the process for a window on it and leaves this window where it was; the command on a
/// place, as a crumb's or a row's middle click runs it, asks for the place.
#[test]
fn shift_enter_asks_for_a_window_on_the_cursors_folder() {
    hosted(
        MODULE,
        "shift_enter_asks_for_a_window_on_the_cursors_folder",
        |mut host| {
            let folder = fixture();
            let here = Location::local(folder.path());
            host.app().open(here.clone());
            let listed = |host: &crate::host::offscreen::Offscreen| {
                host.app().browser().column_entry_counts(0).is_some()
            };
            assert!(host.run_until(PATIENCE, listed), "the folder's listing");
            idle(&mut host);
            let view = shell::view_focus(host.app()).expect("the view's key");
            host.runtime().update(move |_, tx| tx.focus(view, true));
            idle(&mut host);
            let _ = application::take();

            press(&mut host, Key::Down, Mods::default());
            let cursor = host
                .app()
                .browser()
                .focused_entry()
                .expect("a cursor entry");
            assert!(cursor.is_directory(), "{} is a folder", cursor.display_name);
            press(&mut host, Key::Enter, shift());

            assert_eq!(application::take(), [Opening::Argument(cursor.location)]);
            assert_eq!(host.app().location(), Some(&here), "this window stays");

            let place = Location::local(folder.path().join("archive"));
            let mut ran = Outcome::Declined;
            let on = place.clone();
            host.runtime()
                .update(|app, tx| ran = run(app, "open-in-window", On::Place(on), tx));
            assert_eq!(ran, Outcome::Ran);
            assert_eq!(application::take(), [Opening::Argument(place)]);
        },
    );
}

/// GTK's `open-window-at` and `open-tab-at` are a browser window's actions: a chooser's window
/// has neither, so a Ctrl or Shift middle click on one of its folders, or its menu, opens
/// nothing. Here the floating Copy to chooser the entry menu asks for.
#[test]
fn a_chooser_opens_no_window_and_no_tab() {
    use strata_platform::csd::{Backend, Csd, Policy};

    use crate::{App, chooser::floating, host::offscreen::Offscreen, portal::Completion};

    hosted(
        MODULE,
        "a_chooser_opens_no_window_and_no_tab",
        |mut host| {
            let folder = fixture();
            host.app().open(Location::local(folder.path()));
            let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, listed), "the folder's listing");
            idle(&mut host);
            let notes = Location::local(folder.path().join("todo.txt"));
            let id = shell::id_of(host.app(), &notes);
            host.runtime().update(move |app, tx| {
                assert_eq!(run(app, "copy-to", On::Row((0, id)), tx), Outcome::Ran);
            });
            idle(&mut host);
            let request = floating::take(host.app_mut()).expect("a chooser");
            let completion: Completion = Box::new(|_| {});
            let chooser = App::new_chooser(
                &host.ui(),
                Csd::new(Policy::Auto.frame(), Backend::Headless),
                shell::share_settings(host.app()),
                (
                    request.request,
                    request.source,
                    Some(request.destination),
                    completion,
                ),
            );
            let mut chooser = Offscreen::from_app(host.ui(), chooser, (900.0, 600.0));
            chooser.app().open(Location::local(folder.path()));
            assert!(chooser.run_until(PATIENCE, listed), "the chooser's listing");
            let _ = application::take();

            let archive = Location::local(folder.path().join("archive"));
            for command in ["open-in-window", "open-in-tab"] {
                let mut ran = Outcome::Ran;
                let on = archive.clone();
                chooser
                    .runtime()
                    .update(|app, tx| ran = run(app, command, On::Place(on), tx));
                assert_eq!(ran, Outcome::Declined, "{command}");
            }
            idle(&mut chooser);
            assert_eq!(application::take(), []);
            assert_eq!(
                crate::shell::tabs::tabs(chooser.app()).len(),
                1,
                "no tab opened"
            );
        },
    );
}
