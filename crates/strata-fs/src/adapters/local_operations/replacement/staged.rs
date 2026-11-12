// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations) enum StageContent {
    Directory,
    Symlink(OsString),
    Fifo,
}

/// Created and discarded off the main thread; there is no `Drop` cleanup.
pub(in crate::adapters::local_operations) struct StagedSibling {
    pub(in crate::adapters::local_operations) ownership: Option<copy_engine::Ownership>,
    pub(in crate::adapters::local_operations) parent: Arc<OwnedFd>,
    pub(in crate::adapters::local_operations) name: OsString,
    pub(in crate::adapters::local_operations) identity: LocalFileIdentity,
    pub(in crate::adapters::local_operations) restrictive: Vec<copy_engine::Restrictive>,
}

impl StagedSibling {
    pub(in crate::adapters::local_operations) fn create(
        parent: Arc<OwnedFd>,
        content: StageContent,
    ) -> Result<Self, PlaceError> {
        let (name, identity) = placement::reserve_hidden_name(HiddenName::Copy, |name| {
            let entry = Entry::new(&*parent, name);
            let made = match &content {
                StageContent::Directory => {
                    fs().mkdir(entry, rustix::fs::Mode::from_raw_mode(0o700))
                }
                StageContent::Symlink(link) => fs().symlink(link, entry),
                StageContent::Fifo => fs().create_node(
                    entry,
                    rustix::fs::FileType::Fifo,
                    rustix::fs::Mode::from_raw_mode(0o600),
                ),
            };
            match made {
                Ok(()) => fs()
                    .stat(entry, false)
                    .map(|stat| stat.identity)
                    .map_err(PlaceError::Io),
                Err(rustix::io::Errno::EXIST) => {
                    placement::left_by_lost_reply(fs(), entry, |stat| match &content {
                        StageContent::Directory => {
                            stat.is_directory()
                                && fs().names(entry).is_ok_and(|names| names.is_empty())
                        }
                        StageContent::Symlink(link) => {
                            stat.kind == rustix::fs::FileType::Symlink
                                && fs().readlink(entry).is_ok_and(|current| current == *link)
                        }
                        StageContent::Fifo => stat.kind == rustix::fs::FileType::Fifo,
                    })
                    .ok_or(PlaceError::Exists)
                }
                Err(error) => Err(PlaceError::Io(error)),
            }
        })?;
        Ok(Self {
            ownership: None,
            parent,
            name,
            identity,
            restrictive: Vec::new(),
        })
    }

    pub(in crate::adapters::local_operations) fn recorded(&self) -> Recorded {
        Recorded {
            identity: self.identity,
            attributes: self
                .ownership
                .as_ref()
                .and_then(copy_engine::Ownership::attributes),
        }
    }
}

pub(in crate::adapters::local_operations) async fn discard_incomplete_staged(
    staged: StagedSibling,
) -> Result<(), glib::Error> {
    let staged = if staged.restrictive.is_empty() {
        staged
    } else {
        run_blocking(move || {
            copy_engine::unlock_stage(&staged);
            Ok(staged)
        })
        .await?
    };
    let identity = Some(staged.recorded());
    parallel_delete_local(
        vec![PinnedEntry {
            parent: staged.parent,
            name: staged.name,
            identity,
        }],
        gio::Cancellable::new(),
    )
    .await
}

pub(in crate::adapters::local_operations) fn placement_error(error: PlaceError) -> glib::Error {
    glib::Error::new(placement_error_kind(error), &placement_reason(error))
}

pub(in crate::adapters::local_operations) async fn publish_staged_without_replace(
    staged: StagedSibling,
    target_path: PathBuf,
    durable: bool,
    cancellable: gio::Cancellable,
) -> Result<copy_engine::Ownership, glib::Error> {
    let ownership = copy_engine::staged_ownership(&staged);
    let Some(target_name) = target_path.file_name().map(OsStr::to_os_string) else {
        let cleanup = discard_incomplete_staged(staged).await;
        return Err(copy_failure_after_cleanup(
            translated_io_error("Invalid local filesystem target"),
            cleanup,
        ));
    };
    let staged = OffMain::new(staged);
    let parent = staged.parent.clone();
    let (stage_name, name) = (staged.name.clone(), target_name.clone());
    let recorded = staged.identity;
    let step = gio::spawn_blocking(move || {
        copy_engine::stage::publish(fs(), parent.as_fd(), &stage_name, recorded, &name)
    })
    .await;
    let placed = match step {
        Ok(step) => step,
        Err(error) => {
            let cleanup = discard_incomplete_staged(staged.into_inner()).await;
            return Err(copy_failure_after_cleanup(
                io_error(blocking_join_message(error)),
                cleanup,
            ));
        }
    };
    let result = match placed {
        Ok(_) => return Ok(ownership),
        // Where the ladder cannot place the stage, publication falls back to
        // exclusive creation, even though it exposes partial copies.
        Err(PlaceError::Unsupported) => {
            copy_engine::copy_from_stage(&staged, target_name, durable, &cancellable).await
        }
        // Either name may hold the copy, so neither is removed.
        Err(PlaceError::Unconfirmed) => return Err(placement_error(PlaceError::Unconfirmed)),
        Err(error) => Err(placement_error(error)),
    };
    let cleanup = discard_incomplete_staged(staged.into_inner()).await;
    match result {
        Ok(ownership) => cleanup.map(|()| ownership).map_err(|error| {
            io_error(rust_i18n::t!(
                "The item was copied, but its staging copy could not be removed: %{error}",
                error = crate::services::gio_error_detail(&error)
            ))
        }),
        Err(error) => Err(copy_failure_after_cleanup(error, cleanup)),
    }
}

pub(in crate::adapters::local_operations) async fn publish_staged_replacement(
    staged: StagedSibling,
    target_path: PathBuf,
    durable: bool,
) -> Result<copy_engine::Ownership, glib::Error> {
    publish_staged_without_replace(staged, target_path, durable, gio::Cancellable::new())
        .await
        .map_err(|error| original_in_trash(&crate::services::gio_error_detail(&error)))
}
