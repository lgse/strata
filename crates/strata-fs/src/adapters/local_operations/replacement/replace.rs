// SPDX-License-Identifier: MIT

use super::*;

type OnReplaced<'a> = &'a dyn Fn(ReplacedOriginal, Option<PlacedSnapshot>);

/// How a replace runs, and who hears about the original it removes.
pub(in crate::adapters::local_operations) struct Replacing<'a> {
    pub(in crate::adapters::local_operations) move_source: bool,
    pub(in crate::adapters::local_operations) cancellable: &'a gio::Cancellable,
    pub(in crate::adapters::local_operations) affected_locations: Option<&'a mut HashSet<Location>>,
    pub(in crate::adapters::local_operations) copying: Copying,
    pub(in crate::adapters::local_operations) on_replaced: OnReplaced<'a>,
}

/// `admin://` has no Trash to keep the original in, so a confirmed replace overwrites
/// a file in place; a folder is never replaced there.
async fn replace_as_administrator(
    (source, target): (gio::File, gio::File),
    move_source: bool,
    cancellable: &gio::Cancellable,
    on_replaced: OnReplaced<'_>,
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
        super::super::progress::move_file_with_progress(
            source,
            target,
            flags,
            cancellable.clone(),
            None,
        )
        .await?;
    } else {
        super::super::progress::copy_file_with_progress(
            source,
            target,
            flags,
            cancellable.clone(),
            None,
        )
        .await?;
    }
    on_replaced(ReplacedOriginal::Deleted, None);
    Ok(Owed::default())
}

/// Replaces `target` with `source`; the original is removed only once the
/// replacement is in place.
pub(in crate::adapters::local_operations) async fn replace_owing(
    source: gio::File,
    target: gio::File,
    replacing: Replacing<'_>,
) -> Result<Owed, glib::Error> {
    let Replacing {
        move_source,
        cancellable,
        affected_locations,
        copying,
        on_replaced,
    } = replacing;
    if let Some(locations) = affected_locations {
        locations.extend([&source, &target].into_iter().filter_map(location_for_file));
    }
    if target.has_uri_scheme(crate::model::ADMIN_SCHEME) {
        return replace_as_administrator((source, target), move_source, cancellable, on_replaced)
            .await;
    }
    let (source_path, target_path) = replaceable_paths(&source, &target)?;
    LocalReplace {
        source,
        source_path,
        target_path,
        move_source,
        cancellable,
        copying,
        on_replaced,
    }
    .run()
    .await
}

fn replaceable_paths(
    source: &gio::File,
    target: &gio::File,
) -> Result<(PathBuf, PathBuf), glib::Error> {
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
    Ok((source_path, target_path))
}

/// A replace between local paths.
struct LocalReplace<'a> {
    source: gio::File,
    source_path: PathBuf,
    target_path: PathBuf,
    move_source: bool,
    cancellable: &'a gio::Cancellable,
    copying: Copying,
    on_replaced: OnReplaced<'a>,
}

impl LocalReplace<'_> {
    async fn run(self) -> Result<Owed, glib::Error> {
        self.cancellable.set_error_if_cancelled()?;
        let (planned_source, planned_target) = (self.source_path.clone(), self.target_path.clone());
        let move_source = self.move_source;
        let Some(plan) =
            run_blocking(move || plan_replace(&planned_source, &planned_target, move_source))
                .await?
        else {
            return Ok(Owed::default());
        };
        let plan = OffMain::new(Arc::new(plan));
        let removal = match plan.strategy {
            ReplaceStrategy::Staged(removal) => removal,
            ReplaceStrategy::PlaceSource => match self.place_source_first(&plan).await? {
                Some(removal) => removal,
                None => return Ok(Owed::default()),
            },
        };
        self.through_stage(&plan, removal).await
    }

    /// `None` when the source took the original's place and nothing is left to copy.
    async fn place_source_first(
        &self,
        plan: &Arc<ReplacePlan>,
    ) -> Result<Option<Removal>, glib::Error> {
        self.cancellable.set_error_if_cancelled()?;
        let (step_plan, path, step_cancellable) = (
            plan.clone(),
            self.target_path.clone(),
            self.cancellable.clone(),
        );
        match run_blocking(move || place_source(&step_plan, &path, &step_cancellable)).await? {
            SourcePlaced::Trashed(trashed) => {
                (self.on_replaced)(ReplacedOriginal::Trashed(trashed), None);
                Ok(None)
            }
            // The source took its place by a rename, so it was not copied.
            SourcePlaced::SetAside(aside) => {
                (self.on_replaced)(ReplacedOriginal::Deleted, None);
                self.remove_set_aside(plan, aside).await;
                Ok(None)
            }
            SourcePlaced::CopyAfterTrash(trashed) => Ok(Some(Removal::Trashed(trashed))),
            SourcePlaced::PutBack => Ok(Some(Removal::Without(Displacement::SetAside))),
        }
    }

    async fn remove_set_aside(&self, plan: &ReplacePlan, aside: placement::SetAside) {
        let kept = self
            .target_path
            .with_file_name(&aside.originals.name)
            .join(&aside.name);
        let original = DisplacedOriginal::aside(plan.target_parent.clone(), aside);
        let Err(error) = remove_displaced_original(original, &Deadline::default()).await else {
            return;
        };
        let Some(progress) = &self.copying.progress else {
            return;
        };
        progress.report(TransferReport {
            kept: vec![KeptItem {
                location: Location::local(kept),
                cause: KeptCause::OriginalNotRemoved(crate::services::gio_error_detail(&error)),
            }],
            ..TransferReport::default()
        });
    }

    async fn through_stage(
        self,
        plan: &Arc<ReplacePlan>,
        removal: Removal,
    ) -> Result<Owed, glib::Error> {
        let mut owed = Owed::default();
        if self.move_source || !matches!(removal, Removal::Trashed(_)) {
            owed.barrier =
                Some(copy_engine::barrier(split_local_path(&self.target_path)?.0).await?);
        }
        let progress = self.copying.progress.clone();
        let durability = Durability {
            // A Trash attempt that falls back syncs the stage then.
            durable: self.move_source || matches!(removal, Removal::Without(_)),
            each_file: owed
                .barrier
                .as_ref()
                .is_some_and(copy_engine::Barrier::syncs_each_file),
        };
        let target = ReplaceTarget {
            plan,
            target_path: &self.target_path,
            cancellable: self.cancellable,
        };
        let (replaced, (sources, report)) = replace_through_stage(
            target,
            removal,
            (&self.source_path, self.move_source),
            (self.copying, durability),
        )
        .await?;
        (self.on_replaced)(replaced.original, replaced.placed);
        if let Some(progress) = &progress {
            progress.report(report);
        }
        if let Some(original) = replaced.displaced {
            owed.owe(Deletion::Displaced(original, self.target_path));
        }
        if let Some(sources) = sources {
            owed.owe(Deletion::Sources(self.source, sources));
        }
        Ok(owed)
    }
}

#[cfg(test)]
pub(in crate::adapters::local_operations) async fn replace_local_with(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    cancellable: gio::Cancellable,
    affected_locations: Option<&mut HashSet<Location>>,
    copying: Copying,
    on_replaced: &dyn Fn(ReplacedOriginal, Option<PlacedSnapshot>),
) -> Result<(), glib::Error> {
    let progress = copying.progress.clone();
    let replacing = Replacing {
        move_source,
        cancellable: &cancellable,
        affected_locations,
        copying,
        on_replaced,
    };
    replace_owing(source, target, replacing)
        .await?
        .settle(progress.as_deref(), &cancellable)
        .await
}

#[cfg(test)]
pub(in crate::adapters::local_operations) async fn replace_local(
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
