// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(in crate::adapters::local_operations::copy_engine) enum Naming {
    Decide,
    Create,
    /// The filesystem may take two names for one, so names that may collide
    /// are claimed in byte order before their files are written.
    Claim,
}

impl Naming {
    pub(super) fn key(self) -> Key {
        if self == Self::Create {
            Key::Exact
        } else {
            Key::Folded
        }
    }

    pub(super) fn decide(self, fs: &dyn FsOps, family: Family, folder: BorrowedFd<'_>) -> Self {
        if self != Self::Decide {
            return self;
        }
        let never_collide = stage::names_never_collide(family)
            && fs
                .file_flags(folder)
                .is_ok_and(|flags| flags & CASEFOLD == 0);
        if never_collide {
            Self::Create
        } else {
            Self::Claim
        }
    }
}

/// The names of a folder this copy made, against which its aliases are
/// numbered. A numbered name never takes a name an entry of the folder asks
/// for, and each is tried by one job only, so a name found taken is never
/// another job's entry mistaken for one whose reply was lost.
pub(in crate::adapters::local_operations::copy_engine) struct Names {
    pub(super) requested: Vec<OsString>,
    pub(super) keys: OnceLock<HashMap<OsString, usize>>,
    pub(super) numbered: Mutex<HashSet<OsString>>,
    /// What the folder's jobs made, by collision key, where a taken name may
    /// be the copy's own entry whose reply was lost; `None` elsewhere. Some
    /// mounts list a name in another case than it was made in, so the
    /// listing alone cannot tell such an entry from another name's.
    pub(super) made: Option<Mutex<HashMap<OsString, Vec<LocalFileIdentity>>>>,
    pub(super) identity: IdentityTraits,
    pub(super) rules: FolderRules,
}

impl Names {
    pub(super) fn new(
        requested: Vec<OsString>,
        naming: Naming,
        identity: IdentityTraits,
        rules: FolderRules,
    ) -> Arc<Self> {
        Arc::new(Self {
            requested,
            keys: OnceLock::new(),
            numbered: Mutex::default(),
            made: (naming == Naming::Claim).then(Mutex::default),
            identity,
            rules,
        })
    }

    pub(super) fn adopts(&self) -> bool {
        self.made.is_some()
    }

    pub(in crate::adapters::local_operations::copy_engine) fn made(
        &self,
        name: &OsStr,
        identity: LocalFileIdentity,
    ) {
        if let Some(made) = &self.made {
            lock(made)
                .entry(stage::collision_key(name))
                .or_default()
                .push(identity);
        }
    }

    pub(super) fn unclaimed(&self, name: &OsStr, current: &EntryStat) -> bool {
        let Some(made) = &self.made else {
            return false;
        };
        lock(made)
            .get(&stage::collision_key(name))
            .is_none_or(|made| {
                made.iter()
                    .all(|&made| compare(self.identity, made, None, current) == Proof::Different)
            })
    }

    pub(super) fn requested(&self) -> &HashMap<OsString, usize> {
        self.keys.get_or_init(|| {
            let mut keys = HashMap::new();
            for name in &self.requested {
                *keys.entry(stage::collision_key(name)).or_default() += 1;
            }
            keys
        })
    }

    pub(super) fn claims(&self, name: &OsStr, own: Option<&OsStr>) -> bool {
        let key = stage::collision_key(name);
        let asked = match self.requested().get(&key) {
            None => false,
            Some(1) => own.is_none_or(|own| stage::collision_key(own) != key),
            Some(_) => true,
        };
        !asked && lock(&self.numbered).insert(key)
    }

    pub(super) fn number<T>(
        &self,
        name: &OsStr,
        is_directory: bool,
        mut make: impl FnMut(&OsStr) -> Result<T, Failure>,
    ) -> Result<(OsString, T), Failure> {
        for candidate in numbered_names(name, is_directory, self.rules.limit) {
            if !self.claims(&candidate, None) {
                continue;
            }
            match make(&candidate) {
                Err(Failure::Place(PlaceError::Exists)) => {}
                made => return made.map(|value| (candidate, value)),
            }
        }
        Err(Failure::Place(PlaceError::Exists))
    }

    pub(super) fn stricter<T>(
        &self,
        (refused, name): (&OsStr, &OsStr),
        is_directory: bool,
        mut make: impl FnMut(&OsStr) -> Result<T, Failure>,
    ) -> Result<(OsString, T), Failure> {
        if self.claims(name, Some(refused)) {
            match make(name) {
                Err(Failure::Place(PlaceError::Exists)) => {}
                made => return made.map(|value| (name.to_os_string(), value)),
            }
        }
        self.number(name, is_directory, make)
    }
}

pub(in crate::adapters::local_operations::copy_engine) fn make_numbered<T>(
    copy: &Copy,
    names: &Names,
    path: &Path,
    is_directory: bool,
    cause: RenameCause,
    mut make: impl FnMut(&OsStr) -> Result<T, Failure>,
) -> Result<(PathBuf, T), Failure> {
    let name = path.file_name().unwrap_or_default();
    let (saved, made, cause) = match make(name) {
        Err(Failure::Place(PlaceError::Exists)) => {
            let (saved, made) = names.number(name, is_directory, make)?;
            (saved, made, cause)
        }
        Err(refused) if refused.refuses_name() => {
            let target = (name, is_directory);
            retry_refused(copy, &names.rules, target, refused, |stricter| {
                names.stricter((name, stricter), is_directory, &mut make)
            })?
        }
        made => return made.map(|value| (path.to_path_buf(), value)),
    };
    let saved = path.with_file_name(saved);
    copy.renamed(path.to_path_buf(), saved.clone(), cause);
    Ok((saved, made))
}

pub(super) fn planned_rename(copy: &Copy, folder: &Path, name: &OsStr, planned: &Planned) {
    if let Some(cause) = planned.cause {
        copy.renamed(folder.join(name), folder.join(&planned.target), cause);
    }
}
