// SPDX-License-Identifier: MIT

use std::ops::ControlFlow;

use super::*;
use crate::services::PlacedSnapshot;
use replacement::MergePlan;

/// Pastes the items one at a time, in order.
pub(super) struct Sequential<'a> {
    plan: &'a Plan,
    state: State,
    records_copies: bool,
    written_paths: Vec<PathBuf>,
}

struct Item<'a> {
    index: usize,
    entry: &'a PasteItem,
    source: gio::File,
    target: gio::File,
    started: Reported,
    is_duplicate: bool,
    last: bool,
}

/// How one item is being transferred.
struct Transfer {
    taken: Taken,
    aliased: bool,
    replaced_permanently: Rc<Cell<bool>>,
    copied_ownership: Option<copy_engine::Ownership>,
}

type Transferred = Result<(gio::File, Owed), glib::Error>;

impl<'a> Sequential<'a> {
    pub(super) async fn new(plan: &'a Plan, state: State) -> Self {
        Self {
            records_copies: records_copies(plan).await,
            plan,
            state,
            written_paths: Vec::new(),
        }
    }

    pub(super) async fn run(mut self) {
        let plan = self.plan;
        for index in 0..plan.request.items.len() {
            if plan.cancellable.is_cancelled() || self.state.settling.failure().is_some() {
                self.stop_before(index).await;
                return;
            }
            if self.paste_item(index).await.is_break() {
                return;
            }
        }
        self.finish().await;
    }

    async fn paste_item(&mut self, index: usize) -> ControlFlow<()> {
        let plan = self.plan;
        let source = plan.sources[index].clone();
        let started = plan.progress.reported();
        let Some(target) = plan.targets[index].clone() else {
            self.fail_unnamed().await;
            return ControlFlow::Break(());
        };
        let item = Item {
            index,
            entry: &plan.request.items[index],
            is_duplicate: !plan.request.move_sources && source.equal(&target),
            source,
            target,
            started,
            last: index + 1 == plan.request.items.len(),
        };
        if is_noop(plan, &item).await {
            self.skip_noop(&item).await;
            return ControlFlow::Continue(());
        }
        let mut transfer = Transfer::new(&item);
        let default_location = self.note_target(&item);
        let undone = item.entry.conflict != TransferConflict::Merge
            && undone_when_cancelled(&item.source, &item.target);
        let result = self.transfer(&item, &mut transfer, default_location).await;
        if let Err(error) = &result
            && let Some(kind) = unheld(error)
        {
            plan.leave_out(&item, kind);
            return ControlFlow::Continue(());
        }
        match result {
            Ok(transferred) => {
                self.settle(&item, transfer, transferred).await;
                ControlFlow::Continue(())
            }
            Err(error) => {
                self.stop_failed(&item, undone, &error).await;
                ControlFlow::Break(())
            }
        }
    }

    async fn skip_noop(&mut self, item: &Item<'_>) {
        let done = Done {
            index: item.index,
            moved_to: None,
        };
        self.state
            .settling
            .owe(done, Owed::default(), 0, item.last)
            .await;
        self.plan.finish_untransferred(item);
    }

    fn note_target(&mut self, item: &Item<'_>) -> Option<Location> {
        let affected_locations = &mut self.state.affected_locations;
        affected_locations.insert(item.entry.source.clone());
        let default_location = location_for_file(&item.target);
        if let Some(target) = default_location.clone() {
            affected_locations.insert(target);
        }
        if let Some(path) = item.target.path() {
            self.written_paths.push(path);
        }
        default_location
    }

    async fn transfer(
        &mut self,
        item: &Item<'_>,
        transfer: &mut Transfer,
        default_location: Option<Location>,
    ) -> Transferred {
        let plan = self.plan;
        if plan.refuses_link(item.index) {
            return Err(glib::Error::new(Unheld::Symlink, ""));
        }
        if item.is_duplicate {
            return plan.copy_new(item, transfer).await;
        }
        match item.entry.conflict {
            TransferConflict::Merge => self.merge(item).await,
            TransferConflict::ReplaceExisting => {
                self.replace(item, transfer, default_location).await
            }
            _ => self.place_new(item, transfer).await,
        }
    }

    async fn merge(&mut self, item: &Item<'_>) -> Transferred {
        let plan = self.plan;
        let on_merged = |merged| plan.report_merged(item.entry, merged);
        let merging = Merging {
            move_source: plan.request.move_sources,
            cancellable: &plan.cancellable,
            affected_locations: Some(&mut self.state.affected_locations),
            placed_entries: &plan.copying.placed_entries,
            hooks: progress_merge_hooks(
                Some(plan.progress.clone()),
                plan.names.clone(),
                Some(plan.operation.clone()),
                &on_merged,
            ),
        };
        merge_owing(item.source.clone(), item.target.clone(), merging)
            .await
            .map(|owed| (item.target.clone(), owed))
    }

    async fn replace(
        &mut self,
        item: &Item<'_>,
        transfer: &Transfer,
        replaced_target: Option<Location>,
    ) -> Transferred {
        let plan = self.plan;
        let replaced = Replaced {
            emit: plan.emit.clone(),
            request_id: plan.request.id,
            moving: plan.request.move_sources,
            source: &item.entry.source,
            target: replaced_target,
            permanently: transfer.replaced_permanently.clone(),
        };
        let replacing = Replacing {
            move_source: plan.request.move_sources,
            cancellable: &plan.cancellable,
            affected_locations: Some(&mut self.state.affected_locations),
            copying: plan.copying.clone(),
            on_replaced: &move |original, placed| replaced.report(original, placed),
        };
        replace_owing(item.source.clone(), item.target.clone(), replacing)
            .await
            .map(|owed| (item.target.clone(), owed))
    }

    async fn place_new(&mut self, item: &Item<'_>, transfer: &mut Transfer) -> Transferred {
        let plan = self.plan;
        if transfer.taken == Taken::Fail
            && case_variants::taken_by(&item.target, self.state.variants.earlier(item.index)).await
        {
            transfer.taken = Taken::Number;
            transfer.aliased = true;
        }
        if !plan.request.move_sources {
            return plan.copy_new(item, transfer).await;
        }
        move_owing(
            item.source.clone(),
            item.target.clone(),
            Moving::new(
                transfer.taken,
                plan.cancellable.clone(),
                plan.copying.clone(),
            ),
        )
        .await
        .map(|moved| (moved.target, moved.owed))
    }

    async fn settle(
        &mut self,
        item: &Item<'_>,
        mut transfer: Transfer,
        transferred: (gio::File, Owed),
    ) {
        let (target, owed) = transferred;
        let plan = self.plan;
        let target_location = location_for_file(&target);
        if let Some(target) = target_location.clone() {
            self.state.affected_locations.insert(target);
        }
        self.state.variants.saved(item.index, &target).await;
        plan.report_rename(item, &transfer, &target);
        // A merge reports its written paths through Merged instead:
        // copy undo trashes recorded locations, and the merged folder
        // held pre-existing contents that must not be trashed.
        // Undoing a copy deletes it, which must never remove the only
        // copy left after an original was replaced without Trash.
        let created_location = target_location
            .clone()
            .filter(|_| plan.creates_copy(item.entry) && !transfer.replaced_permanently.get());
        if self.records_copies && item.entry.conflict != TransferConflict::ReplaceExisting {
            plan.record_copy(
                item,
                transfer.copied_ownership.take(),
                created_location.as_ref(),
            )
            .await;
        }
        let done = Done {
            index: item.index,
            moved_to: target_location,
        };
        let reported = plan.progress.reported().since(item.started);
        plan.progress.finish_item(
            reported,
            plan.sizes.sizes[item.index],
            plan.sizes.files[item.index],
            created_location,
        );
        self.state
            .settling
            .owe(done, owed, reported.bytes, item.last)
            .await;
    }

    async fn stop_before(self, index: usize) {
        let mut state = self.state;
        state.settling.finish().await;
        let stop = TransferStop {
            completed: state.settling.completed(),
            failed: state.settling.failed(),
            not_attempted: self.plan.request.items[index..]
                .iter()
                .map(|item| item.source.clone())
                .collect(),
            affected_locations: state.affected_locations,
            failure: state.settling.failure(),
            skip_flush: false,
        };
        self.plan.stop(&self.written_paths, stop).await;
    }

    async fn fail_unnamed(&mut self) {
        let plan = self.plan;
        self.state.settling.finish().await;
        let flush_error = flush_written_roots(&self.written_paths, &plan.emit, plan.request.id)
            .await
            .err();
        let flush_error = joined_failures(self.state.settling.failure(), flush_error);
        (plan.emit)(OperationEvent::Failed {
            request_id: plan.request.id,
            message: match flush_error {
                Some(error) => {
                    rust_i18n::t!("A clipboard item has no file name. %{error}", error = error)
                        .into_owned()
                }
                None => crate::i18n::tr("A clipboard item has no file name"),
            },
            password_failure: None,
        });
    }

    async fn stop_failed(&mut self, item: &Item<'_>, undone: bool, error: &glib::Error) {
        let plan = self.plan;
        let settling = &mut self.state.settling;
        settling.finish().await;
        let (failure, skip_flush) =
            copy_failure_on_fat32(error, plan.profile.max_file_size.is_some());
        let untouched = undone && was_cancelled(error);
        let mut failed = settling.failed();
        if !untouched {
            failed.push(item.entry.source.clone());
        }
        let stop = TransferStop {
            completed: settling.completed(),
            failed,
            not_attempted: plan.request.items[item.index + usize::from(!untouched)..]
                .iter()
                .map(|item| item.source.clone())
                .collect(),
            affected_locations: std::mem::take(&mut self.state.affected_locations),
            failure: joined_failures(failure, self.state.settling.failure()),
            skip_flush,
        };
        plan.stop(&self.written_paths, stop).await;
    }

    async fn finish(self) {
        let mut state = self.state;
        state.settling.finish().await;
        let Some(failure) = state.settling.failure() else {
            self.plan.finish(self.written_paths, state).await;
            return;
        };
        let stop = TransferStop {
            completed: state.settling.completed(),
            failed: state.settling.failed(),
            not_attempted: Vec::new(),
            affected_locations: state.affected_locations,
            failure: Some(failure),
            skip_flush: false,
        };
        self.plan.stop(&self.written_paths, stop).await;
    }
}

/// Undo deletes what a paste with a merge copied beside it where there is no Trash.
async fn records_copies(plan: &Plan) -> bool {
    let merges = plan
        .request
        .items
        .iter()
        .any(|item| item.conflict == TransferConflict::Merge);
    if plan.request.move_sources || !merges {
        return false;
    }
    match native_path(&plan.destination) {
        Some(path) => !can_trash(path).await,
        None => false,
    }
}

async fn is_noop(plan: &Plan, item: &Item<'_>) -> bool {
    if item.is_duplicate {
        return item.entry.conflict == TransferConflict::ReplaceExisting;
    }
    let (source, destination, target) = (
        item.source.clone(),
        plan.destination.clone(),
        item.target.clone(),
    );
    gio::spawn_blocking(move || transfer_is_noop(&source, &destination, &target))
        .await
        .unwrap_or(false)
}

impl Transfer {
    fn new(item: &Item<'_>) -> Self {
        let needs_unique_target =
            item.is_duplicate || item.entry.conflict == TransferConflict::KeepBoth;
        Self {
            taken: if needs_unique_target {
                Taken::Number
            } else {
                Taken::Fail
            },
            aliased: false,
            replaced_permanently: Rc::new(Cell::new(false)),
            copied_ownership: None,
        }
    }
}

impl Plan {
    /// A destination known to hold no links is not asked to make
    /// one; a move within its filesystem makes none.
    fn refuses_link(&self, index: usize) -> bool {
        let is_link = self.sizes.kinds[index] == Some(gio::FileType::SymbolicLink);
        if !is_link || self.profile.symlinks != Some(false) {
            return false;
        }
        self.crossing[index] || !self.request.move_sources
    }

    fn creates_copy(&self, entry: &PasteItem) -> bool {
        !self.request.move_sources && entry.conflict != TransferConflict::Merge
    }

    async fn copy_new(&self, item: &Item<'_>, transfer: &mut Transfer) -> Transferred {
        copy_new_with_ownership(
            item.source.clone(),
            item.target.clone(),
            NewCopy {
                taken: transfer.taken,
                cancellable: self.cancellable.clone(),
                progress: Some(self.progress.clone()),
                names: self.names.clone(),
                left: None,
            },
        )
        .await
        .map(|(target, ownership)| {
            transfer.copied_ownership = ownership;
            (target, Owed::default())
        })
    }

    fn finish_untransferred(&self, item: &Item<'_>) {
        self.progress.finish_item(
            self.progress.reported().since(item.started),
            self.sizes.sizes[item.index],
            self.sizes.files[item.index],
            None,
        );
    }

    fn leave_out(&self, item: &Item<'_>, kind: SpecialKind) {
        self.progress.report(left_out(
            &item.entry.source,
            &self.destination,
            kind,
            self.request.move_sources,
        ));
        self.finish_untransferred(item);
    }

    fn report_merged(&self, entry: &PasteItem, merged: MergePlan) {
        for (location, identity) in merged.originals {
            (self.emit)(OperationEvent::ItemTrashed {
                request_id: self.request.id,
                location,
                identity,
            });
        }
        (self.emit)(OperationEvent::Merged {
            request_id: self.request.id,
            source: entry.source.clone(),
            created: merged.created,
            overwritten: merged.overwritten,
            placed: merged.placed,
        });
    }

    fn report_rename(&self, item: &Item<'_>, transfer: &Transfer, target: &gio::File) {
        let planned = &self.planned[item.index];
        let cause = planned
            .as_ref()
            .and_then(|planned| planned.cause)
            .or(transfer.aliased.then_some(RenameCause::Alias));
        let (Some(cause), Some(name)) = (cause, transfer_source_name(&self.sources[item.index]))
        else {
            return;
        };
        // Keep Both's numbers go unreported; the engine reports a
        // name the mount refused from the planned one.
        let saved_as = match (transfer.taken, planned) {
            (Taken::Fail, Some(planned)) => self.destination.child(&planned.target),
            _ => target.clone(),
        };
        self.progress.report(TransferReport {
            renamed: renamed_item(&self.destination.child(name), &saved_as, cause)
                .into_iter()
                .collect(),
            ..TransferReport::default()
        });
    }

    async fn record_copy(
        &self,
        item: &Item<'_>,
        ownership: Option<copy_engine::Ownership>,
        created_location: Option<&Location>,
    ) {
        let Some(location) = created_location else {
            return;
        };
        let (Some(path), Some(ownership)) = (location.native_path(), ownership) else {
            return;
        };
        let Some(placed) = copy_engine::placed_proven(
            path.to_path_buf(),
            ownership,
            self.copying.placed_entries.clone(),
            &self.cancellable,
        )
        .await
        else {
            return;
        };
        (self.emit)(OperationEvent::Merged {
            request_id: self.request.id,
            source: item.entry.source.clone(),
            created: Vec::new(),
            overwritten: Vec::new(),
            placed: HashMap::from([(location.clone(), placed)]),
        });
    }
}

/// Reports an original that a replacing paste set aside.
struct Replaced<'a> {
    emit: Rc<dyn Fn(OperationEvent)>,
    request_id: OperationRequestId,
    moving: bool,
    source: &'a Location,
    target: Option<Location>,
    permanently: Rc<Cell<bool>>,
}

impl Replaced<'_> {
    fn report(&self, original: ReplacedOriginal, placed: Option<PlacedSnapshot>) {
        let identity = match original {
            ReplacedOriginal::Trashed(identity) => identity,
            ReplacedOriginal::Deleted => {
                self.permanently.set(true);
                return;
            }
        };
        if let (Some(target), Some(identity)) = (&self.target, identity) {
            (self.emit)(OperationEvent::ItemTrashed {
                request_id: self.request_id,
                location: target.clone(),
                identity,
            });
        }
        // Copies restore through merge undo; moves attach the ItemTrashed
        // identity to their move-back record instead.
        if self.moving {
            return;
        }
        if let Some(target) = &self.target {
            (self.emit)(OperationEvent::Merged {
                request_id: self.request_id,
                source: self.source.clone(),
                created: Vec::new(),
                overwritten: vec![target.clone()],
                placed: placed
                    .map(|placed| HashMap::from([(target.clone(), placed)]))
                    .unwrap_or_default(),
            });
        }
    }
}
