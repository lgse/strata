// SPDX-License-Identifier: MIT

//! What each mount was seen to support. Entries order fallback rungs and
//! interpret ambiguous errors; every ladder rung refuses to replace on its own,
//! so a stale entry costs speed or a refusal. The exception is the
//! reserved-name rung, which an entry alone enables, with its small window.

use std::{collections::BTreeMap, ffi::OsStr, sync::Mutex, time::Duration};

use super::{CoarseTimes, Family, FuseFormat, MountNames, SillyNames, Strictness};
use crate::services::IdentityTraits;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum MountKey {
    Unique(u64),
    Mount {
        id: u64,
        device: u64,
    },
    Device(u64),
    /// A GVfs FUSE bridge backend, keyed by the bridge and the backend's folder.
    Backend(u64),
    #[cfg(test)]
    Test(u64),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MountCapabilities {
    pub no_replace: Option<bool>,
    pub exchange: Option<bool>,
    pub hard_links: Option<bool>,
    pub links_keep_numbers: bool,
    pub rename_over_empty_directory: Option<bool>,
    pub tmpfile: Option<bool>,
    pub tmpfile_link_through_proc: Option<bool>,
    pub reflink: Option<bool>,
    pub copy_range: Option<bool>,
    /// Namespaces the mount refuses as unsupported; a refused name alone
    /// says nothing about its namespace.
    pub refused_xattrs: XattrNamespaces,
    /// From mountinfo, kept here so a mount change drops it with the rest.
    pub family: Option<Family>,
    pub identity: IdentityTraits,
    pub fuse_format: Option<FuseFormat>,
    pub coarse_times: Option<CoarseTimes>,
    pub deferred_close: Option<Duration>,
    pub silly_names: Option<SillyNames>,
    pub attribute_cache: Option<Duration>,
    pub cuts_long_names_renamed_over: bool,
    pub names: Option<MountNames>,
    pub strictness: Strictness,
    pub symlinks: Option<bool>,
    pub max_file_size: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XattrNamespace {
    User,
    Acl,
    Security,
    Trusted,
    Other,
}

impl XattrNamespace {
    pub(crate) fn of(name: &OsStr) -> Self {
        let name = name.as_encoded_bytes();
        if name.starts_with(b"user.") {
            Self::User
        } else if name.starts_with(b"system.posix_acl_") {
            Self::Acl
        } else if name.starts_with(b"security.") {
            Self::Security
        } else if name.starts_with(b"trusted.") {
            Self::Trusted
        } else {
            Self::Other
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct XattrNamespaces(u8);

impl XattrNamespaces {
    pub(crate) fn contains(self, namespace: XattrNamespace) -> bool {
        self.0 & namespace.bit() != 0
    }

    pub(crate) fn insert(&mut self, namespace: XattrNamespace) {
        self.0 |= namespace.bit();
    }
}

static CAPABILITIES: Mutex<BTreeMap<MountKey, MountCapabilities>> = Mutex::new(BTreeMap::new());

fn capabilities() -> std::sync::MutexGuard<'static, BTreeMap<MountKey, MountCapabilities>> {
    CAPABILITIES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub(crate) fn mount_capabilities(key: MountKey) -> MountCapabilities {
    capabilities().get(&key).copied().unwrap_or_default()
}

pub(crate) fn learn_mount_capability(key: MountKey, learn: impl FnOnce(&mut MountCapabilities)) {
    learn(capabilities().entry(key).or_default());
}

pub(crate) fn teach_mount_capability(
    mount: MountKey,
    key: MountKey,
    learn: impl FnOnce(&mut MountCapabilities),
) {
    let mut capabilities = capabilities();
    if capabilities
        .get(&mount)
        .is_some_and(|capabilities| capabilities.names.is_some())
    {
        learn(capabilities.entry(key).or_default());
    }
}

/// Mount ids can be reused once a mount goes away, so a mount change drops
/// everything learned from real mounts.
pub(crate) fn forget_mount_capabilities() {
    capabilities().retain(|key, _| is_test_key(key));
}

#[cfg(test)]
fn is_test_key(key: &MountKey) -> bool {
    matches!(key, MountKey::Test(_))
}

#[cfg(not(test))]
fn is_test_key(_: &MountKey) -> bool {
    false
}

#[cfg(test)]
pub(crate) fn forget_test_mount_capabilities(id: u64) {
    capabilities().remove(&MountKey::Test(id));
}
