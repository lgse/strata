// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn dialog_copy_wraps_at_word_boundaries() {
    let wrapped = wrap_dialog_text(
        "Those credentials were not accepted. Check the username and password.",
        32,
    );
    assert_eq!(
        wrapped,
        "Those credentials were not\naccepted. Check the username and\npassword."
    );
    assert!(wrapped.lines().all(|line| line.chars().count() <= 32));
}

#[test]
fn dialog_copy_wraps_long_paths_without_spaces() {
    let wrapped = wrap_dialog_text(&"a".repeat(80), 32);
    assert_eq!(
        wrapped.lines().map(str::len).collect::<Vec<_>>(),
        [32, 32, 16]
    );
}
