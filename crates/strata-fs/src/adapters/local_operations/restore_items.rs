// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn restore(request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let operation_cancellable = cancellable.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let entries = match request.source {
            RestoreSource::TrashEntries(items) => items
                .into_iter()
                .map(|item| {
                    Ok(RestoreEntry {
                        source: item.entry.location,
                        display_name: item.entry.display_name,
                        original_target: None,
                        trash_info: None,
                        confirmed_destination: Some(item.destination),
                        physical_path: item.entry.thumbnail_path,
                    })
                })
                .collect(),
            RestoreSource::OriginalLocations {
                locations,
                originals,
            } => {
                match trashed_entries_for_originals(&locations, &operation_cancellable, &originals)
                    .await
                {
                    Ok(entries) => entries,
                    Err(error) if was_cancelled(&error) => {
                        emit(cancelled_event(
                            request.id,
                            Vec::new(),
                            Vec::new(),
                            locations,
                            HashSet::from([Location::uri("trash:///")]),
                        ));
                        return;
                    }
                    Err(error) => {
                        emit(OperationEvent::Failed {
                            request_id: request.id,
                            message: rust_i18n::t!(
                                "Unable to find items in Trash: %{error}",
                                error = error
                            )
                            .into_owned(),
                            password_failure: None,
                        });
                        return;
                    }
                }
            }
        };
        let found = entries.iter().flatten().cloned().collect::<Vec<_>>();
        let _operation = restore::release_trashed(&found, &operation_cancellable).await;
        let total = entries.len();
        let mut errors = Vec::new();
        let mut restored_locations = Vec::new();
        let mut restored = Vec::new();
        let mut failed_locations = Vec::new();
        let mut affected_locations = HashSet::from([Location::uri("trash:///")]);
        let context = RestoreContext::current().await;
        let unlinks = Arc::new(copy_engine::Unlinks::default());
        let worker_count = restore_worker_count(&found).await;
        let mut completed = 0;
        let mut cut_off = Vec::new();
        let source = |entry: &Result<RestoreEntry, (Location, glib::Error)>| match entry {
            Ok(entry) => entry.source.clone(),
            Err((original, _)) => original.clone(),
        };
        for chunk in entries.chunks(worker_count) {
            if operation_cancellable.is_cancelled() {
                emit(cancelled_event(
                    request.id,
                    restored,
                    failed_locations,
                    entries[completed..].iter().map(source).collect(),
                    affected_locations,
                ));
                return;
            }
            let main_context = glib::MainContext::default();
            let tasks = chunk
                .iter()
                .cloned()
                .map(|entry| {
                    let (context, unlinks) = (context.clone(), unlinks.clone());
                    let cancellable = operation_cancellable.clone();
                    main_context.spawn_local(async move {
                        let mut affected = HashSet::new();
                        let result = match &entry {
                            Ok(entry) => {
                                let restoring = restore::Restoring {
                                    context: &context,
                                    unlinks: &unlinks,
                                    cancellable: &cancellable,
                                };
                                restore_trash_entry(entry, &restoring, &mut affected)
                                    .await
                                    .map(|(report, identity)| {
                                        (report, identity, restored_destination(entry))
                                    })
                            }
                            Err((_, refused)) => Err(refused.clone()),
                        };
                        (entry, result, affected)
                    })
                })
                .collect::<Vec<_>>();
            for task in tasks {
                let (entry, result, affected) = match task.await {
                    Ok(outcome) => outcome,
                    Err(_) => (
                        entries[completed].clone(),
                        Err(io_error("Restore worker failed")),
                        HashSet::new(),
                    ),
                };
                let display_name = match &entry {
                    Ok(entry) => entry.display_name.clone(),
                    Err((original, _)) => original.display_name(),
                };
                affected_locations.extend(affected);
                completed += 1;
                let restored_location = match result {
                    Err(error) if was_cancelled(&error) => {
                        cut_off.push(source(&entry));
                        None
                    }
                    Err(error) => {
                        errors.push(item_error(
                            &display_name,
                            crate::services::gio_error_detail(&error),
                        ));
                        failed_locations.push(source(&entry));
                        None
                    }
                    Ok((report, identity, destination)) => {
                        if !report.is_empty() {
                            emit(OperationEvent::ItemsReported {
                                request_id: request.id,
                                report,
                            });
                        }
                        restored_locations.push(source(&entry));
                        emit(OperationEvent::ItemRestored {
                            request_id: request.id,
                            location: destination.clone(),
                            identity,
                        });
                        restored.push(destination.clone());
                        Some(destination)
                    }
                };
                emit(OperationEvent::RestoreProgress {
                    request_id: request.id,
                    completed,
                    total,
                    restored_location,
                });
            }
            if !cut_off.is_empty() || operation_cancellable.is_cancelled() {
                cut_off.extend(entries[completed..].iter().map(source));
                emit(cancelled_event(
                    request.id,
                    restored,
                    failed_locations,
                    cut_off,
                    affected_locations,
                ));
                return;
            }
        }
        if errors.is_empty() {
            emit(OperationEvent::Restored {
                request_id: request.id,
                locations: restored_locations,
                restored,
            });
        } else {
            emit(OperationEvent::RestoreCompletedWithErrors {
                request_id: request.id,
                restored_locations,
                restored,
                message: operation_error_summary(&errors, FailedAction::Restore),
            });
        }
    });
    operation_handle(cancellable)
}
