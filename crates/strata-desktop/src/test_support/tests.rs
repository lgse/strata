// SPDX-License-Identifier: MIT

//! The guard on a test that runs in a process of its own: it has to have run there.

use super::in_own_process;

#[test]
#[should_panic(expected = "ran nowhere")]
fn a_test_whose_path_names_no_test_fails() {
    in_own_process("test_support::tests::a_path_no_test_has", || {});
}
