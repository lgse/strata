// SPDX-License-Identifier: MIT

use super::luks_password_lookups;

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
