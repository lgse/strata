// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn properties_permissions_are_formatted_symbolically_and_numerically() {
    assert_eq!(format_permissions(0o100774), "-rwxrwxr--  774");
    assert_eq!(format_permissions(0o040755), "drwxr-xr-x  755");
}

#[test]
fn individual_permission_bits_can_be_toggled_without_changing_file_type() {
    assert_eq!(toggled_permission(0o100644, 0o100), 0o100744);
    assert_eq!(toggled_permission(0o100744, 0o100), 0o100644);
}

#[test]
fn executable_toggle_changes_all_execute_bits_and_preserves_other_bits() {
    assert_eq!(with_execute_permissions(0o100644, true), 0o100755);
    assert_eq!(with_execute_permissions(0o100775, false), 0o100664);
}
