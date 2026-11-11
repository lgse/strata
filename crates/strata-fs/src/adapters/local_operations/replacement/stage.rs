// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations) type StagedCopy =
    (Option<copy_engine::Manifest>, TransferReport);

#[derive(Clone)]
pub(in crate::adapters::local_operations) struct Copying {
    pub(in crate::adapters::local_operations) progress: Option<Rc<TransferProgressTracker>>,
    pub(in crate::adapters::local_operations) names: NamePolicy,
    pub(in crate::adapters::local_operations) placed_entries: Arc<AtomicUsize>,
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

pub(in crate::adapters::local_operations) struct StagedReplacement {
    pub(in crate::adapters::local_operations) original: ReplacedOriginal,
    pub(in crate::adapters::local_operations) displaced: Option<DisplacedOriginal>,
    pub(in crate::adapters::local_operations) placed: Option<PlacedSnapshot>,
}

#[derive(Clone, Copy)]
pub(in crate::adapters::local_operations) struct Durability {
    pub(in crate::adapters::local_operations) durable: bool,
    pub(in crate::adapters::local_operations) each_file: bool,
}

pub(in crate::adapters::local_operations) async fn set_aside_and_publish(
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

/// Where a staged replacement is published.
pub(in crate::adapters::local_operations) struct ReplaceTarget<'a> {
    pub(in crate::adapters::local_operations) plan: &'a Arc<ReplacePlan>,
    pub(in crate::adapters::local_operations) target_path: &'a Path,
    pub(in crate::adapters::local_operations) cancellable: &'a gio::Cancellable,
}

type Stage = (StagedSibling, OffMain<Arc<OwnedFd>>);
type Replaced = (StagedReplacement, StagedCopy);

/// Every failure discards the stage.
pub(in crate::adapters::local_operations) async fn replace_through_stage(
    target: ReplaceTarget<'_>,
    mut removal: Removal,
    (source_path, moving): (&Path, bool),
    (copying, durability): (Copying, Durability),
) -> Result<Replaced, glib::Error> {
    let options = copy_engine::Options {
        names: copying.names,
        manifest: moving,
        durable: durability.durable,
        ..copy_engine::Options::default()
    };
    let (staged, mut sources, report) = copy_engine::copy_to_stage(
        source_path.to_path_buf(),
        target.target_path.to_path_buf(),
        options,
        copying.progress,
        target.cancellable,
    )
    .await
    .map_err(|error| in_trash(error, removal))?;
    let staged_ownership = copy_engine::staged_ownership(&staged);
    let copied = (sources.take(), report);
    let mut stage = StageReplace {
        target,
        moving,
        placed_entries: copying.placed_entries,
        durability,
    };
    loop {
        let parent = OffMain::new(staged.parent.clone());
        let step = match removal {
            Removal::Trashed(trashed) => {
                return stage
                    .publish_trashed((staged, parent), trashed, copied)
                    .await;
            }
            Removal::Without(Displacement::SetAside) => {
                return stage.publish_aside((staged, parent), copied).await;
            }
            Removal::Trash => stage.trash_original(parent).await,
            Removal::Without(Displacement::Exchange) => match stage.exchange(&staged, parent).await
            {
                Ok(Ok(swapped)) => {
                    return Ok(stage.exchanged((staged, staged_ownership), swapped, copied));
                }
                Ok(Err(next)) => Ok(next),
                Err(error) => Err(error),
            },
        };
        match stage.synced_for_deletion(step, removal, &staged).await {
            Ok(next) => removal = next,
            Err(error) => {
                let cleanup = discard_incomplete_staged(staged).await;
                return Err(copy_failure_after_cleanup(error, cleanup));
            }
        }
    }
}

fn in_trash(error: glib::Error, removal: Removal) -> glib::Error {
    match removal {
        Removal::Trashed(_) => original_in_trash(&crate::services::gio_error_detail(&error)),
        _ => error,
    }
}

/// Publishes a stage over the original, removing the original the way it allows.
struct StageReplace<'a> {
    target: ReplaceTarget<'a>,
    moving: bool,
    placed_entries: Arc<AtomicUsize>,
    durability: Durability,
}

impl StageReplace<'_> {
    async fn publish_trashed(
        &self,
        (staged, parent): Stage,
        trashed: Option<ItemIdentity>,
        mut copied: StagedCopy,
    ) -> Result<Replaced, glib::Error> {
        let ReplaceTarget {
            plan,
            target_path,
            cancellable,
        } = &self.target;
        let ownership =
            publish_staged_replacement(staged, target_path.to_path_buf(), self.durability.durable)
                .await?;
        if let Some(sources) = &mut copied.0 {
            sources.copied_to(copy_engine::published(
                parent.into_inner(),
                plan.target_name.clone(),
                ownership.clone(),
            ));
        }
        let placed = if self.moving {
            None
        } else {
            copy_engine::placed_proven(
                target_path.to_path_buf(),
                ownership,
                self.placed_entries.clone(),
                cancellable,
            )
            .await
        };
        let replaced = StagedReplacement {
            original: ReplacedOriginal::Trashed(trashed),
            displaced: None,
            placed,
        };
        Ok((replaced, copied))
    }

    async fn publish_aside(
        &self,
        (staged, parent): Stage,
        mut copied: StagedCopy,
    ) -> Result<Replaced, glib::Error> {
        let plan = self.target.plan;
        let (replaced, ownership) = set_aside_and_publish(
            plan,
            staged,
            self.target.target_path,
            self.durability.durable,
        )
        .await?;
        if let Some(sources) = &mut copied.0 {
            sources.copied_to(copy_engine::published(
                parent.into_inner(),
                plan.target_name.clone(),
                ownership,
            ));
        }
        Ok((replaced, copied))
    }

    async fn trash_original(&self, parent: OffMain<Arc<OwnedFd>>) -> Result<Removal, glib::Error> {
        let step_plan = self.target.plan.clone();
        let (path, cancellable) = (
            self.target.target_path.to_path_buf(),
            self.target.cancellable.clone(),
        );
        run_blocking(move || trash_or_displace(&step_plan, &parent, &path, &cancellable)).await
    }

    async fn exchange(
        &self,
        staged: &StagedSibling,
        parent: OffMain<Arc<OwnedFd>>,
    ) -> Result<Result<Swapped, Removal>, glib::Error> {
        let step_plan = self.target.plan.clone();
        let (stage_name, copy) = (staged.name.clone(), staged.recorded());
        run_blocking(move || exchange_with_stage(&step_plan, &parent, &stage_name, copy)).await
    }

    fn exchanged(
        &self,
        (staged, staged_ownership): (StagedSibling, copy_engine::Ownership),
        (originals, original): Swapped,
        mut copied: StagedCopy,
    ) -> Replaced {
        let plan = self.target.plan;
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
        (replaced, copied)
    }

    /// The original is deleted after all, once the stage is on disk.
    async fn synced_for_deletion(
        &mut self,
        step: Result<Removal, glib::Error>,
        removal: Removal,
        staged: &StagedSibling,
    ) -> Result<Removal, glib::Error> {
        let Ok(Removal::Without(displacement)) = step else {
            return step;
        };
        if !matches!(removal, Removal::Trash) {
            return step;
        }
        if self.durability.durable || !self.durability.each_file {
            return step;
        }
        self.durability.durable = true;
        let stage = self.target.target_path.with_file_name(&staged.name);
        copy_engine::sync_files(vec![stage], self.target.cancellable)
            .await
            .map(|()| Removal::Without(displacement))
    }
}

type Swapped = (placement::Originals, Recorded);

fn trash_or_displace(
    plan: &ReplacePlan,
    parent: &OwnedFd,
    path: &Path,
    cancellable: &gio::Cancellable,
) -> Result<Removal, glib::Error> {
    let target = Entry::new(parent, &plan.target_name);
    let current = ensure_unchanged(target, &plan.target, &plan.target_pin)?;
    match trash::trash_verified(target, path, &current, cancellable) {
        Ok(trashed) => Ok(Removal::Trashed(trashed)),
        Err(error) if is_trash_unsupported_failure(false, &error) => {
            displacement_without_trash(parent, plan.directory(), &plan.target_display())
                .map(Removal::Without)
        }
        Err(error) => Err(error.error),
    }
}

fn exchange_with_stage(
    plan: &ReplacePlan,
    parent: &OwnedFd,
    stage_name: &OsStr,
    copy: Recorded,
) -> Result<Result<Swapped, Removal>, glib::Error> {
    let target = Entry::new(parent, &plan.target_name);
    let current = ensure_unchanged(target, &plan.target, &plan.target_pin)?;
    let name = plan.target_display();
    let stage = Entry::new(parent, stage_name);
    match placement::exchange_through(fs(), stage, target, copy.identity, copy.attributes) {
        Ok(Exchanged::Swapped(originals)) => Ok(Ok((originals, Recorded::of(&current)))),
        Ok(Exchanged::Refused) => displacement_or_refusal(
            placement::set_aside_if_safe(fs(), parent.as_fd(), plan.directory()),
            &name,
        )
        .map(|displacement| Err(Removal::Without(displacement))),
        Err(error) => Err(placement_failure(error, &name, could_not_replace)),
    }
}
