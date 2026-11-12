// SPDX-License-Identifier: MIT

use std::ops::ControlFlow;

use super::*;
use crate::adapters::bookmarks::DeletedPins;

pub(in crate::adapters::local_operations) enum Reverted {
    Restored(TransferReport, Box<ItemIdentity>),
    Kept,
}

async fn remove_unchanged(
    (path, placed): (&Path, &PlacedSnapshot),
    restoring: &Restoring<'_>,
    deleting: &mut bool,
) -> Result<bool, glib::Error> {
    let removed = copy_engine::remove_placed(
        path.to_path_buf(),
        placed.clone(),
        restoring.unlinks.clone(),
        restoring.cancellable,
    )
    .await;
    *deleting = removed.removed;
    removed.result?;
    if let Some((path, error)) = removed.kept.iter().find_map(|(path, cause)| {
        let error = undeleted_reason(cause)?;
        Some((path, error))
    }) {
        return Err(io_error(rust_i18n::t!(
            "Could not delete “%{name}”: %{error}",
            name = path.file_name().unwrap_or_default().to_string_lossy(),
            error = error
        )));
    }
    Ok(removed.kept.is_empty())
}

fn undeleted_reason(cause: &KeptCause) -> Option<String> {
    match cause {
        KeptCause::Undeletable(error) => Some(crate::services::io_error_detail(
            &io::Error::from_raw_os_error(*error),
        )),
        KeptCause::StillOpen(holder) => Some(crate::services::error_detail(holders::held(
            holder.as_ref(),
        ))),
        _ => None,
    }
}

/// An item a merge overwrote, and the original staged in Trash.
struct Overwrite<'a> {
    location: &'a Location,
    placed: Option<&'a PlacedSnapshot>,
    staged: &'a RestoreEntry,
}

async fn undo_merged_overwrite(
    overwrite: Overwrite<'_>,
    restoring: &Restoring<'_>,
    affected_locations: &mut HashSet<Location>,
    deleting: &mut bool,
) -> Result<Reverted, glib::Error> {
    let freed = match (overwrite.placed, overwrite.location.native_path()) {
        (Some(placed), Some(path)) => remove_unchanged((path, placed), restoring, deleting).await?,
        // Nothing tells what is there, so only an empty path takes the
        // original back.
        _ => is_free(overwrite.location, restoring.cancellable).await?,
    };
    if !freed {
        return Ok(Reverted::Kept);
    }
    restore_trash_entry(overwrite.staged, restoring, affected_locations)
        .await
        .map(|(report, trashed)| Reverted::Restored(report, Box::new(trashed)))
}

async fn query_type(
    file: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<gio::FileInfo, glib::Error> {
    await_cancellable(file, cancellable, |file, cancellable, result| {
        file.query_info_async(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
}

async fn is_free(location: &Location, cancellable: &gio::Cancellable) -> Result<bool, glib::Error> {
    match query_type(&gio_file_for_location(location), cancellable).await {
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(true),
        Err(error) => Err(error),
        Ok(_) => Ok(false),
    }
}

pub(in crate::adapters::local_operations) type StagedOriginalLookup = Rc<
    dyn Fn(
        Location,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<RestoreEntry, glib::Error>>>>,
>;

pub(in crate::adapters::local_operations) async fn run_merge_undo(
    request_id: OperationRequestId,
    (created, overwritten, placed): (
        Vec<Location>,
        Vec<Location>,
        HashMap<Location, PlacedSnapshot>,
    ),
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
    staged_original: StagedOriginalLookup,
) {
    let pending: Vec<Location> = overwritten.iter().chain(&created).cloned().collect();
    let mut affected_locations = HashSet::from([Location::uri("trash:///")]);
    for location in &pending {
        if let Some(parent) = location.parent() {
            affected_locations.insert(parent);
        }
    }
    let deleted_pins = DeletedPins::default();
    let context = RestoreContext::current().await;
    let undo = MergeUndo {
        unlinks: Arc::default(),
        context,
        deleted_pins,
        request_id,
        pending,
        placed,
        emit,
        cancellable,
        staged_original,
        affected_locations,
        outcome: Outcome::default(),
    };
    undo.run(&overwritten, &created).await;
}

#[derive(Default)]
struct Outcome {
    completed_locations: Vec<Location>,
    restored: Vec<Location>,
    removed: Vec<Location>,
    failed_locations: Vec<Location>,
    errors: Vec<String>,
    completed: usize,
}

/// Reverts one merge. The pins of what it deleted are forgotten only once it has reported.
struct MergeUndo {
    unlinks: Arc<copy_engine::Unlinks>,
    context: RestoreContext,
    deleted_pins: DeletedPins,
    request_id: OperationRequestId,
    pending: Vec<Location>,
    placed: HashMap<Location, PlacedSnapshot>,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
    staged_original: StagedOriginalLookup,
    affected_locations: HashSet<Location>,
    outcome: Outcome,
}

/// What removing one created item came to.
#[derive(Default)]
struct Removal {
    trashed_identity: Option<ItemIdentity>,
    deleting: bool,
    kept: bool,
}

impl MergeUndo {
    async fn run(mut self, overwritten: &[Location], created: &[Location]) {
        // Copies only: the incoming item duplicates the surviving source, so it
        // is deleted when unchanged before the original comes back from Trash.
        for (index, location) in overwritten.iter().enumerate() {
            if self.cancellable.is_cancelled() {
                self.cancel(index, None);
                return;
            }
            if self.revert_overwrite(index, location).await.is_break() {
                return;
            }
        }
        for (index, location) in created.iter().enumerate() {
            let position = overwritten.len() + index;
            if self.cancellable.is_cancelled() {
                self.cancel(position, None);
                return;
            }
            if self.remove_created(position, location).await.is_break() {
                return;
            }
        }
        self.finish();
    }

    fn total(&self) -> usize {
        self.pending.len()
    }

    /// Reports the cancel; `touched` was partly undone, so it failed rather than waits.
    fn cancel(&mut self, position: usize, touched: Option<&Location>) {
        if let Some(location) = touched {
            self.outcome.failed_locations.push(location.clone());
        }
        let not_attempted = self.pending[position + usize::from(touched.is_some())..].to_vec();
        self.emit_cancelled(not_attempted);
    }

    fn emit_cancelled(&mut self, not_attempted: Vec<Location>) {
        (self.emit)(cancelled_event(
            self.request_id,
            std::mem::take(&mut self.outcome.completed_locations),
            std::mem::take(&mut self.outcome.failed_locations),
            not_attempted,
            std::mem::take(&mut self.affected_locations),
        ));
    }

    fn fail(&mut self, location: &Location, error: &glib::Error) {
        self.outcome.errors.push(item_error(
            &location.display_name(),
            crate::services::gio_error_detail(error),
        ));
        self.outcome.failed_locations.push(location.clone());
    }

    async fn revert_overwrite(&mut self, index: usize, location: &Location) -> ControlFlow<()> {
        let mut deleting = false;
        let result = match (self.staged_original)(location.clone(), self.cancellable.clone()).await
        {
            Ok(staged) => {
                let restoring = Restoring {
                    context: &self.context,
                    unlinks: &self.unlinks,
                    cancellable: &self.cancellable,
                };
                let overwrite = Overwrite {
                    location,
                    placed: self.placed.get(location),
                    staged: &staged,
                };
                undo_merged_overwrite(
                    overwrite,
                    &restoring,
                    &mut self.affected_locations,
                    &mut deleting,
                )
                .await
            }
            Err(error) => Err(error),
        };
        let restored_location = match result {
            Ok(Reverted::Restored(report, identity)) => {
                Some(self.restored(location, report, *identity))
            }
            Ok(Reverted::Kept) => {
                self.kept_original(location);
                None
            }
            Err(error) if was_cancelled(&error) => {
                self.cancel(index, deleting.then_some(location));
                return ControlFlow::Break(());
            }
            Err(error) => {
                self.fail(location, &error);
                None
            }
        };
        self.outcome.completed += 1;
        (self.emit)(OperationEvent::RestoreProgress {
            request_id: self.request_id,
            completed: self.outcome.completed,
            total: self.total(),
            restored_location,
        });
        ControlFlow::Continue(())
    }

    fn restored(
        &mut self,
        location: &Location,
        report: TransferReport,
        identity: ItemIdentity,
    ) -> Location {
        let request_id = self.request_id;
        if !report.is_empty() {
            (self.emit)(OperationEvent::ItemsReported { request_id, report });
        }
        (self.emit)(OperationEvent::ItemRestored {
            request_id,
            location: location.clone(),
            identity,
        });
        self.outcome.completed_locations.push(location.clone());
        self.outcome.restored.push(location.clone());
        location.clone()
    }

    fn kept_original(&mut self, location: &Location) {
        let request_id = self.request_id;
        (self.emit)(OperationEvent::ItemsReported {
            request_id,
            report: TransferReport {
                kept_in_trash: vec![KeptItem {
                    location: location.clone(),
                    cause: KeptCause::ReplacementChanged,
                }],
                ..TransferReport::default()
            },
        });
        (self.emit)(OperationEvent::ItemKept {
            request_id,
            location: location.clone(),
        });
        self.outcome.completed_locations.push(location.clone());
    }

    async fn remove_created(&mut self, position: usize, location: &Location) -> ControlFlow<()> {
        let file = gio_file_for_location(location);
        let queried = query_type(&file, &self.cancellable).await;
        let found = queried.is_ok();
        let mut removal = Removal::default();
        let result = match queried {
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(()),
            Err(error) => Err(error),
            Ok(_) => self.trash_created(&file, location, &mut removal).await,
        };
        let deleted_locations = match result {
            Ok(()) if removal.kept => {
                self.kept_created(location);
                Vec::new()
            }
            Ok(()) => {
                self.removed_created(location, found, removal.trashed_identity);
                vec![location.clone()]
            }
            Err(error) if was_cancelled(&error) => {
                // Without Trash, an item stays whole until anything of it is deleted.
                let untouched =
                    !found || deletion_undone_when_cancelled(location, removal.deleting);
                self.cancel(position, (!untouched).then_some(location));
                return ControlFlow::Break(());
            }
            Err(error) => {
                self.fail(location, &error);
                Vec::new()
            }
        };
        self.outcome.completed += 1;
        (self.emit)(OperationEvent::DeleteProgress {
            request_id: self.request_id,
            completed: self.outcome.completed,
            total: self.total(),
            deleted_locations,
        });
        ControlFlow::Continue(())
    }

    async fn trash_created(
        &self,
        file: &gio::File,
        location: &Location,
        removal: &mut Removal,
    ) -> Result<(), glib::Error> {
        match trash_with_cleanup(file, &self.cancellable).await {
            Err(error) if error.is_unsupported() => self.delete_unchanged(location, removal).await,
            result => result
                .map(|identity| removal.trashed_identity = identity)
                .map_err(|error| error.error),
        }
    }

    async fn delete_unchanged(
        &self,
        location: &Location,
        removal: &mut Removal,
    ) -> Result<(), glib::Error> {
        let restoring = Restoring {
            context: &self.context,
            unlinks: &self.unlinks,
            cancellable: &self.cancellable,
        };
        let removed = match (self.placed.get(location), location.native_path()) {
            (Some(placed), Some(path)) => {
                remove_unchanged((path, placed), &restoring, &mut removal.deleting).await?
            }
            _ => false,
        };
        // What changed once part of it was deleted holds its folder.
        if !removed && removal.deleting {
            return Err(io_error(crate::services::io_error_detail(
                &io::Error::from_raw_os_error(rustix::io::Errno::NOTEMPTY.raw_os_error()),
            )));
        }
        removal.kept = !removed;
        Ok(())
    }

    fn kept_created(&mut self, location: &Location) {
        (self.emit)(OperationEvent::ItemKept {
            request_id: self.request_id,
            location: location.clone(),
        });
        self.outcome.errors.push(item_error(
            &location.display_name(),
            crate::i18n::tr("It was kept, as it may have changed since it was copied"),
        ));
        self.outcome.failed_locations.push(location.clone());
    }

    fn removed_created(
        &mut self,
        location: &Location,
        found: bool,
        trashed_identity: Option<ItemIdentity>,
    ) {
        if found {
            self.deleted_pins.push(location.clone());
        }
        if let Some(identity) = trashed_identity {
            (self.emit)(OperationEvent::ItemTrashed {
                request_id: self.request_id,
                location: location.clone(),
                identity,
            });
        }
        self.outcome.completed_locations.push(location.clone());
        self.outcome.removed.push(location.clone());
    }

    fn finish(mut self) {
        if self.cancellable.is_cancelled() {
            self.emit_cancelled(Vec::new());
            return;
        }
        let request_id = self.request_id;
        let outcome = std::mem::take(&mut self.outcome);
        if outcome.errors.is_empty() {
            (self.emit)(OperationEvent::Restored {
                request_id,
                locations: outcome.removed,
                restored: outcome.restored,
            });
            return;
        }
        (self.emit)(OperationEvent::CompletedWithErrors {
            request_id,
            deleted_locations: outcome.removed,
            retryable_locations: Vec::new(),
            has_non_retryable_failures: true,
            message: deletion_error_summary(&outcome.errors, true),
        });
    }
}
