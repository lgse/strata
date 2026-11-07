// SPDX-License-Identifier: MIT

use super::*;
use crate::adapters::trash_restore::RestoreContext;

/// Looks up every staged original once, rather than rescanning Trash per moved item.
pub(super) struct ReplacedMoves {
    entries: HashMap<Location, restore::RestoreEntry>,
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
        let locations: Vec<_> = request
            .items
            .iter()
            .filter(|item| item.record.replaced.is_some())
            .map(|item| item.record.current.clone())
            .collect();
        let entries = restore::trashed_entries_for_originals(&locations, cancellable, &originals)
            .await?
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|(_, error)| error)?;
        let operation = restore::release_trashed(&entries, cancellable).await;
        cancellable.set_error_if_cancelled()?;
        Ok(Some(Self {
            entries: locations.into_iter().zip(entries).collect(),
            context: RestoreContext::current().await,
            _operation: operation,
        }))
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
        let mut failure = None;
        for location in self.settling.completed() {
            let Some(entry) = replaced.entries.remove(&location) else {
                continue;
            };
            let restored =
                restore::restore_trash_entry(&entry, &restoring, &mut self.affected_locations)
                    .await;
            match restored {
                Ok((report, identity)) => self.report_restored(location, report, identity),
                Err(error) => {
                    failure =
                        joined_failures(failure, Some(crate::services::gio_error_message(&error)));
                }
            }
        }
        failure
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
