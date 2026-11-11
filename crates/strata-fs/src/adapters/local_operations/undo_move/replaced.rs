// SPDX-License-Identifier: MIT

use super::*;
use crate::adapters::trash_restore::RestoreContext;

/// A failed lookup keeps its own message, which says more than its error code.
type Staged = Result<restore::RestoreEntry, String>;

/// Looks up every staged original once, rather than rescanning Trash per moved item.
pub(super) struct ReplacedMoves {
    entries: HashMap<Location, Staged>,
    context: RestoreContext,
    _operation: ActiveOperation,
}

impl ReplacedMoves {
    pub(super) async fn prepare(
        request: &UndoMoveRequest,
        cancellable: &gio::Cancellable,
    ) -> Result<Option<Self>, glib::Error> {
        let originals: HashMap<_, _> = request
            .items
            .iter()
            .filter_map(|item| Some((item.record.current.clone(), item.record.replaced?)))
            .collect();
        if originals.is_empty() {
            return Ok(None);
        }
        let entries = staged_originals(&originals, cancellable).await?;
        let found: Vec<_> = entries
            .values()
            .filter_map(|staged| staged.as_ref().ok().cloned())
            .collect();
        let operation = restore::release_trashed(&found, cancellable).await;
        cancellable.set_error_if_cancelled()?;
        Ok(Some(Self {
            entries,
            context: RestoreContext::current().await,
            _operation: operation,
        }))
    }
}

/// A failed undo stays on top of the history, so an original that is gone for good must
/// not stop the items from moving back: a failed batch is looked up item by item.
async fn staged_originals(
    originals: &HashMap<Location, ItemIdentity>,
    cancellable: &gio::Cancellable,
) -> Result<HashMap<Location, Staged>, glib::Error> {
    let locations: Vec<_> = originals.keys().cloned().collect();
    match restore::trashed_entries_for_originals(&locations, cancellable, originals).await {
        Ok(found) => {
            let found = found
                .into_iter()
                .map(|staged| staged.map_err(|(_, error)| error.message().to_owned()));
            Ok(locations.into_iter().zip(found).collect())
        }
        Err(error) if was_cancelled(&error) => Err(error),
        Err(_) => {
            let mut entries = HashMap::new();
            for location in locations {
                let staged = restore::trashed_merge_original(
                    location.clone(),
                    cancellable.clone(),
                    originals.clone(),
                )
                .await;
                cancellable.set_error_if_cancelled()?;
                entries.insert(location, staged.map_err(|error| error.message().to_owned()));
            }
            Ok(entries)
        }
    }
}

impl UndoMove {
    pub(super) async fn settle(&mut self) -> Option<String> {
        // Cross-volume sources stay until their batches are durable. Settle
        // them together before restoring originals into the vacated paths.
        self.settling.finish().await;
        joined_failures(self.settling.failure(), self.restore_replaced().await)
    }

    async fn restore_replaced(&mut self) -> Option<String> {
        let mut replaced = self.replaced.take()?;
        // Like settling a completed move, restoring its original must finish
        // even if cancellation arrived after the move committed.
        let cancellable = gio::Cancellable::new();
        let unlinks = Arc::default();
        let restoring = restore::Restoring {
            context: &replaced.context,
            unlinks: &unlinks,
            cancellable: &cancellable,
        };
        let mut failures = Vec::new();
        for location in self.settling.completed() {
            let Some(staged) = replaced.entries.remove(&location) else {
                continue;
            };
            let failure = match staged {
                Ok(entry) => self.restore_original(location, &entry, &restoring).await,
                Err(message) => Some(message),
            };
            if let Some(message) = failure
                && !failures.contains(&message)
            {
                failures.push(message);
            }
        }
        failures.into_iter().map(Some).reduce(joined_failures)?
    }

    async fn restore_original(
        &mut self,
        location: Location,
        entry: &restore::RestoreEntry,
        restoring: &restore::Restoring<'_>,
    ) -> Option<String> {
        match restore::restore_trash_entry(entry, restoring, &mut self.affected_locations).await {
            Ok((report, identity)) => {
                self.report_restored(location, report, identity);
                None
            }
            Err(error) => Some(crate::services::gio_error_message(&error)),
        }
    }

    fn report_restored(
        &mut self,
        location: Location,
        report: TransferReport,
        identity: ItemIdentity,
    ) {
        if !report.is_empty() {
            (self.emit)(OperationEvent::ItemsReported {
                request_id: self.request.id,
                report,
            });
        }
        if let Some(path) = location.native_path() {
            self.restored_paths.push(path.to_path_buf());
        }
        (self.emit)(OperationEvent::ItemRestored {
            request_id: self.request.id,
            location,
            identity,
        });
    }
}
