// SPDX-License-Identifier: MIT

use super::fs_ops::{Entry, EntryStat, OffMain, fs, list_folder};
use crate::{
    adapters::{
        gio_file_for_location, location_for_file,
        trash_support::{TrashError, trash_with_cleanup},
    },
    model::{Location, hidden_names::HiddenName},
    services::{
        ItemIdentity, KeptCause, KeptItem, PlacedSnapshot, ReplacedOriginal, TransferReport,
    },
};
use gtk::{gio, glib, prelude::*};

use super::{
    owed::{Deletion, Owed},
    placement::{Displacement, Exchanged, PlaceError, RenameFailure},
};
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    future::Future,
    ops::ControlFlow,
    os::fd::{AsFd, OwnedFd},
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{Arc, atomic::AtomicUsize},
};

use super::{
    CopyInto, CopyRecord, GioNames, LeftOut, NamedChildren, TargetFolder, await_cancellable,
    blocking_join_message, cancelled_local_operation, copy_engine, copy_failure_after_cleanup,
    copy_recursively_with_progress,
    deletion::{
        PinnedEntry, local_delete_worker_count, parallel_delete_local,
        parallel_delete_local_blocking_with_workers, parallel_delete_local_with,
    },
    device, existing_folder_names, io_error, is_trash_unsupported_failure,
    leftovers::Deadline,
    merge_aside,
    mount_profile::{self, NamePolicy, known_mount},
    native_fs::{
        LocalFileIdentity, Pinned, Recorded, open_local_parent_directory, os_error_detail,
    },
    native_path, os_io_error, placement, placement_error_kind, placement_failure, placement_reason,
    progress::TransferProgressTracker,
    release::ActiveOperation,
    run_blocking, translated_io_error, trash, was_cancelled,
};

pub(super) enum StageContent {
    Directory,
    Symlink(OsString),
    Fifo,
}

/// Created and discarded off the main thread; there is no `Drop` cleanup.
pub(super) struct StagedSibling {
    pub(super) ownership: Option<copy_engine::Ownership>,
    pub(super) parent: Arc<OwnedFd>,
    pub(super) name: OsString,
    pub(super) identity: LocalFileIdentity,
    pub(super) restrictive: Vec<copy_engine::Restrictive>,
}

impl StagedSibling {
    pub(super) fn create(parent: Arc<OwnedFd>, content: StageContent) -> Result<Self, PlaceError> {
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

    pub(super) fn recorded(&self) -> Recorded {
        Recorded {
            identity: self.identity,
            attributes: self
                .ownership
                .as_ref()
                .and_then(copy_engine::Ownership::attributes),
        }
    }
}

pub(super) async fn discard_incomplete_staged(staged: StagedSibling) -> Result<(), glib::Error> {
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

pub(super) fn placement_error(error: PlaceError) -> glib::Error {
    glib::Error::new(placement_error_kind(error), &placement_reason(error))
}

pub(super) async fn publish_staged_without_replace(
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

pub(super) type CopyStep = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        MergeCopy,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<MergeCopied, glib::Error>>>>,
>;

#[derive(Clone, Default)]
pub(super) struct MergeCopy {
    pub(super) moving: bool,
    pub(super) durable: bool,
    pub(super) placing: Option<Arc<copy_engine::Placing>>,
}

#[derive(Default)]
pub(super) struct MergeCopied {
    pub(super) sources: Option<copy_engine::Manifest>,
    pub(super) nested: Vec<copy_engine::Barrier>,
    pub(super) left: Vec<gio::File>,
}

pub(super) fn cannot_replace_safely(name: &str) -> String {
    rust_i18n::t!(
        "“%{name}” can’t be replaced safely at this location. Delete or rename it first, then try again.",
        name = name
    )
    .into_owned()
}

pub(super) fn original_kept_aside(name: &str, kept: &OsStr, error: &str) -> String {
    rust_i18n::t!(
        "Could not replace “%{name}”; the original was kept as “%{kept}”: %{error}",
        name = name,
        kept = kept.to_string_lossy(),
        error = error
    )
    .into_owned()
}

pub(super) fn original_not_removed(name: &str, kept: &OsStr, error: &glib::Error) -> String {
    rust_i18n::t!(
        "“%{name}” was replaced, but its original could not be removed from “%{kept}”: %{error}",
        name = name,
        kept = kept.to_string_lossy(),
        error = crate::services::gio_error_detail(error)
    )
    .into_owned()
}

pub(super) fn could_not_replace(_name: &str, error: &str) -> String {
    rust_i18n::t!("Could not safely replace the item: %{error}", error = error).into_owned()
}

pub(super) fn original_in_trash(error: &str) -> glib::Error {
    io_error(rust_i18n::t!(
        "Could not finish placing the replacement item; the original is in Trash: %{error}",
        error = error
    ))
}

#[derive(Clone, Copy)]
pub(super) enum Removal {
    Trash,
    Trashed(Option<ItemIdentity>),
    Without(Displacement),
}

#[derive(Clone, Copy)]
pub(super) enum ReplaceStrategy {
    PlaceSource,
    Staged(Removal),
}

pub(super) struct ReplacePlan {
    pub(super) source_parent: Arc<OwnedFd>,
    pub(super) source_name: OsString,
    pub(super) source: EntryStat,
    pub(super) target_parent: Arc<OwnedFd>,
    pub(super) target_name: OsString,
    pub(super) target: EntryStat,
    pub(super) target_pin: Pinned,
    pub(super) strategy: ReplaceStrategy,
}

impl ReplacePlan {
    pub(super) fn source_entry(&self) -> Entry<'_> {
        Entry::new(&*self.source_parent, &self.source_name)
    }

    pub(super) fn target_entry(&self) -> Entry<'_> {
        Entry::new(&*self.target_parent, &self.target_name)
    }

    pub(super) fn target_display(&self) -> String {
        self.target_name.to_string_lossy().into_owned()
    }

    pub(super) fn directory(&self) -> bool {
        self.source.is_directory()
    }

    pub(super) fn copy_stays_outside_source(&self) -> bool {
        !self.directory()
            || placement::contains(fs(), self.source.identity, self.target_parent.as_fd())
                == Some(false)
    }
}

pub(super) fn split_local_path(path: &Path) -> Result<(PathBuf, OsString), glib::Error> {
    path.parent()
        .zip(path.file_name())
        .map(|(parent, name)| (parent.to_path_buf(), name.to_os_string()))
        .ok_or_else(|| translated_io_error("Invalid local filesystem target"))
}

/// `None` means both names are one item.
pub(super) fn plan_replace(
    source_path: &Path,
    target_path: &Path,
    move_source: bool,
) -> Result<Option<ReplacePlan>, glib::Error> {
    let (source_parent_path, source_name) = split_local_path(source_path)?;
    let (target_parent_path, target_name) = split_local_path(target_path)?;
    let source_parent = open_local_parent_directory(&source_parent_path).map_err(io_error)?;
    let target_parent = open_local_parent_directory(&target_parent_path).map_err(io_error)?;
    let inspect = |parent: &OwnedFd, name: &OsStr, path: &Path| {
        fs().stat(Entry::new(parent, name), false).map_err(|error| {
            io_error(rust_i18n::t!(
                "Could not inspect %{path}: %{error}",
                path = path.display(),
                error = os_error_detail(error)
            ))
        })
    };
    let source = inspect(&source_parent, &source_name, source_path)?;
    let target = inspect(&target_parent, &target_name, target_path)?;
    if source.is_directory() != target.is_directory() {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr("A file and a folder cannot safely replace one another"),
        ));
    }
    let target_directory = fs().stat_fd(target_parent.as_fd()).ok();
    let shared_parent = target_directory.is_some()
        && fs()
            .stat_fd(source_parent.as_fd())
            .ok()
            .map(|dir| dir.identity)
            == target_directory.map(|dir| dir.identity);
    // A hard link elsewhere in the folder is a separate name and is replaced.
    if shared_parent
        && source.identity == target.identity
        && (source_name == target_name
            || placement::alias_of(
                fs(),
                Entry::new(&source_parent, &source_name),
                Entry::new(&target_parent, &target_name),
            )
            .is_some())
    {
        return Ok(None);
    }
    let strategy = if move_source
        && fs().mount_key(source_parent.as_fd()) == fs().mount_key(target_parent.as_fd())
        && target_directory.is_some_and(|dir| dir.identity.device == source.identity.device)
    {
        ReplaceStrategy::PlaceSource
    } else if fs().can_trash(target_path) {
        ReplaceStrategy::Staged(Removal::Trash)
    } else {
        ReplaceStrategy::Staged(Removal::Without(displacement_without_trash(
            &target_parent,
            source.is_directory(),
            &target_name.to_string_lossy(),
        )?))
    };
    let target_pin = Pinned::at(
        fs(),
        Entry::new(&target_parent, &target_name),
        &target,
        known_mount(fs(), target_parent.as_fd()).1.identity,
    );
    Ok(Some(ReplacePlan {
        source_parent: Arc::new(source_parent),
        source_name,
        source,
        target_parent: Arc::new(target_parent),
        target_name,
        target,
        target_pin,
        strategy,
    }))
}

pub(super) fn displacement_without_trash(
    parent: &OwnedFd,
    directory: bool,
    name: &str,
) -> Result<Displacement, glib::Error> {
    displacement_or_refusal(
        placement::displacement_without_trash(fs(), parent.as_fd(), directory),
        name,
    )
}

pub(super) fn displacement_or_refusal(
    displacement: Result<Option<Displacement>, PlaceError>,
    name: &str,
) -> Result<Displacement, glib::Error> {
    match displacement {
        Ok(Some(displacement)) => Ok(displacement),
        Ok(None) => Err(io_error(cannot_replace_safely(name))),
        Err(error) => Err(placement_failure(error, name, could_not_replace)),
    }
}

pub(super) fn ensure_unchanged(
    entry: Entry<'_>,
    recorded: &EntryStat,
    pin: &Pinned,
) -> Result<EntryStat, glib::Error> {
    let unchanged = |stat: &EntryStat| {
        pin.unchanged(
            fs(),
            mount_profile::identity_at(fs(), entry.dir),
            recorded,
            stat,
        )
    };
    match fs().stat(entry, false) {
        Ok(stat) if unchanged(&stat) => Ok(stat),
        Ok(_) | Err(rustix::io::Errno::NOENT) => Err(translated_io_error(
            "The target changed while the operation was in progress",
        )),
        Err(error) => Err(os_io_error(error)),
    }
}

pub(super) fn set_aside_failure(failure: RenameFailure, name: &str) -> glib::Error {
    match failure {
        RenameFailure::Place(PlaceError::Unsupported) => io_error(cannot_replace_safely(name)),
        RenameFailure::Place(PlaceError::Changed) => {
            translated_io_error("The target changed while the operation was in progress")
        }
        RenameFailure::Place(error) => placement_failure(error, name, could_not_replace),
        RenameFailure::Stranded { temporary, error } => io_error(original_kept_aside(
            name,
            &temporary,
            &crate::services::error_detail(placement_reason(error)),
        )),
    }
}

pub(super) struct DisplacedOriginal {
    pub(super) parent: OffMain<Arc<OwnedFd>>,
    pub(super) originals: placement::Originals,
    pub(super) name: OsString,
    pub(super) identity: Recorded,
}

impl DisplacedOriginal {
    pub(super) fn aside(parent: Arc<OwnedFd>, aside: placement::SetAside) -> Self {
        Self {
            parent: OffMain::new(parent),
            originals: aside.originals,
            name: aside.name,
            identity: Recorded {
                identity: aside.identity,
                attributes: aside.attributes,
            },
        }
    }

    pub(super) fn kept_in(&self) -> &OsStr {
        &self.originals.name
    }
}

pub(super) async fn remove_displaced_original(
    original: DisplacedOriginal,
    deadline: &Deadline,
) -> Result<(), glib::Error> {
    let DisplacedOriginal {
        parent,
        originals,
        name,
        identity,
    } = original;
    let roots = vec![PinnedEntry {
        parent: originals.handle(),
        name,
        identity: Some(identity),
    }];
    let deadline = deadline.clone();
    parallel_delete_local_with(roots, gio::Cancellable::new(), move |roots, cancelled| {
        let workers = local_delete_worker_count(&roots);
        parallel_delete_local_blocking_with_workers(roots, cancelled, workers, &deadline)?;
        originals
            .remove(fs(), parent.as_fd(), &deadline)
            .map_err(|refused| originals.refusal(fs(), parent.as_fd(), refused))
    })
    .await
}

pub(super) enum SourcePlaced {
    Trashed(Option<ItemIdentity>),
    SetAside(placement::SetAside),
    CopyAfterTrash(Option<ItemIdentity>),
    /// The original is back at its name, and the source must be copied.
    PutBack,
}

pub(super) fn place_source(
    plan: &ReplacePlan,
    target_path: &Path,
    cancellable: &gio::Cancellable,
) -> Result<SourcePlaced, glib::Error> {
    let target = plan.target_entry();
    let source = plan.source_entry();
    let expected = Some(plan.source.identity);
    let current = ensure_unchanged(target, &plan.target, &plan.target_pin)?;
    match trash::trash_verified(target, target_path, &current, cancellable) {
        Ok(trashed) => {
            return match placement::place(fs(), source, target, expected) {
                Ok(_) => Ok(SourcePlaced::Trashed(trashed)),
                Err(
                    error @ (PlaceError::Exists
                    | PlaceError::Changed
                    | PlaceError::Unconfirmed
                    | PlaceError::IntoItself),
                ) => Err(original_in_trash(&crate::services::error_detail(
                    placement_reason(error),
                ))),
                Err(_) if !plan.copy_stays_outside_source() => Err(original_in_trash(
                    &crate::services::error_detail(placement_reason(PlaceError::IntoItself)),
                )),
                Err(_) => Ok(SourcePlaced::CopyAfterTrash(trashed)),
            };
        }
        Err(error) if is_trash_unsupported_failure(false, &error) => {}
        Err(error) => return Err(error.error),
    }
    let name = plan.target_display();
    let aside = placement::set_aside(fs(), target, current.identity, current.kept_attributes())
        .map_err(|error| set_aside_failure(error, &name))?;
    let error = match placement::place(fs(), source, target, expected) {
        Ok(_) => return Ok(SourcePlaced::SetAside(aside)),
        Err(error) => error,
    };
    if placement::put_back(fs(), plan.target_parent.as_fd(), &aside, &plan.target_name).is_err() {
        return Err(io_error(original_kept_aside(
            &name,
            &aside.kept(),
            &crate::services::error_detail(placement_reason(error)),
        )));
    }
    match error {
        PlaceError::CrossDevice | PlaceError::Unsupported if plan.copy_stays_outside_source() => {
            Ok(SourcePlaced::PutBack)
        }
        error => Err(placement_failure(
            error,
            &plan.source_name.to_string_lossy(),
            could_not_replace,
        )),
    }
}

pub(super) struct StagedReplacement {
    pub(super) original: ReplacedOriginal,
    pub(super) displaced: Option<DisplacedOriginal>,
    pub(super) placed: Option<PlacedSnapshot>,
}

#[derive(Clone, Copy)]
pub(super) struct Durability {
    pub(super) durable: bool,
    pub(super) each_file: bool,
}

/// Every failure discards the stage.
pub(super) async fn replace_through_stage(
    plan: &Arc<ReplacePlan>,
    mut removal: Removal,
    (source_path, moving): (&Path, bool),
    target_path: &Path,
    cancellable: &gio::Cancellable,
    (
        copying,
        Durability {
            mut durable,
            each_file,
        },
    ): (Copying, Durability),
) -> Result<(StagedReplacement, StagedCopy), glib::Error> {
    let in_trash = |error: glib::Error, removal: Removal| match removal {
        Removal::Trashed(_) => original_in_trash(&crate::services::gio_error_detail(&error)),
        _ => error,
    };
    let options = copy_engine::Options {
        names: copying.names,
        manifest: moving,
        durable,
        ..copy_engine::Options::default()
    };
    let placed_entries = copying.placed_entries;
    let (staged, mut sources, report) = copy_engine::copy_to_stage(
        source_path.to_path_buf(),
        target_path.to_path_buf(),
        options,
        copying.progress,
        cancellable,
    )
    .await
    .map_err(|error| in_trash(error, removal))?;
    let staged_ownership = copy_engine::staged_ownership(&staged);
    let mut copied = (sources.take(), report);
    loop {
        let parent = OffMain::new(staged.parent.clone());
        let step_plan = plan.clone();
        let step = match removal {
            Removal::Trashed(trashed) => {
                let ownership =
                    publish_staged_replacement(staged, target_path.to_path_buf(), durable).await?;
                if let Some(sources) = &mut copied.0 {
                    sources.copied_to(copy_engine::published(
                        parent.into_inner(),
                        plan.target_name.clone(),
                        ownership.clone(),
                    ));
                }
                let placed = if moving {
                    None
                } else {
                    copy_engine::placed_proven(
                        target_path.to_path_buf(),
                        ownership,
                        placed_entries.clone(),
                        cancellable,
                    )
                    .await
                };
                let replaced = StagedReplacement {
                    original: ReplacedOriginal::Trashed(trashed),
                    displaced: None,
                    placed,
                };
                return Ok((replaced, copied));
            }
            Removal::Without(Displacement::SetAside) => {
                let (replaced, ownership) =
                    set_aside_and_publish(plan, staged, target_path, durable).await?;
                if let Some(sources) = &mut copied.0 {
                    sources.copied_to(copy_engine::published(
                        parent.into_inner(),
                        plan.target_name.clone(),
                        ownership,
                    ));
                }
                return Ok((replaced, copied));
            }
            Removal::Trash => {
                let (path, cancellable) = (target_path.to_path_buf(), cancellable.clone());
                run_blocking(move || {
                    let target = Entry::new(&*parent, &step_plan.target_name);
                    let current =
                        ensure_unchanged(target, &step_plan.target, &step_plan.target_pin)?;
                    match trash::trash_verified(target, &path, &current, &cancellable) {
                        Ok(trashed) => Ok(Removal::Trashed(trashed)),
                        Err(error) if is_trash_unsupported_failure(false, &error) => {
                            displacement_without_trash(
                                &parent,
                                step_plan.directory(),
                                &step_plan.target_display(),
                            )
                            .map(Removal::Without)
                        }
                        Err(error) => Err(error.error),
                    }
                })
                .await
            }
            Removal::Without(Displacement::Exchange) => {
                let (stage_name, copy) = (staged.name.clone(), staged.recorded());
                let exchanged = run_blocking(move || {
                    let target = Entry::new(&*parent, &step_plan.target_name);
                    let current =
                        ensure_unchanged(target, &step_plan.target, &step_plan.target_pin)?;
                    let name = step_plan.target_display();
                    let stage = Entry::new(&*parent, &stage_name);
                    match placement::exchange_through(
                        fs(),
                        stage,
                        target,
                        copy.identity,
                        copy.attributes,
                    ) {
                        Ok(Exchanged::Swapped(originals)) => {
                            Ok(Ok((originals, Recorded::of(&current))))
                        }
                        Ok(Exchanged::Refused) => displacement_or_refusal(
                            placement::set_aside_if_safe(
                                fs(),
                                parent.as_fd(),
                                step_plan.directory(),
                            ),
                            &name,
                        )
                        .map(|displacement| Err(Removal::Without(displacement))),
                        Err(error) => Err(placement_failure(error, &name, could_not_replace)),
                    }
                })
                .await;
                match exchanged {
                    Ok(Ok((originals, original))) => {
                        if let Some(sources) = &mut copied.0 {
                            sources.copied_to(copy_engine::published(
                                staged.parent.clone(),
                                plan.target_name.clone(),
                                staged_ownership,
                            ));
                        }
                        let replaced = StagedReplacement {
                            original: ReplacedOriginal::Deleted,
                            displaced: Some(DisplacedOriginal {
                                parent: OffMain::new(staged.parent),
                                originals,
                                name: plan.target_name.clone(),
                                identity: original,
                            }),
                            placed: None,
                        };
                        return Ok((replaced, copied));
                    }
                    Ok(Err(next)) => Ok(next),
                    Err(error) => Err(error),
                }
            }
        };
        let step = match step {
            // The original is deleted after all, once the stage is on disk.
            Ok(Removal::Without(displacement))
                if matches!(removal, Removal::Trash) && !durable && each_file =>
            {
                durable = true;
                let stage = target_path.with_file_name(&staged.name);
                copy_engine::sync_files(vec![stage], cancellable)
                    .await
                    .map(|()| Removal::Without(displacement))
            }
            step => step,
        };
        match step {
            Ok(next) => removal = next,
            Err(error) => {
                let cleanup = discard_incomplete_staged(staged).await;
                return Err(copy_failure_after_cleanup(error, cleanup));
            }
        }
    }
}

pub(super) async fn set_aside_and_publish(
    plan: &Arc<ReplacePlan>,
    staged: StagedSibling,
    target_path: &Path,
    durable: bool,
) -> Result<(StagedReplacement, copy_engine::Ownership), glib::Error> {
    let name = plan.target_display();
    let parent = staged.parent.clone();
    let step_plan = plan.clone();
    let step_name = name.clone();
    let aside = run_blocking(move || {
        let target = Entry::new(&*parent, &step_plan.target_name);
        let current = ensure_unchanged(target, &step_plan.target, &step_plan.target_pin)?;
        placement::set_aside(fs(), target, current.identity, current.kept_attributes())
            .map_err(|error| set_aside_failure(error, &step_name))
    })
    .await;
    let aside = match aside {
        Ok(aside) => aside,
        Err(error) => {
            let cleanup = discard_incomplete_staged(staged).await;
            return Err(copy_failure_after_cleanup(error, cleanup));
        }
    };
    let parent = staged.parent.clone();
    // Publication finishes regardless of cancellation once the original is aside.
    let ownership = match publish_staged_without_replace(
        staged,
        target_path.to_path_buf(),
        durable,
        gio::Cancellable::new(),
    )
    .await
    {
        Ok(ownership) => ownership,
        Err(error) => {
            let detail = crate::services::gio_error_detail(&error);
            let kept = aside.kept();
            let step_plan = plan.clone();
            let restored = run_blocking(move || {
                placement::put_back(fs(), parent.as_fd(), &aside, &step_plan.target_name)
                    .map_err(placement_error)
            })
            .await;
            return Err(io_error(match restored {
                Ok(()) => could_not_replace(&name, &detail),
                Err(_) => original_kept_aside(&name, &kept, &detail),
            }));
        }
    };
    Ok((
        StagedReplacement {
            original: ReplacedOriginal::Deleted,
            displaced: Some(DisplacedOriginal::aside(parent, aside)),
            placed: None,
        },
        ownership,
    ))
}

/// `admin://` has no Trash to keep the original in, so a confirmed replace overwrites
/// a file in place; a folder is never replaced there.
async fn replace_as_administrator(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: &gio::Cancellable,
    on_replaced: &dyn Fn(ReplacedOriginal, Option<PlacedSnapshot>),
) -> Result<Owed, glib::Error> {
    let target_kind = await_cancellable(&target, cancellable, |target, cancellable, result| {
        target.query_info_async(
            gio::FILE_ATTRIBUTE_STANDARD_TYPE,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?
    .file_type();
    if target_kind == gio::FileType::Directory {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr("Safe replacement is unavailable at this destination"),
        ));
    }
    let flags = gio::FileCopyFlags::OVERWRITE | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
    if move_source {
        super::progress::move_file_with_progress(source, target, flags, cancellable.clone(), None)
            .await?;
    } else {
        super::progress::copy_file_with_progress(source, target, flags, cancellable.clone(), None)
            .await?;
    }
    on_replaced(ReplacedOriginal::Deleted, None);
    Ok(Owed::default())
}

/// Replaces `target` with `source`; the original is removed only once the
/// replacement is in place.
pub(super) async fn replace_owing(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: &gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    copying: Copying,
    on_replaced: &dyn Fn(ReplacedOriginal, Option<PlacedSnapshot>),
) -> Result<Owed, glib::Error> {
    if let Some(locations) = affected_locations {
        locations.extend([&source, &target].into_iter().filter_map(location_for_file));
    }
    if target.has_uri_scheme(crate::model::ADMIN_SCHEME) {
        return replace_as_administrator(source, target, move_source, cancellable, on_replaced)
            .await;
    }
    let source_path = source.path().ok_or_else(|| {
        glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr("Safe replacement is unavailable for this source"),
        )
    })?;
    let target_path = target.path().ok_or_else(|| {
        glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr("Safe replacement is unavailable at this destination"),
        )
    })?;
    cancellable.set_error_if_cancelled()?;
    let (planned_source, planned_target) = (source_path.clone(), target_path.clone());
    let Some(plan) =
        run_blocking(move || plan_replace(&planned_source, &planned_target, move_source)).await?
    else {
        return Ok(Owed::default());
    };
    let plan = OffMain::new(Arc::new(plan));
    let removal = match plan.strategy {
        ReplaceStrategy::Staged(removal) => removal,
        ReplaceStrategy::PlaceSource => {
            cancellable.set_error_if_cancelled()?;
            let (step_plan, path, step_cancellable) =
                (plan.clone(), target_path.clone(), cancellable.clone());
            match run_blocking(move || place_source(&step_plan, &path, &step_cancellable)).await? {
                SourcePlaced::Trashed(trashed) => {
                    on_replaced(ReplacedOriginal::Trashed(trashed), None);
                    return Ok(Owed::default());
                }
                // The source took its place by a rename, so it was not copied.
                SourcePlaced::SetAside(aside) => {
                    on_replaced(ReplacedOriginal::Deleted, None);
                    let kept = target_path
                        .with_file_name(&aside.originals.name)
                        .join(&aside.name);
                    let original = DisplacedOriginal::aside(plan.target_parent.clone(), aside);
                    if let Err(error) =
                        remove_displaced_original(original, &Deadline::default()).await
                        && let Some(progress) = &copying.progress
                    {
                        progress.report(TransferReport {
                            kept: vec![KeptItem {
                                location: Location::local(kept),
                                cause: KeptCause::OriginalNotRemoved(
                                    crate::services::gio_error_detail(&error),
                                ),
                            }],
                            ..TransferReport::default()
                        });
                    }
                    return Ok(Owed::default());
                }
                SourcePlaced::CopyAfterTrash(trashed) => Removal::Trashed(trashed),
                SourcePlaced::PutBack => Removal::Without(Displacement::SetAside),
            }
        }
    };
    let mut owed = Owed::default();
    if move_source || !matches!(removal, Removal::Trashed(_)) {
        owed.barrier = Some(copy_engine::barrier(split_local_path(&target_path)?.0).await?);
    }
    let progress = copying.progress.clone();
    let durability = Durability {
        // A Trash attempt that falls back syncs the stage then.
        durable: move_source || matches!(removal, Removal::Without(_)),
        each_file: owed
            .barrier
            .as_ref()
            .is_some_and(copy_engine::Barrier::syncs_each_file),
    };
    let (replaced, (sources, report)) = replace_through_stage(
        &plan,
        removal,
        (&source_path, move_source),
        &target_path,
        cancellable,
        (copying, durability),
    )
    .await?;
    on_replaced(replaced.original, replaced.placed);
    if let Some(progress) = &progress {
        progress.report(report);
    }
    if let Some(original) = replaced.displaced {
        owed.owe(Deletion::Displaced(original, target_path));
    }
    if let Some(sources) = sources {
        owed.owe(Deletion::Sources(source, sources));
    }
    Ok(owed)
}

#[cfg(test)]
pub(super) async fn replace_local_with(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    copying: Copying,
    on_replaced: &dyn Fn(ReplacedOriginal, Option<PlacedSnapshot>),
) -> Result<(), glib::Error> {
    let progress = copying.progress.clone();
    replace_owing(
        source,
        target,
        move_source,
        &cancellable,
        affected_locations,
        copying,
        on_replaced,
    )
    .await?
    .settle(progress.as_deref(), &cancellable)
    .await
}

pub(super) async fn publish_staged_replacement(
    staged: StagedSibling,
    target_path: PathBuf,
    durable: bool,
) -> Result<copy_engine::Ownership, glib::Error> {
    publish_staged_without_replace(staged, target_path, durable, gio::Cancellable::new())
        .await
        .map_err(|error| original_in_trash(&crate::services::gio_error_detail(&error)))
}

pub(super) type StagedCopy = (Option<copy_engine::Manifest>, TransferReport);

#[derive(Clone)]
pub(super) struct Copying {
    pub(super) progress: Option<Rc<TransferProgressTracker>>,
    pub(super) names: NamePolicy,
    pub(super) placed_entries: Arc<AtomicUsize>,
}

impl Default for Copying {
    fn default() -> Self {
        Self {
            progress: None,
            names: NamePolicy::default(),
            placed_entries: copy_engine::placed_budget(),
        }
    }
}

#[cfg(test)]
pub(super) async fn replace_local(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
) -> Result<(), glib::Error> {
    replace_local_with(
        source,
        target,
        move_source,
        cancellable,
        affected_locations,
        Copying::default(),
        &|_, _| {},
    )
    .await
}

/// What a merge will write: `created` are the topmost destination paths with
/// no existing counterpart, `overwritten` are leaf collisions whose originals
/// go to Trash, or aside where Trash is unavailable.
#[derive(Default)]
pub(super) struct MergePlan {
    pub(super) created: Vec<Location>,
    pub(super) overwritten: Vec<Location>,
    pub(super) originals: HashMap<Location, ItemIdentity>,
    pub(super) placed: HashMap<Location, PlacedSnapshot>,
}

/// Unsupported native Trash triggers the set-aside fallback.
pub(super) type StageOverwrite = Rc<
    dyn Fn(
        Location,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ItemIdentity>, TrashError>>>>,
>;

pub(super) fn trash_stage_overwrite(
    location: Location,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<Option<ItemIdentity>, TrashError>>>> {
    let file = gio_file_for_location(&location);
    Box::pin(async move { trash_with_cleanup(&file, &cancellable).await })
}

pub(super) struct MergeHooks<'a> {
    pub(super) names: NamePolicy,
    pub(super) copy_into_target: CopyStep,
    pub(super) stage_overwrite: StageOverwrite,
    pub(super) on_merged: &'a dyn Fn(MergePlan),
    pub(super) progress: Option<Rc<TransferProgressTracker>>,
}

pub(super) fn classify_merge<'a>(
    (source, target): (&'a gio::File, &'a gio::File),
    plan: &'a mut MergePlan,
    cancellable: &'a gio::Cancellable,
    (policy, folder): (&'a NamePolicy, TargetFolder),
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>> + 'a>> {
    Box::pin(async move {
        let enumerator;
        let mut listed = match native_children(source, cancellable).await? {
            Some(children) => NamedChildren::listed(children, folder.names),
            None => {
                enumerator =
                    await_cancellable(source, cancellable, |source, cancellable, result| {
                        source.enumerate_children_async(
                            "standard::name,standard::type",
                            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    })
                    .await?;
                NamedChildren::new(&enumerator, folder.names)
            }
        };
        loop {
            let children = listed.next(cancellable).await?;
            if children.is_empty() {
                return Ok(());
            }
            for (name, kind, planned) in children {
                // Left out, so nothing at its name is replaced.
                if kind == gio::FileType::SymbolicLink && !folder.names.links {
                    continue;
                }
                let child = (
                    source.child(name),
                    kind == gio::FileType::Directory,
                    target.child(planned.target),
                );
                classify_merge_child(child, plan, cancellable, (policy, folder)).await?;
            }
        }
    })
}

async fn native_children(
    folder: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<Option<Vec<(OsString, gio::FileType)>>, glib::Error> {
    let Some(path) = folder.path().filter(|_| folder.is_native()) else {
        return Ok(None);
    };
    let cancellable = cancellable.clone();
    run_blocking(move || {
        let mut children = Vec::new();
        let listed = list_folder(&path, &mut |chunk| {
            children.extend(chunk.into_iter().map(|(name, kind)| (name, gio_kind(kind))));
            if cancellable.is_cancelled() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        if cancellable.is_cancelled() {
            return Err(cancelled_local_operation());
        }
        listed.map_err(trash::errno_error)?;
        Ok(Some(children))
    })
    .await
}

fn gio_kind(kind: rustix::fs::FileType) -> gio::FileType {
    match kind {
        rustix::fs::FileType::Directory => gio::FileType::Directory,
        rustix::fs::FileType::Symlink => gio::FileType::SymbolicLink,
        rustix::fs::FileType::RegularFile => gio::FileType::Regular,
        _ => gio::FileType::Special,
    }
}

async fn classify_merge_child(
    (child_source, source_is_directory, child_target): (gio::File, bool, gio::File),
    plan: &mut MergePlan,
    cancellable: &gio::Cancellable,
    (policy, folder): (&NamePolicy, TargetFolder),
) -> Result<(), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    let target_info =
        match await_cancellable(&child_target, cancellable, |target, cancellable, result| {
            target.query_info_async(
                "standard::type,unix::device",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await
        {
            Ok(info) => Some(info),
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => None,
            Err(error) => return Err(error),
        };
    match target_info.as_ref().map(gio::FileInfo::file_type) {
        None => {
            if let Some(location) = location_for_file(&child_target) {
                plan.created.push(location);
            }
        }
        Some(gio::FileType::Directory) if source_is_directory => {
            let folders = (&child_source, &child_target);
            let source_device = if child_source.is_native() && child_target.is_native() {
                merged_folder_device(&child_source, cancellable).await?
            } else {
                None
            };
            let devices = (
                source_device,
                target_info.and_then(|info| device(&child_target, &info)),
            );
            let folder =
                existing_folder_names(policy, folder, folders, devices, cancellable).await?;
            classify_merge(folders, plan, cancellable, (policy, folder)).await?;
        }
        Some(gio::FileType::Directory) => {
            return Err(glib::Error::new(
                gio::IOErrorEnum::NotSupported,
                &crate::i18n::tr("A file and a folder cannot safely replace one another"),
            ));
        }
        Some(_) if source_is_directory => {
            return Err(glib::Error::new(
                gio::IOErrorEnum::NotSupported,
                &crate::i18n::tr("A file and a folder cannot safely replace one another"),
            ));
        }
        Some(_) => {
            if let Some(location) = location_for_file(&child_target) {
                plan.overwritten.push(location);
            }
        }
    }
    Ok(())
}

async fn merged_folder_device(
    folder: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<Option<u64>, glib::Error> {
    let info = await_cancellable(folder, cancellable, |folder, cancellable, result| {
        folder.query_info_async(
            "standard::type,unix::device",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?;
    if info.file_type() != gio::FileType::Directory {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr("Only folders can be merged"),
        ));
    }
    Ok(device(folder, &info))
}

/// Publish completed children without replacing concurrent arrivals.
/// Already-published children survive cancellation.
pub(super) async fn merge_owing(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: &gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    placed_entries: &Arc<AtomicUsize>,
    hooks: MergeHooks<'_>,
) -> Result<Owed, glib::Error> {
    if let Some(locations) = affected_locations {
        locations.extend([&source, &target].into_iter().filter_map(location_for_file));
    }
    let devices = (
        merged_folder_device(&source, cancellable).await?,
        merged_folder_device(&target, cancellable).await?,
    );
    let mut plan = MergePlan::default();
    let folder = TargetFolder {
        names: hooks.names.folder(&source, &target, cancellable).await?,
        devices,
    };
    let folders = (&source, &target);
    classify_merge(folders, &mut plan, cancellable, (&hooks.names, folder)).await?;
    let target_path = target.path();
    let progress = hooks.progress.as_deref();
    // Originals go to Trash, or aside where Trash is unavailable, before
    // anything is copied.
    let mut trashed = Vec::new();
    let mut originals = HashMap::new();
    let mut aside = None;
    for location in &plan.overwritten {
        let staged = if cancellable.is_cancelled() {
            Err(cancelled_local_operation())
        } else {
            match (hooks.stage_overwrite)(location.clone(), cancellable.clone()).await {
                Ok(identity) => {
                    trashed.push(location.clone());
                    originals.extend(identity.map(|identity| (location.clone(), identity)));
                    Ok(())
                }
                Err(error) => match (target_path.as_deref(), location.native_path()) {
                    (Some(root), Some(path)) if is_trash_unsupported_failure(false, &error) => {
                        merge_aside::set_aside(&mut aside, root, path, location.display_name())
                            .await
                    }
                    _ if was_cancelled(&error.error) => Err(error.error),
                    _ => Err(glib::Error::new(
                        gio::IOErrorEnum::Failed,
                        &rust_i18n::t!(
                            "Could not move %{name} to Trash before merging: %{error}",
                            name = location.display_name(),
                            error = crate::services::gio_error_detail(&error.error)
                        ),
                    )),
                },
            }
        };
        if let Err(error) = staged {
            (hooks.on_merged)(MergePlan {
                overwritten: trashed,
                originals,
                ..MergePlan::default()
            });
            return Err(merge_aside::abandon(aside, error, HashMap::new(), progress).await);
        }
    }
    let written: Vec<PathBuf> = plan
        .created
        .iter()
        .chain(&plan.overwritten)
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect();
    // Undo trashes what a merge created, so it needs a record of it only
    // where originals could not go to the Trash, or none were offered it.
    let records_created = match &target_path {
        Some(root) if !move_source && !plan.created.is_empty() => {
            aside.is_some() || (trashed.is_empty() && !can_trash(root.clone()).await)
        }
        _ => false,
    };
    let records = !move_source && (!trashed.is_empty() || records_created);
    let placing = target_path
        .as_ref()
        .filter(|_| records || aside.is_some())
        .map(|root| {
            let path = |location: &Location| location.native_path().map(Path::to_path_buf);
            let created = plan.created.iter().filter(|_| records_created);
            let wanted: HashSet<_> = trashed
                .iter()
                .chain(created)
                .filter(|_| records)
                .filter_map(path)
                .collect();
            let unwanted = records_created.then(|| {
                plan.overwritten
                    .iter()
                    .filter_map(path)
                    .filter(|path| !wanted.contains(path))
                    .collect()
            });
            let aside = aside
                .as_ref()
                .map(merge_aside::MergeAside::paths)
                .unwrap_or_default();
            copy_engine::Placing::new(
                root.clone(),
                wanted,
                unwanted,
                aside,
                placed_entries.clone(),
            )
        });
    (hooks.on_merged)(MergePlan {
        created: plan.created,
        overwritten: trashed,
        originals,
        ..MergePlan::default()
    });
    let mut owed = Owed::default();
    if let Some(path) = native_path(&target).filter(|_| move_source || aside.is_some()) {
        match copy_engine::barrier(path).await {
            Ok(barrier) => owed.barrier = Some(barrier),
            Err(error) => {
                return Err(merge_aside::abandon(aside, error, HashMap::new(), progress).await);
            }
        }
    }
    let copying = MergeCopy {
        moving: move_source,
        durable: owed.barrier.is_some(),
        placing: placing.clone(),
    };
    let copied =
        (hooks.copy_into_target)(source.clone(), target, copying, cancellable.clone()).await;
    let over_aside = placing
        .as_ref()
        .map(|placing| placing.over_aside())
        .unwrap_or_default();
    if let Some(placing) = placing {
        let placed: HashMap<_, _> = placing
            .take()
            .into_iter()
            .map(|(path, placed)| (Location::local(path), placed))
            .collect();
        if !placed.is_empty() {
            (hooks.on_merged)(MergePlan {
                placed,
                ..MergePlan::default()
            });
        }
    }
    let copied = match copied {
        Ok(copied) => copied,
        Err(error) => return Err(merge_aside::abandon(aside, error, over_aside, progress).await),
    };
    if native_path(&source).is_none()
        && owed
            .barrier
            .as_ref()
            .is_some_and(copy_engine::Barrier::syncs_each_file)
        && let Err(error) = copy_engine::sync_files(written, cancellable).await
    {
        return Err(merge_aside::abandon(aside, error, over_aside, progress).await);
    }
    owed.nested = copied.nested;
    if let Some(aside) = merge_aside::finish(aside).await? {
        owed.owe(Deletion::Aside(aside));
    }
    if move_source {
        owed.owe(match copied.sources {
            Some(sources) => Deletion::Sources(source, sources),
            None => Deletion::Source(source, copied.left),
        });
    }
    Ok(owed)
}

pub(super) async fn can_trash(path: PathBuf) -> bool {
    run_blocking(move || Ok(fs().can_trash(&path)))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
pub(super) async fn merge_local_with(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    hooks: MergeHooks<'_>,
) -> Result<(), glib::Error> {
    merge_owing(
        source,
        target,
        move_source,
        &cancellable,
        affected_locations,
        &copy_engine::placed_budget(),
        hooks,
    )
    .await?
    .settle(None, &cancellable)
    .await
}

pub(super) fn progress_merge_hooks(
    progress: Option<Rc<TransferProgressTracker>>,
    names: NamePolicy,
    operation: Option<Rc<ActiveOperation>>,
    on_merged: &dyn Fn(MergePlan),
) -> MergeHooks<'_> {
    MergeHooks {
        names: names.clone(),
        copy_into_target: merge_copy(progress.clone(), names),
        stage_overwrite: releasing_stage_overwrite(operation),
        on_merged,
        progress,
    }
}

fn releasing_stage_overwrite(operation: Option<Rc<ActiveOperation>>) -> StageOverwrite {
    Rc::new(move |location, cancellable| {
        let operation = operation.clone();
        Box::pin(async move {
            if let (Some(operation), Some(path)) = (operation, location.native_path()) {
                operation
                    .release(vec![path.to_path_buf()], &cancellable)
                    .await;
            }
            trash_stage_overwrite(location, cancellable).await
        })
    })
}

pub(super) fn merge_copy(
    progress: Option<Rc<TransferProgressTracker>>,
    names: NamePolicy,
) -> CopyStep {
    Rc::new(
        move |source,
              target,
              MergeCopy {
                  moving,
                  durable,
                  placing,
              },
              cancellable| {
            let (progress, names) = (progress.clone(), names.clone());
            Box::pin(async move {
                match (native_path(&source), native_path(&target)) {
                    (Some(source_path), Some(target_path)) => {
                        let options = copy_engine::Options {
                            names,
                            manifest: moving,
                            durable,
                            ..copy_engine::Options::default()
                        };
                        let copied = copy_engine::copy_local(
                            source_path,
                            copy_engine::Destination::Merge(target_path, placing),
                            options,
                            progress,
                            &cancellable,
                        )
                        .await?;
                        Ok(MergeCopied {
                            sources: copied.manifest,
                            nested: copied.nested,
                            left: Vec::new(),
                        })
                    }
                    (source_path, _) => {
                        // Recorded before GIO copies it, so a change during the copy shows.
                        let sources = match source_path.as_ref().filter(|_| moving) {
                            Some(source_path) => Some(
                                copy_engine::snapshot(source_path.clone(), &cancellable).await?,
                            ),
                            None => None,
                        };
                        let left = LeftOut::default();
                        copy_recursively_with_progress(
                            source,
                            target,
                            CopyInto::Existing,
                            cancellable,
                            // GIO provides no creation provenance for destructive undo.
                            CopyRecord::Nothing,
                            progress.clone(),
                            GioNames::new(names, left.clone()),
                        )
                        .await?;
                        let left = match (&sources, source_path) {
                            (Some(sources), Some(path)) => {
                                left.keep_in(sources, &path);
                                Vec::new()
                            }
                            _ if moving => left.sources(),
                            _ => {
                                left.report(progress.as_deref());
                                Vec::new()
                            }
                        };
                        Ok(MergeCopied {
                            sources,
                            left,
                            ..MergeCopied::default()
                        })
                    }
                }
            })
        },
    )
}

#[cfg(test)]
pub(super) async fn merge_local(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    on_merged: &dyn Fn(MergePlan),
) -> Result<(), glib::Error> {
    merge_local_with(
        source,
        target,
        move_source,
        cancellable,
        None,
        progress_merge_hooks(None, NamePolicy::default(), None, on_merged),
    )
    .await
}
