// SPDX-License-Identifier: MIT

//! The guard on a test that runs in a process of its own: it has to have run there.

use super::{hosted, in_own_process};

/// A test whose path is written wrong starts a child that runs nothing and exits well
/// (finding lgse/strata#2193): the helper fails it.
#[test]
#[should_panic(expected = "ran nowhere")]
fn a_test_whose_path_names_no_test_fails() {
    in_own_process("test_support::tests::a_path_no_test_has", &[], || {});
}

/// The same through the hosted helper, which every part's tests use: a test that moved into
/// a module its name string does not follow.
#[test]
#[should_panic(expected = "ran nowhere")]
fn a_hosted_test_whose_module_is_written_wrong_fails() {
    hosted(
        "test_support",
        "a_hosted_test_whose_module_is_written_wrong_fails",
        |_| {},
    );
}

#[test]
fn a_test_run_in_its_own_process_runs_there() {
    let test = "test_support::tests::a_test_run_in_its_own_process_runs_there";
    let mut ran = false;
    in_own_process(test, &[], || ran = true);
    // The child ran the body; the parent only started it.
    assert_eq!(ran, std::env::var_os(super::RAN).is_some());
}
