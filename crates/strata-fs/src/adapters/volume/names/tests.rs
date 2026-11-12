// SPDX-License-Identifier: MIT

use super::{NameProfile, gvfs_profile};

#[test]
fn gvfs_backends_without_known_rules_learn_them_and_have_no_limit() {
    assert_eq!(gvfs_profile("dav"), NameProfile::posix(None).learning());
}
