// SPDX-License-Identifier: MIT

use super::{
    ForgetCachedPasswordError, delete_error_is_item_locked, forget_failure_from_delete_prompt,
    forget_failure_from_search, luks_password_lookups,
};

#[test]
fn luks_password_lookups_cover_gvfs_and_disks() {
    let lookups = luks_password_lookups("6E5D75A7-E4E2-4C7D-9C1C-8E5A5E5D75A7");
    assert_eq!(
        lookups,
        [
            (
                "gvfs-luks-uuid",
                "6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7".into()
            ),
            ("gvfs-luks-uuid", "6e5d75a7e4e24c7d9c1c8e5a5e5d75a7".into()),
            (
                "gvfs.crypto.luks.uuid",
                "6e5d75a7-e4e2-4c7d-9c1c-8e5a5e5d75a7".into()
            ),
            (
                "gvfs.crypto.luks.uuid",
                "6e5d75a7e4e24c7d9c1c8e5a5e5d75a7".into()
            ),
        ]
    );
    assert!(luks_password_lookups("").is_empty());
    assert!(luks_password_lookups("not-a-uuid").is_empty());
    assert!(luks_password_lookups("/dev/mapper/luks").is_empty());
}

#[test]
fn forget_delete_prompt_is_not_success() {
    assert_eq!(forget_failure_from_delete_prompt("/"), Ok(()));
    assert_eq!(
        forget_failure_from_delete_prompt("/org/freedesktop/secrets/prompt/p1"),
        Err(ForgetCachedPasswordError::NeedsConfirmation)
    );
}

#[test]
fn forget_locked_search_items_are_not_success() {
    assert_eq!(forget_failure_from_search(0), Ok(()));
    assert_eq!(
        forget_failure_from_search(1),
        Err(ForgetCachedPasswordError::ItemLocked)
    );
    assert!(delete_error_is_item_locked(
        "org.freedesktop.Secret.Error.IsLocked"
    ));
    assert!(!delete_error_is_item_locked(
        "org.freedesktop.DBus.Error.UnknownMethod"
    ));
}
