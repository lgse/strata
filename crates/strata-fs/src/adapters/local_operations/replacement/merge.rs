// SPDX-License-Identifier: MIT

use super::*;

mod classify;
mod copy;

pub(in crate::adapters::local_operations) use classify::*;
pub(in crate::adapters::local_operations) use copy::*;

pub(in crate::adapters::local_operations) type CopyStep = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        MergeCopy,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<MergeCopied, glib::Error>>>>,
>;

#[derive(Clone, Default)]
pub(in crate::adapters::local_operations) struct MergeCopy {
    pub(in crate::adapters::local_operations) moving: bool,
    pub(in crate::adapters::local_operations) durable: bool,
    pub(in crate::adapters::local_operations) placing: Option<Arc<copy_engine::Placing>>,
}

#[derive(Default)]
pub(in crate::adapters::local_operations) struct MergeCopied {
    pub(in crate::adapters::local_operations) sources: Option<copy_engine::Manifest>,
    pub(in crate::adapters::local_operations) nested: Vec<copy_engine::Barrier>,
    pub(in crate::adapters::local_operations) left: Vec<gio::File>,
}

/// What a merge will write: `created` are the topmost destination paths with
/// no existing counterpart, `overwritten` are leaf collisions whose originals
/// go to Trash, or aside where Trash is unavailable.
#[derive(Default)]
pub(in crate::adapters::local_operations) struct MergePlan {
    pub(in crate::adapters::local_operations) created: Vec<Location>,
    pub(in crate::adapters::local_operations) overwritten: Vec<Location>,
    pub(in crate::adapters::local_operations) originals: HashMap<Location, ItemIdentity>,
    pub(in crate::adapters::local_operations) placed: HashMap<Location, PlacedSnapshot>,
}

/// Unsupported native Trash triggers the set-aside fallback.
pub(in crate::adapters::local_operations) type StageOverwrite = Rc<
    dyn Fn(
        Location,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ItemIdentity>, TrashError>>>>,
>;

pub(in crate::adapters::local_operations) fn trash_stage_overwrite(
    location: Location,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<Option<ItemIdentity>, TrashError>>>> {
    let file = gio_file_for_location(&location);
    Box::pin(async move { trash_with_cleanup(&file, &cancellable).await })
}

pub(in crate::adapters::local_operations) struct MergeHooks<'a> {
    pub(in crate::adapters::local_operations) names: NamePolicy,
    pub(in crate::adapters::local_operations) copy_into_target: CopyStep,
    pub(in crate::adapters::local_operations) stage_overwrite: StageOverwrite,
    pub(in crate::adapters::local_operations) on_merged: &'a dyn Fn(MergePlan),
    pub(in crate::adapters::local_operations) progress: Option<Rc<TransferProgressTracker>>,
}

pub(in crate::adapters::local_operations) async fn can_trash(path: PathBuf) -> bool {
    run_blocking(move || Ok(fs().can_trash(&path)))
        .await
        .unwrap_or(false)
}

pub(in crate::adapters::local_operations) fn progress_merge_hooks(
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

/// How a merge runs, and the hooks that copy, stage and report it.
pub(in crate::adapters::local_operations) struct Merging<'a> {
    pub(in crate::adapters::local_operations) move_source: bool,
    pub(in crate::adapters::local_operations) cancellable: &'a gio::Cancellable,
    pub(in crate::adapters::local_operations) affected_locations: Option<&'a mut HashSet<Location>>,
    pub(in crate::adapters::local_operations) placed_entries: &'a Arc<AtomicUsize>,
    pub(in crate::adapters::local_operations) hooks: MergeHooks<'a>,
}

/// Publish completed children without replacing concurrent arrivals.
/// Already-published children survive cancellation.
pub(in crate::adapters::local_operations) async fn merge_owing(
    source: gio::File,
    target: gio::File,
    merging: Merging<'_>,
) -> Result<Owed, glib::Error> {
    let Merging {
        move_source,
        cancellable,
        affected_locations,
        placed_entries,
        hooks,
    } = merging;
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
    let merge = Merge {
        target_path: target.path(),
        source,
        target,
        move_source,
        cancellable,
        placed_entries,
        hooks,
    };
    merge.run(plan).await
}

#[cfg(test)]
pub(in crate::adapters::local_operations) async fn merge_local_with(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    hooks: MergeHooks<'_>,
) -> Result<(), glib::Error> {
    let merging = Merging {
        move_source,
        cancellable: &cancellable,
        affected_locations,
        placed_entries: &copy_engine::placed_budget(),
        hooks,
    };
    merge_owing(source, target, merging)
        .await?
        .settle(None, &cancellable)
        .await
}

#[cfg(test)]
pub(in crate::adapters::local_operations) async fn merge_local(
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

/// One classified merge, from staging the overwritten originals to copying the source in.
struct Merge<'a> {
    source: gio::File,
    target: gio::File,
    target_path: Option<PathBuf>,
    move_source: bool,
    cancellable: &'a gio::Cancellable,
    placed_entries: &'a Arc<AtomicUsize>,
    hooks: MergeHooks<'a>,
}

/// What the merge copies with, once the originals are out of the way.
struct Prepared {
    aside: Option<merge_aside::MergeAside>,
    placing: Option<Arc<copy_engine::Placing>>,
    written: Vec<PathBuf>,
}

/// The overwritten originals, gone to Trash or set aside.
#[derive(Default)]
struct Staged {
    trashed: Vec<Location>,
    originals: HashMap<Location, ItemIdentity>,
    aside: Option<merge_aside::MergeAside>,
}

impl Merge<'_> {
    async fn run(self, plan: MergePlan) -> Result<Owed, glib::Error> {
        let prepared = self.prepare(plan).await?;
        self.copy_in(prepared).await
    }

    /// Stages the overwritten originals and reports the plan; a failure is already abandoned.
    async fn prepare(&self, plan: MergePlan) -> Result<Prepared, glib::Error> {
        // Originals go to Trash, or aside where Trash is unavailable, before
        // anything is copied.
        let mut staged = Staged::default();
        if let Err(error) = self.stage_overwritten(&plan, &mut staged).await {
            (self.hooks.on_merged)(MergePlan {
                overwritten: staged.trashed,
                originals: staged.originals,
                ..MergePlan::default()
            });
            let progress = self.hooks.progress.as_deref();
            return Err(merge_aside::abandon(staged.aside, error, HashMap::new(), progress).await);
        }
        let written: Vec<PathBuf> = plan
            .created
            .iter()
            .chain(&plan.overwritten)
            .filter_map(|location| location.native_path().map(Path::to_path_buf))
            .collect();
        let records_created = self.records_created(&plan, &staged).await;
        let records = !self.move_source && (!staged.trashed.is_empty() || records_created);
        let placing = self.placing(&plan, &staged, (records, records_created));
        (self.hooks.on_merged)(MergePlan {
            created: plan.created,
            overwritten: staged.trashed,
            originals: staged.originals,
            ..MergePlan::default()
        });
        Ok(Prepared {
            aside: staged.aside,
            placing,
            written,
        })
    }

    async fn copy_in(self, prepared: Prepared) -> Result<Owed, glib::Error> {
        let Prepared {
            aside,
            placing,
            written,
        } = prepared;
        let progress = self.hooks.progress.as_deref();
        let mut owed = Owed::default();
        if let Some(path) =
            native_path(&self.target).filter(|_| self.move_source || aside.is_some())
        {
            match copy_engine::barrier(path).await {
                Ok(barrier) => owed.barrier = Some(barrier),
                Err(error) => {
                    return Err(merge_aside::abandon(aside, error, HashMap::new(), progress).await);
                }
            }
        }
        let copying = MergeCopy {
            moving: self.move_source,
            durable: owed.barrier.is_some(),
            placing: placing.clone(),
        };
        let copied = (self.hooks.copy_into_target)(
            self.source.clone(),
            self.target.clone(),
            copying,
            self.cancellable.clone(),
        )
        .await;
        let over_aside = placing
            .as_ref()
            .map(|placing| placing.over_aside())
            .unwrap_or_default();
        if let Some(placing) = placing {
            self.report_placed(&placing);
        }
        let copied = match copied {
            Ok(copied) => copied,
            Err(error) => {
                return Err(merge_aside::abandon(aside, error, over_aside, progress).await);
            }
        };
        if let Err(error) = self.sync_written(&owed, written).await {
            return Err(merge_aside::abandon(aside, error, over_aside, progress).await);
        }
        owed.nested = copied.nested;
        if let Some(aside) = merge_aside::finish(aside).await? {
            owed.owe(Deletion::Aside(aside));
        }
        if self.move_source {
            owed.owe(match copied.sources {
                Some(sources) => Deletion::Sources(self.source, sources),
                None => Deletion::Source(self.source, copied.left),
            });
        }
        Ok(owed)
    }

    async fn stage_overwritten(
        &self,
        plan: &MergePlan,
        staged: &mut Staged,
    ) -> Result<(), glib::Error> {
        for location in &plan.overwritten {
            if self.cancellable.is_cancelled() {
                return Err(cancelled_local_operation());
            }
            self.stage_original(location, staged).await?;
        }
        Ok(())
    }

    async fn stage_original(
        &self,
        location: &Location,
        staged: &mut Staged,
    ) -> Result<(), glib::Error> {
        let error =
            match (self.hooks.stage_overwrite)(location.clone(), self.cancellable.clone()).await {
                Ok(identity) => {
                    staged.trashed.push(location.clone());
                    staged
                        .originals
                        .extend(identity.map(|identity| (location.clone(), identity)));
                    return Ok(());
                }
                Err(error) => error,
            };
        match (self.target_path.as_deref(), location.native_path()) {
            (Some(root), Some(path)) if is_trash_unsupported_failure(false, &error) => {
                merge_aside::set_aside(&mut staged.aside, root, path, location.display_name()).await
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
        }
    }

    /// Undo trashes what a merge created, so it needs a record of it only
    /// where originals could not go to the Trash, or none were offered it.
    async fn records_created(&self, plan: &MergePlan, staged: &Staged) -> bool {
        let Some(root) = &self.target_path else {
            return false;
        };
        if self.move_source || plan.created.is_empty() {
            return false;
        }
        if staged.aside.is_some() {
            return true;
        }
        staged.trashed.is_empty() && !can_trash(root.clone()).await
    }

    fn placing(
        &self,
        plan: &MergePlan,
        staged: &Staged,
        (records, records_created): (bool, bool),
    ) -> Option<Arc<copy_engine::Placing>> {
        let root = self
            .target_path
            .as_ref()
            .filter(|_| records || staged.aside.is_some())?;
        let path = |location: &Location| location.native_path().map(Path::to_path_buf);
        let created = plan.created.iter().filter(|_| records_created);
        let wanted: HashSet<_> = staged
            .trashed
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
        let aside = staged
            .aside
            .as_ref()
            .map(merge_aside::MergeAside::paths)
            .unwrap_or_default();
        Some(copy_engine::Placing::new(
            root.clone(),
            wanted,
            unwanted,
            aside,
            self.placed_entries.clone(),
        ))
    }

    fn report_placed(&self, placing: &copy_engine::Placing) {
        let placed: HashMap<_, _> = placing
            .take()
            .into_iter()
            .map(|(path, placed)| (Location::local(path), placed))
            .collect();
        if !placed.is_empty() {
            (self.hooks.on_merged)(MergePlan {
                placed,
                ..MergePlan::default()
            });
        }
    }

    async fn sync_written(&self, owed: &Owed, written: Vec<PathBuf>) -> Result<(), glib::Error> {
        let syncs_each_file = owed
            .barrier
            .as_ref()
            .is_some_and(copy_engine::Barrier::syncs_each_file);
        if native_path(&self.source).is_some() || !syncs_each_file {
            return Ok(());
        }
        copy_engine::sync_files(written, self.cancellable).await
    }
}
