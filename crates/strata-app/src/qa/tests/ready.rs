// SPDX-License-Identifier: MIT

//! The window a run starts in: the view the run's mode names, or a refusal before any step.

use strata_core::storage;
use strata_qa::Backend as _;

use super::{now, staged_after};
use crate::test_support::fixture;

/// Seven Icons runs once passed on the Columns view, which the shell shows when no view
/// answers to the saved mode (the review of #2192). `ready` refuses that start.
#[test]
fn a_run_whose_mode_no_view_answers_to_is_refused_at_ready() {
    staged_after(
        "ready::a_run_whose_mode_no_view_answers_to_is_refused_at_ready",
        None,
        || {
            let config = storage::config_directory();
            std::fs::create_dir_all(&config).expect("the config folder");
            std::fs::write(
                config.join("settings.toml"),
                "browser_mode = \"nonesuch\"\n",
            )
            .expect("the settings");
        },
        |backend| {
            let folder = fixture();
            backend.open(folder.path());
            let refused = now(backend.ready()).expect_err("no view answers to the mode");
            assert!(
                refused.contains("\"nonesuch\"") && refused.contains("Columns"),
                "{refused}"
            );
        },
    );
}
