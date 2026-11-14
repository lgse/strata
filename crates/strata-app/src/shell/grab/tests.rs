// SPDX-License-Identifier: MIT

//! A modal layer over the List view ends its click run, on the window the crate registers:
//! what `executable-requires-confirmation` does with its second double click.

use std::{cell::Cell, rc::Rc};

use rune::{Key, KeyEvent, Mods};
use strata_fs::app::BrowserEvent;

use crate::{
    App,
    error_dialog::{self, Error},
    host::offscreen::Offscreen,
    test_support::{
        folder_of, send_key,
        view::{Shape, double_click, hosted_with, open},
    },
};

const MODULE: &str = "shell::grab::tests";

const LIST: Shape = Shape {
    described: "List view",
    list: "list",
    entry: "list-item",
};

fn turns(host: &mut Offscreen) {
    for _ in 0..3 {
        host.turn();
    }
}

/// Finding lgse/strata#2454: a double click on a file, a dialog over the view, and a second
/// double click within the double-click interval of the first, which opens the file again.
#[test]
fn a_double_click_after_a_dialog_closes_opens_again() {
    hosted_with(
        MODULE,
        "list",
        String::new(),
        "a_double_click_after_a_dialog_closes_opens_again",
        |mut host| {
            let folder = folder_of(&["todo.txt"]);
            open(&mut host, LIST, folder.path());
            let opened = Rc::new(Cell::new(0));
            let heard = opened.clone();
            host.app().browser().observe(move |event| {
                if matches!(event, BrowserEvent::OpenRequested { .. }) {
                    heard.set(heard.get() + 1);
                }
            });

            double_click(&mut host, LIST, "todo.txt");
            assert_eq!(opened.get(), 1, "the first double click opens todo.txt");

            host.runtime().update(|app: &mut App, _| {
                error_dialog::show(app, Error::new("Failed", "Why it failed"));
            });
            // A few frames, not the dialog's whole entrance: the second double click has to
            // come within the double-click interval of the first, as in the scenario.
            turns(&mut host);
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            turns(&mut host);

            double_click(&mut host, LIST, "todo.txt");
            assert_eq!(opened.get(), 2, "the second double click opens it again");
        },
    );
}
