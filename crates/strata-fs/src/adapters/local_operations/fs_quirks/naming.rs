// SPDX-License-Identifier: MIT

use super::*;

impl Scope {
    pub fn invalid(&self, name: &OsStr) -> Option<Errno> {
        self.quirk(|quirks| quirks.invalid_name)
            .and_then(|invalid| invalid(name))
    }

    pub fn too_long(
        &self,
        name: &OsStr,
        limit: impl FnOnce(&Quirks) -> Option<(usize, Errno)>,
    ) -> Option<Errno> {
        self.quirk(limit)
            .filter(|(bytes, _)| name.len() > *bytes)
            .map(|(_, errno)| errno)
    }

    pub fn refusal(&self, name: &OsStr) -> Option<Errno> {
        self.invalid(name)
            .or_else(|| self.too_long(name, |quirks| quirks.name_limit))
    }

    pub fn stored<'a>(&self, name: &'a OsStr) -> Cow<'a, OsStr> {
        match self
            .quirk(|quirks| quirks.stored_as)
            .and_then(|stored| stored(name))
        {
            Some(stored) if !stored.is_empty() => Cow::Owned(stored),
            _ => Cow::Borrowed(name),
        }
    }

    /// The other existing name a lookup of `entry` would find: one that
    /// differs in case on a case-insensitive mount, or the name a create of
    /// `entry` stores. Lookups revalidate, so an already case-insensitive
    /// FUSE mount cannot answer from its cache for a name that has moved.
    pub fn alias(&self, entry: Entry<'_>) -> Option<OsString> {
        let (case_insensitive, stored_as) =
            self.quirk(|quirks| (quirks.case_insensitive, quirks.stored_as.is_some()));
        let compares_names = case_insensitive || stored_as;
        if !compares_names || System.stat(entry, true).is_ok() {
            return None;
        }
        let stored = self.stored(entry.name);
        if !case_insensitive {
            let found = System
                .stat(
                    Entry {
                        dir: entry.dir,
                        name: &stored,
                    },
                    true,
                )
                .is_ok();
            return (found && stored != entry.name).then(|| stored.into_owned());
        }
        System
            .names(Entry {
                dir: entry.dir,
                name: OsStr::new("."),
            })
            .ok()?
            .into_iter()
            .find(|name| name != entry.name && name.eq_ignore_ascii_case(&stored))
    }

    pub fn stat(
        &self,
        at: Entry<'_>,
        path: Option<&Path>,
        force_sync: bool,
    ) -> Result<EntryStat, Errno> {
        let (stale_aliases, alias_inodes) =
            self.quirk(|quirks| (quirks.stale_aliases, quirks.alias_inodes));
        let path = path.filter(|_| stale_aliases);
        if let Some(path) = path {
            let mut state = self.state();
            if force_sync {
                state.stale.remove(path);
            } else if let Some(stat) = state.stale.get(path) {
                return Ok(*stat);
            }
        }
        let exact = System.stat(at, force_sync);
        let Some(alias) = self.alias_if_missing(at, &exact) else {
            return exact;
        };
        let mut stat = System.stat(
            Entry {
                dir: at.dir,
                name: &alias,
            },
            force_sync,
        )?;
        if alias_inodes {
            stat.identity.inode = !stat.identity.inode;
        }
        if let Some(path) = path {
            self.state().stale.insert(path.to_path_buf(), stat);
        }
        Ok(stat)
    }

    pub fn stale(&self, entry: Entry<'_>) -> bool {
        path_of(entry.dir).is_some_and(|dir| self.state().stale.contains_key(&dir.join(entry.name)))
    }

    pub fn occupied(&self, entry: Entry<'_>) -> bool {
        System.stat(entry, true).is_ok() || self.alias(entry).is_some()
    }

    pub fn collision(&self) -> Errno {
        if self.quirk(|quirks| quirks.collisions_as_eperm) {
            Errno::PERM
        } else {
            Errno::EXIST
        }
    }

    pub fn creating<T>(
        &self,
        at: Entry<'_>,
        create: impl FnOnce(Entry<'_>) -> Result<T, Errno>,
    ) -> Result<T, Errno> {
        let _naming = self.naming();
        if let Some(errno) = self.refusal(at.name) {
            return Err(errno);
        }
        if self.alias(at).is_some() {
            return Err(self.collision());
        }
        let stored = self.stored(at.name);
        let stored = Entry {
            dir: at.dir,
            name: &stored,
        };
        let created = create(stored).map_err(|error| {
            if error == Errno::EXIST {
                self.collision()
            } else {
                error
            }
        })?;
        if self.quirk(|quirks| quirks.births == Births::MadeUp)
            && let Ok(stat) = System.stat(stored, false)
        {
            self.state()
                .created
                .insert((stat.identity.device, stat.identity.inode));
        }
        Ok(created)
    }

    pub fn resolved<T>(
        &self,
        at: Entry<'_>,
        read: impl Fn(Entry<'_>) -> Result<T, Errno>,
    ) -> Result<T, Errno> {
        let exact = read(at);
        match self.alias_if_missing(at, &exact) {
            Some(alias) => read(Entry {
                dir: at.dir,
                name: &alias,
            }),
            None => exact,
        }
    }

    /// The alias of `at` once a call naming it exactly found nothing. A real
    /// case-insensitive lookup finds the entry whichever case it was renamed
    /// to meanwhile, so the exact name is tried first.
    pub fn alias_if_missing<T>(&self, at: Entry<'_>, exact: &Result<T, Errno>) -> Option<OsString> {
        matches!(exact, Err(Errno::NOENT))
            .then(|| self.alias(at))
            .flatten()
    }
}
