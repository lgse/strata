// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::adapters::local_operations) enum Renamed {
    Direct(Placed),
    CaseOnly,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::adapters::local_operations) enum RenameFailure {
    Place(PlaceError),
    Stranded {
        temporary: OsString,
        error: PlaceError,
    },
}

/// Renames `from` to `to` without replacing another item. A name that only
/// differs in case on a case-insensitive mount resolves to the item itself,
/// so it is renamed in two steps through a hidden temporary name.
pub(in crate::adapters::local_operations) fn rename_in_place(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
) -> Result<Renamed, RenameFailure> {
    if from.name == to.name && same_directory(fs, from.dir, to.dir) {
        return Ok(Renamed::Unchanged);
    }
    match place(fs, from, to, None) {
        Err(PlaceError::Exists) => {}
        result => return result.map(Renamed::Direct).map_err(RenameFailure::Place),
    }
    let Some(identity) = alias_of(fs, from, to) else {
        return Err(RenameFailure::Place(PlaceError::Exists));
    };
    let aside = set_aside(fs, from, identity, None)?;
    // The SMB client keeps answering for the lookup of `to` for about a
    // second; revalidating drops it.
    let _ = fs.stat(to, true);
    let renamed = match place_back(fs, aside.entry(), to, aside.identity, None) {
        Ok(_) => Ok(Renamed::CaseOnly),
        Err(error) => match place_back(fs, aside.entry(), from, aside.identity, None) {
            Ok(_) => Err(RenameFailure::Place(error)),
            Err(_) => {
                return Err(RenameFailure::Stranded {
                    temporary: aside.kept(),
                    error,
                });
            }
        },
    };
    let _ = aside.originals.remove(fs, from.dir, &Deadline::default());
    renamed
}

fn same_directory(fs: &dyn FsOps, left: BorrowedFd<'_>, right: BorrowedFd<'_>) -> bool {
    match (fs.stat_fd(left), fs.stat_fd(right)) {
        (Ok(left), Ok(right)) => left.identity == right.identity,
        _ => false,
    }
}

/// The item's identity when `to` names `from` itself rather than another
/// item: a case alias, never a separate hard link.
pub(in crate::adapters::local_operations) fn alias_of(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
) -> Option<LocalFileIdentity> {
    let source = fs.stat(from, true).ok()?;
    let target = fs.stat(to, true).ok()?;
    let names = || fs.names(at(to.dir, OsStr::new(".")));
    if source.identity != target.identity {
        // libfuse without `use_ino` numbers each spelling as its own node.
        let same_item = (source.kind, source.size, source.mtime)
            == (target.kind, target.size, target.mtime)
            && same_directory(fs, from.dir, to.dir)
            && names().is_ok_and(|names| {
                let mut spellings = names
                    .iter()
                    .filter(|name| same_ignoring_case(name, to.name));
                spellings.next().is_some_and(|name| name == from.name) && spellings.next().is_none()
            });
        return same_item.then_some(source.identity);
    }
    let single_name = !source.is_directory() && source.nlink == 1;
    let alias = single_name
        || names().is_ok_and(|names| {
            !names.iter().any(|name| name == to.name)
                && names
                    .iter()
                    .filter(|name| same_ignoring_case(name, to.name))
                    .all(|name| name == from.name)
        });
    alias.then_some(source.identity)
}

fn same_ignoring_case(left: &OsStr, right: &OsStr) -> bool {
    match (left.to_str(), right.to_str()) {
        (Some(left), Some(right)) => left.to_lowercase() == right.to_lowercase(),
        _ => left.eq_ignore_ascii_case(right),
    }
}
