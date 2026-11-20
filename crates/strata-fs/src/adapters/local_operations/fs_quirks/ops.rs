// SPDX-License-Identifier: MIT

use super::*;

impl FsOps for Scoped {
    fn open_at(
        &self,
        at: Entry<'_>,
        flags: OFlags,
        mode: Mode,
        resolve: Option<ResolveFlags>,
    ) -> Result<OwnedFd, Errno> {
        at_entry(
            at,
            Op::OpenAt,
            |scope| {
                scope
                    .resolved(at, |at| System.open_at(at, flags, mode, resolve))
                    .map(|opened| scope.record_open(opened))
            },
            || System.open_at(at, flags, mode, resolve),
        )
    }

    fn open_path(&self, path: &OsStr, flags: OFlags, mode: Mode) -> Result<OwnedFd, Errno> {
        let file = std::path::Path::new(path);
        let scope = std::fs::File::open(file)
            .or_else(|error| file.parent().map_or(Err(error), std::fs::File::open))
            .ok()
            .and_then(|opened| scope_for(opened.as_fd()));
        match scope {
            Some(scope) => scope.run(Op::OpenPath, &[path], |scope| {
                System
                    .open_path(path, flags, mode)
                    .map(|opened| scope.record_open(opened))
            }),
            None => System.open_path(path, flags, mode),
        }
    }

    fn dup(&self, fd: BorrowedFd<'_>) -> Result<OwnedFd, Errno> {
        on_fd(fd, Op::Dup, None, |scope| {
            let opened = System.dup(fd)?;
            if let Some(scope) = scope {
                let mut state = scope.state();
                let reported = state
                    .reported_losses
                    .get(&fd.as_raw_fd())
                    .copied()
                    .unwrap_or_default();
                state.reported_losses.insert(opened.as_raw_fd(), reported);
            }
            Ok(opened)
        })
    }

    fn stat_vfs(&self, fd: BorrowedFd<'_>) -> Result<rustix::fs::StatVfs, Errno> {
        on_fd(fd, Op::StatVfs, None, |_| System.stat_vfs(fd))
    }
    fn rename(&self, from: Entry<'_>, to: Entry<'_>, flags: RenameFlags) -> Result<(), Errno> {
        assert_off_main_thread();
        let source = scope_for(from.dir);
        let target = scope_for(to.dir);
        if !same_scope(&source, &target) {
            return Err(Errno::XDEV);
        }
        let op = if flags.contains(RenameFlags::EXCHANGE) {
            Op::Exchange
        } else {
            Op::Rename
        };
        match target {
            Some(scope) => {
                let kept = scope.inode_kept_on_lost_reply(to);
                let renamed = scope.run(op, &[from.name, to.name], |scope| {
                    scope.rename(from, to, flags)?;
                    scope.renamed(from, to, flags);
                    Ok(())
                });
                if let (Err(_), Some(kept)) = (renamed, kept) {
                    scope.keep_inode(to, kept);
                }
                renamed
            }
            None => System.rename(from, to, flags),
        }
    }

    fn link(&self, from: Entry<'_>, to: Entry<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        let source = scope_for(from.dir);
        let target = scope_for(to.dir);
        if !same_scope(&source, &target) {
            return Err(Errno::XDEV);
        }
        match target {
            Some(scope) => scope.run(Op::Link, &[from.name, to.name], |scope| {
                scope.link(from, to)
            }),
            None => System.link(from, to),
        }
    }

    fn unlink(&self, at: Entry<'_>, flags: AtFlags) -> Result<(), Errno> {
        assert_off_main_thread();
        at_entry(
            at,
            Op::Unlink,
            |scope| {
                if let Some(unlinked) = scope.unlink_held(at, flags) {
                    return unlinked;
                }
                scope.resolved(at, |at| System.unlink(at, flags))?;
                scope.unlinked(at);
                Ok(())
            },
            || System.unlink(at, flags),
        )
    }

    fn mkdir(&self, at: Entry<'_>, mode: Mode) -> Result<(), Errno> {
        assert_off_main_thread();
        at_entry(
            at,
            Op::Mkdir,
            |scope| {
                scope.creating(at, |at| {
                    match scope.too_long(at.name, |quirks| quirks.folder_name_limit) {
                        Some(errno) => Err(errno),
                        None => System.mkdir(at, mode),
                    }
                })
            },
            || System.mkdir(at, mode),
        )
    }

    fn symlink(&self, target: &OsStr, at: Entry<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        at_entry(
            at,
            Op::Symlink,
            |scope| {
                if let Support::Rejected(error) = scope.quirk(|quirks| quirks.symlinks) {
                    return Err(if scope.occupied(at) {
                        scope.collision()
                    } else {
                        error
                    });
                }
                scope.creating(at, |at| System.symlink(target, at))
            },
            || System.symlink(target, at),
        )
    }

    fn readlink(&self, at: Entry<'_>) -> Result<OsString, Errno> {
        assert_off_main_thread();
        resolved_at(at, Op::Readlink, |at| System.readlink(at))
    }

    fn stat(&self, at: Entry<'_>, force_sync: bool) -> Result<EntryStat, Errno> {
        assert_off_main_thread();
        match scope_for(at.dir) {
            Some(scope) => {
                let path = path_of(at.dir).map(|dir| dir.join(at.name));
                if force_sync {
                    scope.revalidate_read_only(path.as_deref());
                }
                scope
                    .run_logged(
                        Logged {
                            op: Op::Stat,
                            names: &[at.name],
                            forced: force_sync,
                        },
                        |scope| scope.stat(at, path.as_deref(), force_sync),
                    )
                    .map(|stat| {
                        let stat = if force_sync {
                            scope.revalidate_written_time(&stat);
                            stat
                        } else {
                            scope.with_written_time(stat)
                        };
                        scope.reported(path, scope.cached(stat, force_sync))
                    })
            }
            None => System.stat(at, force_sync),
        }
    }

    fn stat_fd(&self, fd: BorrowedFd<'_>) -> Result<EntryStat, Errno> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope
                .run(Op::StatFd, &[&fd_name(fd)], |_| System.stat_fd(fd))
                .map(|stat| {
                    let stat = scope.with_written_time(stat);
                    scope.reported(path_of(fd), scope.cached(stat, false))
                }),
            None => System.stat_fd(fd),
        }
    }

    fn open_parent(&self, dir: BorrowedFd<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match scope_for(dir) {
            Some(scope) => scope.run(Op::OpenParent, &[], |_| System.open_parent(dir)),
            None => System.open_parent(dir),
        }
    }

    fn create_exclusive(&self, at: Entry<'_>, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        at_entry(
            at,
            Op::Create,
            |scope| {
                scope
                    .creating(at, |at| System.create_exclusive(at, mode))
                    .inspect(|opened| scope.opened_for_writing(opened.as_fd()))
            },
            || System.create_exclusive(at, mode),
        )
    }

    fn names(&self, dir: Entry<'_>) -> Result<Vec<OsString>, Errno> {
        assert_off_main_thread();
        at_entry(dir, Op::Names, |_| System.names(dir), || System.names(dir))
    }

    fn mount_key(&self, dir: BorrowedFd<'_>) -> MountKey {
        assert_off_main_thread();
        match scope_for(dir) {
            Some(scope) => scope
                .run(Op::MountKey, &[], |scope| Ok(MountKey::Test(scope.id)))
                .unwrap_or(MountKey::Test(scope.id)),
            None => System.mount_key(dir),
        }
    }

    fn statfs(&self, dir: BorrowedFd<'_>) -> Option<FsStat> {
        assert_off_main_thread();
        match scope_for(dir) {
            Some(scope) => scope
                .run(Op::Statfs, &[], |scope| {
                    let system = System.statfs(dir);
                    let (kind, name_max) = scope.quirk(|quirks| (quirks.fs_type, quirks.name_max));
                    Ok(kind.or(system.map(|stat| stat.kind)).map(|kind| FsStat {
                        kind,
                        name_max: name_max.or(system.map(|stat| stat.name_max)).unwrap_or(0),
                    }))
                })
                .ok()
                .flatten(),
            None => System.statfs(dir),
        }
    }

    fn open_source(
        &self,
        at: Entry<'_>,
        listed: bool,
    ) -> Result<Option<(OwnedFd, EntryStat)>, Errno> {
        assert_off_main_thread();
        let Some(scope) = scope_for(at.dir) else {
            return System.open_source(at, listed);
        };
        let path = path_of(at.dir).map(|dir| dir.join(at.name));
        let opened = scope.run(Op::OpenSource, &[at.name], |scope| {
            if scope.close_deferred(at.name) {
                return Err(Errno::INVAL);
            }
            let opened = scope.resolved(at, |at| System.open_source(at, listed))?;
            let name_opens = scope.quirk(|quirks| quirks.name_opens);
            if let (Some((fd, stat)), Some(path)) = (&opened, &path)
                && name_opens != NameOpens::Any
            {
                scope.open_name(name_opens, (fd.as_fd(), stat), path)?;
            }
            Ok(opened)
        })?;
        Ok(opened.map(|(fd, stat)| (fd, scope.reported(path, scope.cached(stat, false)))))
    }

    fn open_tmpfile(&self, dir: BorrowedFd<'_>, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        match scope_for(dir) {
            Some(scope) => scope.run(Op::OpenTmpfile, &[], |scope| {
                match scope.quirk(|quirks| quirks.tmpfile) {
                    Support::Rejected(error) => Err(error),
                    Support::Supported if smb_base(dir) => Err(Errno::OPNOTSUPP),
                    Support::Supported => System.open_tmpfile(dir, mode),
                }
            }),
            None => System.open_tmpfile(dir, mode),
        }
    }

    fn link_fd(&self, fd: BorrowedFd<'_>, to: Entry<'_>, through_proc: bool) -> Result<(), Errno> {
        assert_off_main_thread();
        let target = scope_for(to.dir);
        if !same_scope(&scope_for(fd), &target) {
            return Err(Errno::XDEV);
        }
        let Some(scope) = target else {
            return link_fd_as_since_linux_6_10(fd, to, through_proc);
        };
        scope.run(Op::LinkFd, &[to.name], |scope| {
            scope.creating(to, |to| {
                let (links, tmpfile_link) =
                    scope.quirk(|quirks| (quirks.links, quirks.tmpfile_link));
                if let Support::Rejected(error) = links {
                    return Err(error);
                }
                match (tmpfile_link, through_proc) {
                    (TmpfileLink::ProcOnly, false) | (TmpfileLink::Unavailable, _) => {
                        Err(Errno::NOENT)
                    }
                    _ => link_fd_as_since_linux_6_10(fd, to, through_proc),
                }
            })
        })
    }

    fn reflink(&self, target: BorrowedFd<'_>, source: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        let refused =
            scope_for(target).and_then(|scope| match scope.quirk(|quirks| quirks.reflink) {
                Support::Rejected(error) => Some(error),
                Support::Supported => None,
            });
        between(Op::Clone, source, target, || match refused {
            Some(error) => Err(error),
            None => System.reflink(target, source).map(|()| 0),
        })
        .map(drop)
    }

    fn copy_range(
        &self,
        source: BorrowedFd<'_>,
        source_offset: u64,
        target: BorrowedFd<'_>,
        target_offset: u64,
        len: usize,
    ) -> Result<usize, Errno> {
        assert_off_main_thread();
        let end = scope_for(source).and_then(|scope| {
            scope.quirk(|quirks| {
                if quirks.virtual_sizes {
                    Some(0)
                } else {
                    quirks.copy_range_end
                }
            })
        });
        let len = end.map_or(len, |end| {
            len.min(usize::try_from(end.saturating_sub(source_offset)).unwrap_or(len))
        });
        between(Op::CopyRange, source, target, || {
            if len == 0 {
                return Ok(0);
            }
            System.copy_range(source, source_offset, target, target_offset, len)
        })
    }

    fn read_at(&self, fd: BorrowedFd<'_>, buffer: &mut [u8], offset: u64) -> Result<usize, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::Read, None, |scope| {
            let limit = io_limit(scope, buffer.len());
            System.read_at(fd, &mut buffer[..limit], offset)
        })
    }

    fn write_at(&self, fd: BorrowedFd<'_>, buffer: &[u8], offset: u64) -> Result<usize, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::Write, None, |scope| {
            let limit = io_limit(scope, buffer.len());
            if let Some(scope) = scope {
                scope.mark_written(fd);
            }
            System.write_at(fd, &buffer[..limit], offset)
        })
    }

    fn set_len(&self, fd: BorrowedFd<'_>, len: u64) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SetLen, None, |_| System.set_len(fd, len))
    }

    fn seek_data(&self, fd: BorrowedFd<'_>, offset: u64) -> Result<u64, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SeekData, None, |_| System.seek_data(fd, offset))
    }

    fn seek_hole(&self, fd: BorrowedFd<'_>, offset: u64) -> Result<u64, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SeekHole, None, |_| System.seek_hole(fd, offset))
    }

    fn sync_data(&self, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SyncData, None, |_| System.sync_data(fd))
    }

    fn sync_fs(&self, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SyncFs, None, |scope| {
            if let Some(scope) = scope {
                let mut state = scope.state();
                let lost = state.writebacks_lost;
                let reported = state.reported_losses.entry(fd.as_raw_fd()).or_default();
                if lost > *reported {
                    *reported = lost;
                    return Err(Errno::IO);
                }
            }
            System.sync_fs(fd)
        })
    }

    fn sync_dir(&self, fd: BorrowedFd<'_>) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SyncDir, None, |_| System.sync_dir(fd))
    }

    fn close(&self, fd: OwnedFd) -> Result<(), Errno> {
        assert_off_main_thread();
        let Some(scope) = scope_for(fd.as_fd()) else {
            return System.close(fd);
        };
        let name = fd_name(fd.as_fd());
        // A fault leaves `fd` to its drop, which closes it.
        scope.run(Op::Close, &[&name], move |scope| {
            scope.closing(fd.as_fd());
            System.close(fd)
        })
    }

    fn chown_fd(
        &self,
        fd: BorrowedFd<'_>,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::ChownFd, None, |scope| {
            match scope.map(|scope| scope.quirk(|quirks| quirks.chown)) {
                Some(Support::Rejected(error)) => Err(error),
                _ => System.chown_fd(fd, uid, gid),
            }
        })
    }

    fn chmod_fd(&self, fd: BorrowedFd<'_>, mode: u32) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::ChmodFd, None, |scope| {
            if scope.is_some_and(|scope| scope.keeps_read_only(fd, mode)) {
                return Ok(());
            }
            System.chmod_fd(fd, mode)
        })
    }

    fn set_times_fd(
        &self,
        fd: BorrowedFd<'_>,
        atime: (i64, u32),
        mtime: (i64, u32),
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SetTimes, None, |scope| {
            System.set_times_fd(fd, atime, mtime)?;
            if let (Some(scope), Ok(stat)) = (scope, System.stat_fd(fd)) {
                scope.revalidate_written_time(&stat);
            }
            Ok(())
        })
    }

    fn xattr_names(&self, fd: BorrowedFd<'_>) -> Result<Vec<OsString>, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::XattrNames, None, |scope| {
            let extra = scope.map_or_else(Vec::new, |scope| {
                scope.quirk(|quirks| quirks.extra_xattrs.clone())
            });
            let mut names = match System.xattr_names(fd) {
                Ok(names) => names,
                Err(_) if !extra.is_empty() => Vec::new(),
                Err(error) => return Err(error),
            };
            names.extend(extra.iter().map(|(name, _)| OsString::from(name)));
            if scope.is_some_and(|scope| scope.quirk(|quirks| quirks.acls_listed_first)) {
                names.sort_by_key(|name| !name.as_encoded_bytes().starts_with(b"system."));
            }
            Ok(names)
        })
    }

    fn get_xattr(&self, fd: BorrowedFd<'_>, name: &OsStr) -> Result<Vec<u8>, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::GetXattr, Some(name), |scope| {
            let extra = scope.and_then(|scope| scope.extra_xattr(name));
            extra.map_or_else(|| System.get_xattr(fd, name), Ok)
        })
    }

    fn set_xattr(&self, fd: BorrowedFd<'_>, name: &OsStr, value: &[u8]) -> Result<(), Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::SetXattr, Some(name), |scope| {
            let refused = scope.and_then(|scope| scope.xattr_refusal(name));
            refused.map_or_else(|| System.set_xattr(fd, name, value), Err)
        })
    }

    fn open_dir(&self, at: Entry<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        at_entry(
            at,
            Op::OpenDir,
            |scope| {
                let opened = scope.resolved(at, |at| System.open_dir(at))?;
                Ok(scope.record_open(opened))
            },
            || System.open_dir(at),
        )
    }

    fn list(&self, dir: BorrowedFd<'_>) -> Result<Vec<(OsString, FileType)>, Errno> {
        assert_off_main_thread();
        on_fd(dir, Op::List, None, |_| System.list(dir))
    }

    fn list_chunks(
        &self,
        dir: BorrowedFd<'_>,
        each: &mut dyn FnMut(Listed) -> ControlFlow<()>,
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        let Some(scope) = scope_for(dir) else {
            return System.list_chunks(dir, each);
        };
        let name = fd_name(dir);
        let mut listed = Vec::new();
        System.list_chunks(dir, &mut |chunk| {
            listed.extend(chunk);
            ControlFlow::Continue(())
        })?;
        if let Some(folder) = path_of(dir) {
            listed = scope.listed(&folder, listed);
        }
        let size = scope
            .quirk(|quirks| quirks.listing_chunk)
            .unwrap_or(usize::MAX)
            .max(1);
        let mut chunks = listed.chunks(size).map(<[_]>::to_vec);
        loop {
            let chunk = chunks.next();
            scope.run(Op::ListChunks, &[name.as_os_str()], |_| Ok(()))?;
            if chunk.is_none_or(|chunk| each(chunk).is_break()) {
                return Ok(());
            }
        }
    }

    fn folder_stamp(&self, path: &Path, force_sync: bool) -> Result<FolderStamp, Errno> {
        assert_off_main_thread();
        match scope_for_folder(path) {
            Some(scope) => scope
                .run_logged(
                    Logged {
                        op: Op::FolderStamp,
                        names: &[path.file_name().unwrap_or_default()],
                        forced: force_sync,
                    },
                    |_| System.folder_stamp(path, force_sync),
                )
                .map(|stamp| scope.reported_stamp(path, scope.cached_stamp(stamp, force_sync))),
            None => System.folder_stamp(path, force_sync),
        }
    }

    fn access(&self, path: &Path, access: Access) -> Result<(), Errno> {
        assert_off_main_thread();
        match scope_for_folder(path) {
            Some(scope) => scope.run(Op::Access, &[path.file_name().unwrap_or_default()], |_| {
                System.access(path, access)
            }),
            None => System.access(path, access),
        }
    }

    fn create_node(&self, at: Entry<'_>, kind: FileType, mode: Mode) -> Result<(), Errno> {
        assert_off_main_thread();
        at_entry(
            at,
            Op::CreateNode,
            |scope| {
                let support = scope.quirk(|quirks| match kind {
                    FileType::Fifo => quirks.fifos,
                    _ => quirks.node_create,
                });
                scope.creating(at, |at| match support {
                    Support::Rejected(error) => Err(error),
                    Support::Supported => System.create_node(at, kind, mode),
                })
            },
            || System.create_node(at, kind, mode),
        )
    }

    fn open_fifo(&self, at: Entry<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        resolved_at(at, Op::OpenFifo, |at| System.open_fifo(at))
    }

    fn create_readonly(&self, at: Entry<'_>, mode: Mode) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        at_entry(
            at,
            Op::CreateReadonly,
            |scope| scope.creating(at, |at| System.create_readonly(at, mode)),
            || System.create_readonly(at, mode),
        )
    }

    fn open_write(&self, at: Entry<'_>) -> Result<OwnedFd, Errno> {
        assert_off_main_thread();
        at_entry(
            at,
            Op::OpenWrite,
            |scope| {
                scope
                    .resolved(at, |at| System.open_write(at))
                    .inspect(|opened| scope.opened_for_writing(opened.as_fd()))
            },
            || System.open_write(at),
        )
    }

    fn file_flags(&self, fd: BorrowedFd<'_>) -> Result<u32, Errno> {
        assert_off_main_thread();
        on_fd(fd, Op::FileFlags, None, |_| System.file_flags(fd))
    }

    fn set_link_times(
        &self,
        at: Entry<'_>,
        atime: (i64, u32),
        mtime: (i64, u32),
    ) -> Result<(), Errno> {
        assert_off_main_thread();
        at_entry(
            at,
            Op::SetLinkTimes,
            |scope| {
                scope.resolved(at, |at| {
                    System.set_link_times(at, atime, mtime)?;
                    if let Ok(stat) = System.stat(at, false) {
                        scope.revalidate_written_time(&stat);
                    }
                    Ok(())
                })
            },
            || System.set_link_times(at, atime, mtime),
        )
    }
}
