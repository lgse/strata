// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use strata_core::model::Location;
use strata_qa::{Backend as _, Outcome, Remembered, execute, parse_request};

use super::{SIZE, harness_previews_off, now};
use crate::{
    application::{self, Opening},
    host::offscreen::Offscreen,
    qa::{backend::Rune, input_log::InputLog},
    startup::{self, UiThread},
    test_support::{fixture, in_own_process},
};

fn passes(backend: &Rune, kept: &Remembered<Rune>, step: &Value) {
    let command = parse_request(&step.to_string()).expect("a step the protocol knows");
    let outcome: Outcome = now(execute(backend, kept, command));
    assert!(
        outcome.error.is_none() && outcome.failures.is_empty(),
        "{step} came to {outcome:?}"
    );
}

#[test]
fn a_launch_handed_over_opens_a_window_the_steps_reach() {
    in_own_process(
        "qa::tests::windows::a_launch_handed_over_opens_a_window_the_steps_reach",
        &[],
        || {
            harness_previews_off();
            let folder = fixture();
            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let host = Offscreen::sized(ui, settings, SIZE).without_animations();
            host.app().open(Location::local(folder.path()));
            let log = InputLog::from_environment().expect("the input log");
            let backend = Rune::new(host, log);
            now(backend.ready()).expect("the first listing");
            let documents = Location::local(folder.path().join("documents"));
            application::queue_first([Opening::Argument(documents)]);
            let kept = Remembered::default();

            for step in [
                json!({ "wait": { "windows": { "title": "Strata", "count": 2 } } }),
                json!({ "wait": { "windows": { "title": "documents — Strata", "count": 1 } } }),
                json!({ "wait": { "present": { "role": "list item", "name": "notes.txt", "within": { "other_window": "Strata" } } } }),
                json!({ "expect": { "absent": { "role": "list item", "name": "notes.txt" } } }),
                json!({ "activate": { "role": "button", "name": "Settings", "within": { "other_window": "Strata" } } }),
                json!({ "wait": { "present": { "role": "dialog", "name": "Settings", "within": { "other_window": "Strata" } } } }),
                json!({ "expect": { "absent": { "role": "dialog", "name": "Settings" } } }),
            ] {
                passes(&backend, &kept, &step);
            }
        },
    );
}
