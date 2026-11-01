// SPDX-License-Identifier: MIT

use super::filter_change_for;
use gtk::FilterChange;

#[test]
fn filter_change_classifies_typo_eligibility_per_word() {
    for (previous, settled, expected) in [
        ("ab t", "ab tx", FilterChange::MoreStrict),
        ("ab tx", "ab t", FilterChange::LessStrict),
        ("abc", "abc ", FilterChange::MoreStrict),
        ("abc tex", "abc text", FilterChange::Different),
        ("abc text", "abc tex", FilterChange::Different),
        ("duck", "duck t", FilterChange::Different),
        ("ab cd", "*.md", FilterChange::Different),
        ("abc", "xyz", FilterChange::Different),
    ] {
        assert_eq!(
            filter_change_for(previous, settled),
            expected,
            "previous={previous:?}, settled={settled:?}",
        );
    }
}
