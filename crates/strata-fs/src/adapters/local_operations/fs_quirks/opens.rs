// SPDX-License-Identifier: MIT

use super::*;

impl Scope {
    /// Whether opening `name` waits on a close the client defers.
    pub fn close_deferred(&self, name: &OsStr) -> bool {
        self.quirk(|quirks| quirks.deferred_close)
            .is_some_and(|(pattern, until)| matches(pattern, &[name]) && clock_for_test() < until)
    }

    /// Records `path` as a name `opened` was opened by, refusing it while
    /// another name of the same file is in the way.
    pub fn open_name(
        &self,
        name_opens: NameOpens,
        (fd, stat): (BorrowedFd<'_>, &EntryStat),
        path: &Path,
    ) -> Result<(), Errno> {
        let key = (stat.identity.device, stat.identity.inode);
        let mut state = self.state();
        if name_opens == NameOpens::OneAtATime {
            state
                .opened_names
                .retain(|&(key, ref name, fd)| still_open(fd, name, key));
        }
        if state
            .opened_names
            .iter()
            .any(|(other, name, _)| *other == key && name != path)
        {
            return Err(Errno::INVAL);
        }
        state
            .opened_names
            .push((key, path.to_path_buf(), fd.as_raw_fd()));
        Ok(())
    }

    /// A stat forcing revalidation sees the folder at `path` as it is.
    pub fn revalidate_read_only(&self, path: Option<&Path>) {
        if self.quirk(|quirks| quirks.stale_read_only)
            && let Some(path) = path.and_then(|path| path.canonicalize().ok())
        {
            self.state().read_only_unseen.remove(&path);
        }
    }

    /// Whether giving the folder `fd` back `mode` does nothing, as write
    /// access given back before a revalidating stat does.
    pub fn keeps_read_only(&self, fd: BorrowedFd<'_>, mode: u32) -> bool {
        let Some(path) = path_of(fd) else {
            return false;
        };
        if !self.quirk(|quirks| quirks.stale_read_only) || !path.is_dir() {
            return false;
        }
        let mut state = self.state();
        if mode & 0o200 == 0 {
            state.read_only_unseen.insert(path);
            return false;
        }
        state.read_only_unseen.contains(&path)
    }

    pub fn extra_xattr(&self, name: &OsStr) -> Option<Vec<u8>> {
        self.quirk(|quirks| {
            quirks
                .extra_xattrs
                .iter()
                .find(|(extra, _)| OsStr::new(extra) == name)
                .map(|(_, value)| value.to_vec())
        })
    }

    pub fn xattr_refusal(&self, name: &OsStr) -> Option<Errno> {
        self.quirk(|quirks| {
            quirks
                .xattrs_refused
                .iter()
                .find(|(prefix, _)| name.as_encoded_bytes().starts_with(prefix.as_bytes()))
                .map(|(_, error)| *error)
        })
    }
}
