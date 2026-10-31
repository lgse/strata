// SPDX-License-Identifier: MIT

//! What the mount behind a folder says about itself: learned once per mount from mountinfo,
//! udev and `statfs`, and read once per operation.

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    ffi::OsStr,
    hash::{DefaultHasher, Hash, Hasher},
    os::fd::{AsFd, BorrowedFd},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use gtk::{gio, glib, prelude::*};

use super::{
    await_cancellable,
    fs_ops::{FsOps, fs},
    io_error,
    native_fs::{open_local_parent_directory, opened_path, os_error_detail},
    run_blocking,
};
use crate::{
    adapters::volume::{
        Family, FuseFormat, MountCapabilities, MountKey, MountNames, MountType, NameEncoding,
        NameLimit, NameProfile, Strictness, family, gvfs_bridge_profile, gvfs_profile,
        holds_symlinks, learn_mount_capability, max_file_size, mount_capabilities,
        teach_mount_capability,
    },
    services::{IdentityTraits, InodeNumbers, NameMode},
};

pub(super) fn known_mount(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> (MountKey, MountCapabilities) {
    let key = fs.mount_key(dir);
    (key, known(fs, dir, key))
}

pub(super) fn known(fs: &dyn FsOps, dir: BorrowedFd<'_>, key: MountKey) -> MountCapabilities {
    let capabilities = mount_capabilities(key);
    if capabilities.family.is_some() {
        return capabilities;
    }
    learn_mount_type(fs, dir, key)
}

pub(super) fn identity_now(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    family: Family,
    identity: IdentityTraits,
) -> IdentityTraits {
    if family == Family::Smb
        && fs
            .current_mount_type(dir)
            .is_some_and(|mount| mount.server_numbers())
    {
        return IdentityTraits {
            numbers: InodeNumbers::Kept,
            ..identity
        };
    }
    identity
}

pub(super) fn identity_at(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> IdentityTraits {
    let (_, capabilities) = known_mount(fs, dir);
    let family = capabilities.family.unwrap_or(Family::Other);
    identity_now(fs, dir, family, capabilities.identity)
}

pub(super) fn with_links(
    family: Option<Family>,
    identity: IdentityTraits,
    links_keep_numbers: bool,
) -> IdentityTraits {
    if links_keep_numbers
        && family == Some(Family::Fuse)
        && identity.numbers == InodeNumbers::Renewed
    {
        return IdentityTraits {
            numbers: InodeNumbers::Kept,
            ..identity
        };
    }
    identity
}

/// For `fuseblk`, udev's record of the device tells which daemon serves it. What another
/// thread learned of the mount meanwhile stays.
fn learn_mount_type(fs: &dyn FsOps, dir: BorrowedFd<'_>, key: MountKey) -> MountCapabilities {
    let mount = fs.mount_type(dir);
    let mount_type = mount.as_ref().map(|(mount_type, _)| mount_type);
    let family = mount_type.map_or(Family::Other, |mount| family(&mount.fs_type));
    let identity = mount_type.map_or_else(IdentityTraits::default, MountType::identity);
    let device_format = mount_type
        .filter(|mount| mount.fs_type == "fuseblk")
        .and_then(|_| fs.device_format(dir).ok().flatten());
    let format = device_format.as_deref();
    let fuse_format = format.and_then(FuseFormat::of);
    let identity = IdentityTraits {
        folder_births: identity.folder_births && fuse_format != Some(FuseFormat::Exfat),
        ..identity
    };
    let coarse_times = mount_type.and_then(|mount| mount.coarse_times(fuse_format));
    let deferred_close = mount_type.and_then(MountType::deferred_close);
    let silly_names = mount_type.and_then(MountType::silly_names);
    let attribute_cache = mount_type.and_then(MountType::attribute_cache);
    let cuts_long_names_renamed_over =
        mount_type.is_some_and(|mount| mount.cuts_long_names_renamed_over(format));
    let names = match &mount {
        Some((mount_type, site)) if mount_type.is_gvfs_bridge() => match folders(&site.root).next()
        {
            // A bind mount of one backend's folder, or of a folder inside it.
            Some(backend) => MountNames::Profile(gvfs_bridge_profile(Some(backend))),
            None => MountNames::PerBackend {
                depth: u16::try_from(folders(&site.point).count()).unwrap_or(u16::MAX),
            },
        },
        _ => {
            let name_max = fs.statfs(dir).map_or(0, |stat| stat.name_max);
            MountNames::Profile(mount_type.map_or_else(
                || NameProfile::unknown(name_max),
                |mount| mount.name_profile(format, name_max),
            ))
        }
    };
    let symlinks = mount_type.and_then(|mount| holds_symlinks(&mount.fs_type, format));
    let max_file_size = mount_type.and_then(|mount| max_file_size(&mount.fs_type, format));
    let mut learned = MountCapabilities::default();
    learn_mount_capability(key, |capabilities| {
        if capabilities.family.is_none() {
            capabilities.family = Some(family);
            capabilities.identity =
                with_links(Some(family), identity, capabilities.links_keep_numbers);
            capabilities.fuse_format = fuse_format;
            capabilities.coarse_times = coarse_times;
            capabilities.deferred_close = deferred_close;
            capabilities.silly_names = silly_names;
            capabilities.attribute_cache = attribute_cache;
            capabilities.cuts_long_names_renamed_over = cuts_long_names_renamed_over;
            capabilities.names = Some(names);
            capabilities.symlinks = symlinks;
            capabilities.max_file_size = max_file_size;
        }
        learned = *capabilities;
    });
    learned
}

fn folders(path: &Path) -> impl Iterator<Item = &OsStr> {
    path.components().filter_map(|component| match component {
        Component::Normal(folder) => Some(folder),
        _ => None,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MountProfile {
    pub names: NameProfile,
    pub strictness: Strictness,
    pub symlinks: Option<bool>,
    pub max_file_size: Option<u64>,
    pub family: Family,
    pub fuse_format: Option<FuseFormat>,
}

impl MountProfile {
    fn of(seen: MountCapabilities, site: &Site, taught: MountCapabilities) -> Self {
        Self {
            names: site.rules,
            strictness: taught.strictness,
            symlinks: taught.symlinks,
            max_file_size: seen.max_file_size,
            family: seen.family.unwrap_or(Family::Other),
            fuse_format: seen.fuse_format,
        }
    }

    fn unknown() -> Self {
        Self {
            names: NameProfile::unknown(0),
            strictness: Strictness::default(),
            symlinks: None,
            max_file_size: None,
            family: Family::Other,
            fuse_format: None,
        }
    }

    pub(super) fn gvfs(scheme: &str) -> Self {
        Self {
            names: gvfs_profile(scheme),
            strictness: Strictness::default(),
            symlinks: None,
            max_file_size: None,
            family: Family::Other,
            fuse_format: None,
        }
    }

    pub(super) fn windows_disk(&self) -> bool {
        matches!(self.family, Family::Fat | Family::Ntfs3) || self.fuse_format.is_some()
    }
}

/// Each mount is read once per operation, so its planner and workers agree even when a mount
/// change drops what was learned.
#[derive(Clone, Default)]
pub(super) struct Profiles(Arc<Mutex<HashMap<MountKey, MountCapabilities>>>);

impl Profiles {
    pub(super) fn mount(
        &self,
        fs: &dyn FsOps,
        dir: BorrowedFd<'_>,
    ) -> (MountKey, MountCapabilities) {
        let key = fs.mount_key(dir);
        let seen = self.seen().get(&key).copied();
        let seen = seen.unwrap_or_else(|| {
            let capabilities = known(fs, dir, key);
            *self.seen().entry(key).or_insert(capabilities)
        });
        (key, seen)
    }

    #[cfg(test)]
    pub(super) fn of(&self, fs: &dyn FsOps, dir: BorrowedFd<'_>) -> MountProfile {
        let (key, seen) = self.mount(fs, dir);
        let site = self.site(key, seen.names, dir);
        MountProfile::of(seen, &site, self.taught(&site))
    }

    /// Pins a GVfs bridge backend's capabilities for the rest of the operation.
    pub(super) fn site(
        &self,
        key: MountKey,
        names: Option<MountNames>,
        dir: BorrowedFd<'_>,
    ) -> Site {
        let site = Site::of(key, names, dir);
        if site.key() != key {
            self.seen()
                .entry(site.key())
                .or_insert_with(|| mount_capabilities(site.key()));
        }
        site
    }

    fn taught(&self, site: &Site) -> MountCapabilities {
        self.seen().get(&site.key()).copied().unwrap_or_default()
    }

    /// Lasts for later operations too, until the mount changes.
    pub(super) fn learn(&self, site: &Site, strictness: Strictness) {
        self.teach(site, |capabilities| {
            capabilities.strictness = capabilities.strictness.and(strictness);
        });
    }

    pub(super) fn holds_links(&self, site: &Site) -> bool {
        self.taught(site).symlinks != Some(false)
    }

    /// Where names must be UTF-8, a link target that is not stays behind: SMB with POSIX
    /// extensions stores each malformed byte as `?`, and NTFS refuses it.
    pub(super) fn holds_link_to(&self, site: &Site, target: &OsStr) -> bool {
        let taught = self.taught(site);
        taught.symlinks != Some(false)
            && (target.to_str().is_some()
                || site.rules.taught(taught.strictness).encoding == NameEncoding::AnyBytes)
    }

    pub(super) fn refuse_links(&self, site: &Site) {
        self.teach(site, |capabilities| capabilities.symlinks = Some(false));
    }

    fn teach(&self, site: &Site, learn: impl Fn(&mut MountCapabilities)) {
        if let Some(seen) = self.seen().get_mut(&site.key()) {
            learn(seen);
        }
        teach_mount_capability(site.mount, site.key(), learn);
    }

    fn seen(&self) -> MutexGuard<'_, HashMap<MountKey, MountCapabilities>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[derive(Clone)]
pub(super) struct NamePolicy {
    pub profiles: Profiles,
    pub mode: NameMode,
}

impl Default for NamePolicy {
    fn default() -> Self {
        Self::new(NameMode::Keep)
    }
}

impl NamePolicy {
    pub(super) fn new(mode: NameMode) -> Self {
        Self {
            profiles: Profiles::default(),
            mode,
        }
    }

    /// Names given back keep all but what no name on the mount can have.
    pub(super) fn applied(&self, rules: NameProfile, strictness: Strictness) -> NameProfile {
        match self.mode {
            NameMode::Fix => rules.taught(strictness),
            NameMode::Keep => rules.encoding_rules(),
        }
    }

    pub(super) fn in_folder(&self, site: &Site) -> FolderRules {
        let strictness = if site.rules.learns {
            self.profiles.taught(site).strictness
        } else {
            Strictness::default()
        };
        FolderRules {
            fixed: self.applied(site.rules, strictness),
            limit: site.rules.taught(strictness).limit,
            site: *site,
        }
    }

    pub(super) async fn destination(
        &self,
        destination: &gio::File,
        sources: &[gio::File],
        cancellable: &gio::Cancellable,
    ) -> PasteDestination {
        let Some(path) = destination.path().filter(|_| destination.is_native()) else {
            let scheme = destination.uri_scheme().unwrap_or_default();
            return PasteDestination {
                profile: MountProfile::gvfs(&scheme),
                crossing: sources
                    .iter()
                    .map(|source| source.is_native().then_some(true))
                    .collect(),
                filesystem: filesystem_id(destination, cancellable).await.ok().flatten(),
            };
        };
        let profiles = self.profiles.clone();
        let (sources, unknown) = (sources.to_vec(), sources.len());
        gio::spawn_blocking(move || {
            let folders = sources
                .iter()
                .map(|source| {
                    source
                        .parent()
                        .filter(|_| source.is_native())
                        .and_then(|parent| parent.path())
                })
                .collect::<Vec<_>>();
            native_destination(&profiles, &path, &folders)
        })
        .await
        .unwrap_or_else(|_| PasteDestination {
            profile: MountProfile::unknown(),
            crossing: vec![Some(true); unknown],
            filesystem: None,
        })
    }

    pub(super) async fn folder(
        &self,
        source: &gio::File,
        target: &gio::File,
        cancellable: &gio::Cancellable,
    ) -> Result<FolderNames, glib::Error> {
        let Some(target_path) = target.path().filter(|_| target.is_native()) else {
            let rules = gvfs_profile(target.uri_scheme().as_deref().unwrap_or_default());
            let same = !source.is_native()
                && match filesystem_id(source, cancellable).await? {
                    Some(id) => filesystem_id(target, cancellable).await?.as_ref() == Some(&id),
                    None => false,
                };
            return Ok(FolderNames {
                rules: self.applied(rules, Strictness::default()),
                fixing: !same,
                links: true,
            });
        };
        let source_path = source.path().filter(|_| source.is_native());
        let profiles = self.profiles.clone();
        let (rules, strictness, crossing, links) = run_blocking(move || {
            let fs = fs();
            let target = open_local_parent_directory(&target_path).map_err(io_error)?;
            let (key, seen) = profiles.mount(fs, target.as_fd());
            let site = profiles.site(key, seen.names, target.as_fd());
            let taught = profiles.taught(&site);
            let crossing = match source_path {
                Some(source) => {
                    let source = open_local_parent_directory(&source).map_err(io_error)?;
                    let device = fs
                        .stat_fd(target.as_fd())
                        .map_err(|error| io_error(os_error_detail(error)))?
                        .identity
                        .device;
                    crosses_into(fs, source.as_fd(), (device, &site))
                }
                None => true,
            };
            let links = taught.symlinks != Some(false);
            Ok((site.rules, taught.strictness, crossing, links))
        })
        .await?;
        Ok(FolderNames {
            rules: self.applied(rules, strictness),
            fixing: crossing,
            links,
        })
    }
}

pub(super) struct PasteDestination {
    pub profile: MountProfile,
    /// `None` where GIO's filesystem ids decide.
    crossing: Vec<Option<bool>>,
    filesystem: Option<glib::GString>,
}

impl PasteDestination {
    pub(super) fn fixing(&self, filesystems: &[Option<glib::GString>]) -> Vec<bool> {
        self.crossing
            .iter()
            .zip(filesystems)
            .map(|(crossing, filesystem)| {
                crossing.unwrap_or_else(|| filesystem.is_none() || *filesystem != self.filesystem)
            })
            .collect()
    }
}

fn native_destination(
    profiles: &Profiles,
    path: &Path,
    folders: &[Option<PathBuf>],
) -> PasteDestination {
    let fs = fs();
    let destination = open_local_parent_directory(path).ok().and_then(|dir| {
        let (key, seen) = profiles.mount(fs, dir.as_fd());
        let site = profiles.site(key, seen.names, dir.as_fd());
        let device = fs.stat_fd(dir.as_fd()).ok()?.identity.device;
        let profile = MountProfile::of(seen, &site, profiles.taught(&site));
        Some((profile, (device, site)))
    });
    let Some((profile, (device, site))) = destination else {
        return PasteDestination {
            profile: MountProfile::unknown(),
            crossing: vec![Some(true); folders.len()],
            filesystem: None,
        };
    };
    let mut seen = HashMap::<&Path, bool>::new();
    let crossing = folders
        .iter()
        .map(|folder| {
            let Some(folder) = folder else {
                return Some(true);
            };
            let crossing = *seen.entry(folder).or_insert_with(|| {
                open_local_parent_directory(folder)
                    .ok()
                    .is_none_or(|dir| crosses_into(fs, dir.as_fd(), (device, &site)))
            });
            Some(crossing)
        })
        .collect();
    PasteDestination {
        profile,
        crossing,
        filesystem: None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FolderRules {
    pub fixed: NameProfile,
    pub limit: NameLimit,
    pub site: Site,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FolderNames {
    pub rules: NameProfile,
    pub fixing: bool,
    pub links: bool,
}

/// The GVfs bridge serves every backend from one mount, so there a site is a backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Site {
    pub rules: NameProfile,
    pub mount: MountKey,
    /// `None` on the bridge where no backend is known.
    backend: Option<MountKey>,
}

impl Site {
    pub(super) fn of(mount: MountKey, names: Option<MountNames>, dir: BorrowedFd<'_>) -> Self {
        let (rules, backend) = match names {
            Some(MountNames::Profile(rules)) => (rules, Some(mount)),
            Some(MountNames::PerBackend { depth }) => {
                let path = opened_path(dir).ok();
                let folder = path
                    .as_deref()
                    .and_then(|path| folders(path).nth(usize::from(depth)));
                (
                    gvfs_bridge_profile(folder),
                    folder.map(|folder| backend_key(mount, folder)),
                )
            }
            None => (NameProfile::unknown(0), Some(mount)),
        };
        Self {
            rules,
            mount,
            backend,
        }
    }

    pub(super) fn key(&self) -> MountKey {
        self.backend.unwrap_or(self.mount)
    }
}

/// The bridge names a backend's folder `<scheme>:<details>`, such as `smb-share:server=…`.
fn backend_key(bridge: MountKey, folder: &OsStr) -> MountKey {
    let mut hasher = DefaultHasher::new();
    (bridge, folder).hash(&mut hasher);
    MountKey::Backend(hasher.finish())
}

pub(super) fn crosses(source: (u64, &Site), target: (u64, &Site)) -> bool {
    source.0 != target.0 || source.1.backend.is_none() || source.1.backend != target.1.backend
}

fn crosses_into(fs: &dyn FsOps, source: BorrowedFd<'_>, target: (u64, &Site)) -> bool {
    let Ok(stat) = fs.stat_fd(source) else {
        return true;
    };
    if stat.identity.device != target.0 {
        return true;
    }
    let (key, seen) = known_mount(fs, source);
    crosses(
        (stat.identity.device, &Site::of(key, seen.names, source)),
        target,
    )
}

/// GIO's id of the filesystem holding `file`, as Nautilus compares them.
pub(super) async fn filesystem_id(
    file: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<Option<glib::GString>, glib::Error> {
    let info = await_cancellable(file, cancellable, |file, cancellable, result| {
        file.query_info_async(
            gio::FILE_ATTRIBUTE_ID_FILESYSTEM,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?;
    Ok(info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM))
}

pub(super) fn name_rules(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> NameProfile {
    let (key, seen) = known_mount(fs, dir);
    let site = Site::of(key, seen.names, dir);
    site.rules.taught(mount_capabilities(site.key()).strictness)
}

pub(super) fn name_limit(fs: &dyn FsOps, dir: BorrowedFd<'_>) -> NameLimit {
    name_rules(fs, dir).limit
}

pub(super) async fn folder_name_rules(folder: &gio::File) -> Option<NameProfile> {
    let Some(path) = folder.path().filter(|_| folder.is_native()) else {
        return Some(gvfs_profile(
            folder.uri_scheme().as_deref().unwrap_or_default(),
        ));
    };
    gio::spawn_blocking(move || {
        open_local_parent_directory(&path).map(|dir| name_rules(fs(), dir.as_fd()))
    })
    .await
    .ok()
    .and_then(Result::ok)
}

pub(super) async fn folder_name_limit(folder: &gio::File) -> NameLimit {
    folder_name_rules(folder)
        .await
        .map_or_else(NameLimit::default, |rules| rules.limit)
}
