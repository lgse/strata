// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations) struct Originals {
    pub name: OsString,
    handle: OffMain<Arc<OwnedFd>>,
}

impl Originals {
    pub(in crate::adapters::local_operations) fn create(
        fs: &dyn FsOps,
        dir: BorrowedFd<'_>,
    ) -> Result<Self, PlaceError> {
        let (name, identity) = reserve_hidden_name(HiddenName::Originals, |name| {
            fs.mkdir(at(dir, name), Mode::from_raw_mode(0o700))
                .map_err(creation_error)?;
            fs.stat(at(dir, name), false)
                .map(|stat| stat.identity)
                .map_err(PlaceError::Io)
        })
        .map_err(|error| match error {
            PlaceError::InvalidName => PlaceError::Unsupported,
            error => error,
        })?;
        let opened = open_beneath(fs, dir, Path::new(&name)).and_then(|handle| {
            match fs.stat_fd(handle.as_fd()) {
                Ok(stat) if stat.identity == identity => Ok(handle),
                Ok(_) => Err(PlaceError::Changed),
                Err(error) => Err(PlaceError::Io(error)),
            }
        });
        match opened {
            Ok(handle) => Ok(Self {
                name,
                handle: OffMain::new(Arc::new(handle)),
            }),
            Err(error) => {
                remove_if_unchanged(fs, at(dir, &name), identity, None, AtFlags::REMOVEDIR);
                Err(error)
            }
        }
    }

    pub(in crate::adapters::local_operations) fn entry<'a>(&'a self, name: &'a OsStr) -> Entry<'a> {
        Entry::new(&**self.handle, name)
    }

    pub(in crate::adapters::local_operations) fn handle(&self) -> Arc<OwnedFd> {
        Arc::clone(&self.handle)
    }

    pub(in crate::adapters::local_operations) fn kept(&self, name: &OsStr) -> OsString {
        Path::new(&self.name).join(name).into_os_string()
    }

    pub(in crate::adapters::local_operations) fn remove(
        &self,
        fs: &dyn FsOps,
        dir: BorrowedFd<'_>,
        deadline: &Deadline,
    ) -> Result<(), Refused> {
        let folder = at(dir, &self.name);
        leftovers::remove_held_folder(fs, folder, self.handle.as_fd(), deadline, &|| false)
    }

    pub(in crate::adapters::local_operations) fn refusal(
        &self,
        fs: &dyn FsOps,
        dir: BorrowedFd<'_>,
        refused: Refused,
    ) -> String {
        let folder = at(dir, &self.name);
        match refused {
            Refused::StillOpen(_) => {
                leftovers::still_open(fs, folder).map(crate::services::error_detail)
            }
            Refused::Changed(_) => Some(
                rust_i18n::t!(
                    "%{name} changed while the operation was in progress",
                    name = self.name.to_string_lossy()
                )
                .into_owned(),
            ),
            _ => None,
        }
        .unwrap_or_else(|| native_fs::os_error_detail(refused.errno()))
    }
}

pub(in crate::adapters::local_operations) struct SetAside {
    pub originals: Originals,
    pub name: OsString,
    pub identity: LocalFileIdentity,
    pub attributes: Option<Attributes>,
}

impl SetAside {
    pub(in crate::adapters::local_operations) fn entry(&self) -> Entry<'_> {
        self.originals.entry(&self.name)
    }

    pub(in crate::adapters::local_operations) fn kept(&self) -> OsString {
        self.originals.kept(&self.name)
    }
}

pub(in crate::adapters::local_operations) fn open_beneath(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    relative: &Path,
) -> Result<OwnedFd, PlaceError> {
    native_fs::retry_local_open(|| {
        fs.open_at(
            at(dir, relative.as_os_str()),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            Some(
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            ),
        )
    })
    .map_err(|error| match error {
        Errno::XDEV => PlaceError::CrossDevice,
        error => PlaceError::Io(error),
    })
}

pub(in crate::adapters::local_operations) fn move_aside(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    expected: LocalFileIdentity,
    attributes: Option<Attributes>,
) -> Result<(LocalFileIdentity, Option<Attributes>), PlaceError> {
    let current = unchanged_identity(fs, from, (expected, attributes))?;
    renamed_aside(Placing { fs, from, to }, current).map(|_| (current, attributes))
}

type Expected = (LocalFileIdentity, Option<Attributes>);

fn unchanged_identity(
    fs: &dyn FsOps,
    from: Entry<'_>,
    (expected, attributes): Expected,
) -> Result<LocalFileIdentity, PlaceError> {
    let unchanged = |current: &EntryStat| {
        current.identity == expected
            || native_fs::unchanged(
                mount_profile::identity_at(fs, from.dir),
                expected,
                attributes,
                current,
            )
    };
    match fs.stat(from, false) {
        Ok(current) if unchanged(&current) => Ok(current.identity),
        Ok(_) | Err(Errno::NOENT) => Err(PlaceError::Changed),
        Err(error) => Err(PlaceError::Io(error)),
    }
}

fn renamed_aside(placing: Placing<'_>, current: LocalFileIdentity) -> Result<Placed, PlaceError> {
    let Placing { fs, from, to } = placing;
    match fs.rename(from, to, RenameFlags::empty()) {
        Ok(()) => verified(fs, to, Some(current), Ok(Placed::Renamed)),
        Err(Errno::XDEV) => Err(PlaceError::CrossDevice),
        Err(error) if may_have_lost_reply(error) => {
            resolve(&placing, Some(current), error).map(|()| Placed::Renamed)
        }
        Err(error) => Err(PlaceError::Io(error)),
    }
}

pub(in crate::adapters::local_operations) fn set_aside(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    expected: LocalFileIdentity,
    attributes: Option<Attributes>,
) -> Result<SetAside, RenameFailure> {
    let originals = Originals::create(fs, entry.dir).map_err(RenameFailure::Place)?;
    let moved = with_owner_write(fs, entry, (expected, attributes), || {
        move_aside(fs, entry, originals.entry(entry.name), expected, attributes)
    });
    match moved {
        Ok((identity, attributes)) => Ok(SetAside {
            originals,
            name: entry.name.to_os_string(),
            identity,
            attributes,
        }),
        Err(error @ PlaceError::Unconfirmed) => Err(RenameFailure::Stranded {
            temporary: originals.kept(entry.name),
            error,
        }),
        Err(error) => {
            let _ = originals.remove(fs, entry.dir, &Deadline::default());
            Err(RenameFailure::Place(error))
        }
    }
}

pub(in crate::adapters::local_operations) fn put_back(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    aside: &SetAside,
    name: &OsStr,
) -> Result<(), PlaceError> {
    place_back(
        fs,
        aside.entry(),
        at(dir, name),
        aside.identity,
        aside.attributes,
    )?;
    let _ = aside.originals.remove(fs, dir, &Deadline::default());
    Ok(())
}

pub(in crate::adapters::local_operations) fn place_back(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    identity: LocalFileIdentity,
    attributes: Option<Attributes>,
) -> Result<Placed, PlaceError> {
    let current = match fs.stat(from, false) {
        Ok(current) => current,
        Err(_) => return place(fs, from, to, Some(identity)),
    };
    if !native_fs::unchanged(
        mount_profile::identity_at(fs, from.dir),
        identity,
        attributes,
        &current,
    ) {
        return Err(PlaceError::Changed);
    }
    with_owner_write(fs, from, (current.identity, None), || {
        place(fs, from, to, Some(current.identity))
    })
}

fn with_owner_write<T>(
    fs: &dyn FsOps,
    folder: Entry<'_>,
    (expected, attributes): Expected,
    moved: impl Fn() -> Result<T, PlaceError>,
) -> Result<T, PlaceError> {
    let refused = match moved() {
        Err(refused @ PlaceError::Io(Errno::ACCESS)) => refused,
        result => return result,
    };
    let Ok(opened) = fs.open_dir(folder) else {
        return Err(refused);
    };
    let locked = fs.stat_fd(opened.as_fd()).ok().filter(|stat| {
        stat.mode & 0o200 == 0
            && native_fs::unchanged(
                mount_profile::identity_at(fs, folder.dir),
                expected,
                attributes,
                stat,
            )
    });
    let Some(locked) = locked else {
        return Err(refused);
    };
    if fs.chmod_fd(opened.as_fd(), locked.mode | 0o200).is_err() {
        return Err(refused);
    }
    let result = moved();
    let _ = fs.chmod_fd(opened.as_fd(), locked.mode);
    result
}
