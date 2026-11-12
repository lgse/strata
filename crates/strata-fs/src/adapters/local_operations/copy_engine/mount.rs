// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy)]
pub(super) struct Mount {
    pub(super) key: MountKey,
    pub(super) family: Family,
    pub(super) identity: IdentityTraits,
    pub(super) fuse_format: Option<FuseFormat>,
    pub(super) coarse_times: Option<CoarseTimes>,
    pub(super) deferred_close: Option<Duration>,
    pub(super) attribute_cache: Option<Duration>,
    pub(super) names: Option<MountNames>,
}

impl Mount {
    pub(super) fn new(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> Self {
        let (key, capabilities) = known_mount(fs, dir);
        Self::with(key, capabilities)
    }

    pub(super) fn with(key: MountKey, capabilities: MountCapabilities) -> Self {
        Self {
            key,
            family: capabilities.family.unwrap_or(Family::Other),
            identity: capabilities.identity,
            fuse_format: capabilities.fuse_format,
            coarse_times: capabilities.coarse_times,
            deferred_close: capabilities.deferred_close,
            attribute_cache: capabilities.attribute_cache,
            names: capabilities.names,
        }
    }

    /// The SMB client answers stats from attributes it cached, whose change time can predate
    /// one the server set since, such as once the deferred close of a file written there lands.
    pub(super) fn caches_attributes(&self) -> bool {
        self.attribute_cache.is_some()
    }

    pub(super) fn identity_now(&self, fs: &dyn FsOps, dir: BorrowedFd<'_>) -> IdentityTraits {
        mount_profile::identity_now(fs, dir, self.family, self.identity)
    }

    pub(super) fn may_be(&self, recorded: LocalFileIdentity, current: &EntryStat) -> bool {
        compare(self.identity, recorded, None, current) != Proof::Different
    }

    pub(super) fn capabilities(&self) -> MountCapabilities {
        mount_capabilities(self.key)
    }

    pub(super) fn learn(&self, learn: impl FnOnce(&mut MountCapabilities)) {
        learn_mount_capability(self.key, learn);
    }
}

pub(super) fn link_tmpfile(
    fs: &dyn FsOps,
    mount: &Mount,
    fd: BorrowedFd<'_>,
    to: Entry<'_>,
) -> Result<(), Errno> {
    let through_proc = mount.capabilities().tmpfile_link_through_proc;
    if through_proc != Some(true) {
        match fs.link_fd(fd, to, false) {
            Ok(()) => {
                if through_proc.is_none() {
                    mount
                        .learn(|capabilities| capabilities.tmpfile_link_through_proc = Some(false));
                }
                return Ok(());
            }
            // Once the plain form has worked, ENOENT is about the folder.
            Err(Errno::NOENT) if through_proc.is_none() => {}
            Err(error) => return Err(error),
        }
    }
    match fs.link_fd(fd, to, true) {
        Ok(()) => {
            if through_proc.is_none() {
                mount.learn(|capabilities| capabilities.tmpfile_link_through_proc = Some(true));
            }
            Ok(())
        }
        // Unless the folder itself was removed meanwhile, or the mount may
        // refuse the name, as SMB and exfat-fuse do with `ENOENT`.
        Err(Errno::NOENT)
            if through_proc.is_none()
                && !may_be_refused(to.name)
                && fs.stat_fd(to.dir).is_ok_and(|folder| folder.nlink > 0) =>
        {
            mount.learn(|capabilities| capabilities.tmpfile = Some(false));
            Err(Errno::NOENT)
        }
        Err(error) => Err(error),
    }
}
