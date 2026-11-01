// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn undo_move(request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let operation_cancellable = cancellable.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let mut affected_locations = HashSet::new();
        for item in &request.items {
            for location in [&item.record.current, &item.record.original] {
                affected_locations.insert(location.clone());
                if let Some(parent) = location.parent() {
                    affected_locations.insert(parent);
                }
            }
        }
        for location in &request.cleanup_locations {
            affected_locations.insert(location.clone());
            if let Some(parent) = location.parent() {
                affected_locations.insert(parent);
            }
        }
        let moved = release::natives(request.items.iter().map(|item| &item.record.current));
        let operation =
            ActiveOperation::begin(release::parents(&moved).into_iter().chain(release::parents(
                &release::natives(request.items.iter().map(|item| &item.record.original)),
            )));
        let sources = request
            .items
            .iter()
            .map(|item| gio_file_for_location(&item.record.current))
            .collect::<Vec<_>>();
        let TransferSizes {
            sizes: item_sizes,
            total: total_bytes,
            files: item_files,
            total_files,
            ..
        } = match transfer_sizes(&sources, &operation_cancellable, None).await {
            Ok(sizes) => sizes,
            Err(error) if was_cancelled(&error) => {
                emit(cancelled_event(
                    request.id,
                    Vec::new(),
                    Vec::new(),
                    request
                        .items
                        .iter()
                        .map(|item| item.record.current.clone())
                        .collect(),
                    affected_locations,
                ));
                return;
            }
            Err(error) => {
                emit(OperationEvent::TransferFailed {
                    request_id: request.id,
                    completed_locations: Vec::new(),
                    message: crate::services::gio_error_message(&error),
                });
                return;
            }
        };
        let replaced = release::natives(
            request
                .items
                .iter()
                .filter(|item| item.conflict == TransferConflict::ReplaceExisting)
                .map(|item| &item.record.original),
        );
        let cleaned = release::natives(&request.cleanup_locations);
        operation
            .release(
                moved.into_iter().chain(replaced).chain(cleaned).collect(),
                &operation_cancellable,
            )
            .await;
        let names = NamePolicy::new(request.names);
        let progress =
            TransferProgressTracker::new(request.id, total_bytes, total_files, emit.clone());
        progress.emit();
        let undone = request
            .items
            .iter()
            .map(|item| owed::Source {
                location: item.record.current.clone(),
                merged: false,
            })
            .collect();
        let mut settling =
            Settling::new(request.id, (undone, true), emit.clone(), progress.clone());
        let mut restored_paths = Vec::new();
        for (index, item) in request.items.iter().enumerate() {
            let remaining = || {
                request.items[index..]
                    .iter()
                    .map(|item| item.record.current.clone())
                    .collect::<Vec<_>>()
            };
            if operation_cancellable.is_cancelled() || settling.failure().is_some() {
                settling.finish().await;
                stop_transfer(
                    &restored_paths,
                    &emit,
                    request.id,
                    TransferStop {
                        completed: settling.completed(),
                        failed: settling.failed(),
                        not_attempted: remaining(),
                        affected_locations,
                        failure: settling.failure(),
                        skip_flush: false,
                    },
                )
                .await;
                return;
            }
            let source = sources[index].clone();
            let started = progress.reported();
            let restored_path = item.record.original.native_path().map(Path::to_path_buf);
            if let Some(path) = restored_path {
                restored_paths.push(path);
            }
            let target = gio_file_for_location(&item.record.original);
            let undone = undone_when_cancelled(&source, &target);
            let copying = Copying {
                progress: Some(progress.clone()),
                names: names.clone(),
                ..Copying::default()
            };
            let result = if item.conflict == TransferConflict::ReplaceExisting {
                replace_owing(
                    source,
                    target,
                    true,
                    &operation_cancellable,
                    Some(&mut affected_locations),
                    copying,
                    &|original, _| {
                        if let ReplacedOriginal::Trashed(Some(identity)) = original {
                            emit(OperationEvent::ItemTrashed {
                                request_id: request.id,
                                location: item.record.original.clone(),
                                identity,
                            });
                        }
                    },
                )
                .await
            } else {
                move_owing(
                    source,
                    target,
                    Taken::Fail,
                    operation_cancellable.clone(),
                    copying,
                )
                .await
                .map(|moved| moved.owed)
            };
            let mut owed = match result {
                Ok(owed) => owed,
                Err(error) => {
                    settling.finish().await;
                    let failure = (!was_cancelled(&error))
                        .then(|| crate::services::gio_error_message(&error));
                    let untouched = undone && was_cancelled(&error);
                    let mut failed = settling.failed();
                    if !untouched {
                        failed.push(item.record.current.clone());
                    }
                    stop_transfer(
                        &restored_paths,
                        &emit,
                        request.id,
                        TransferStop {
                            completed: settling.completed(),
                            failed,
                            not_attempted: request.items[index + usize::from(!untouched)..]
                                .iter()
                                .map(|item| item.record.current.clone())
                                .collect(),
                            affected_locations,
                            failure: joined_failures(failure, settling.failure()),
                            skip_flush: false,
                        },
                    )
                    .await;
                    return;
                }
            };
            owed.follow_renames(&restore_renamed(&item.record, &progress).await);
            let reported = progress.reported().since(started);
            progress.finish_item(reported, item_sizes[index], item_files[index], None);
            let done = Done {
                index,
                moved_to: Some(item.record.original.clone()),
            };
            let last = index + 1 == request.items.len();
            settling.owe(done, owed, reported.bytes, last).await;
        }
        settling.finish().await;
        if let Some(failure) = settling.failure() {
            stop_transfer(
                &restored_paths,
                &emit,
                request.id,
                TransferStop {
                    completed: settling.completed(),
                    failed: settling.failed(),
                    not_attempted: Vec::new(),
                    affected_locations,
                    failure: Some(failure),
                    skip_flush: false,
                },
            )
            .await;
            return;
        }
        let completed = settling.completed();
        if !flush_removable_writes(
            restored_paths,
            &operation_cancellable,
            &emit,
            request.id,
            &completed,
            affected_locations.clone(),
        )
        .await
        {
            return;
        }
        for location in &request.cleanup_locations {
            let file = gio_file_for_location(location);
            let identity = match trash_with_cleanup(&file, &operation_cancellable).await {
                Ok(identity) => identity,
                Err(error) if error.is_unsupported() => {
                    match remove_empty_directory(&file, &operation_cancellable).await {
                        Err(error) if was_cancelled(&error) => {
                            emit(cancelled_event(
                                request.id,
                                completed,
                                Vec::new(),
                                vec![location.clone()],
                                affected_locations,
                            ));
                            return;
                        }
                        _ => continue,
                    }
                }
                Err(error) => {
                    if was_cancelled(&error.error) {
                        emit(cancelled_event(
                            request.id,
                            completed,
                            Vec::new(),
                            vec![location.clone()],
                            affected_locations,
                        ));
                    } else {
                        emit(OperationEvent::TransferFailed {
                            request_id: request.id,
                            completed_locations: completed,
                            message: crate::services::gio_error_message(&error.error),
                        });
                    }
                    return;
                }
            };
            if let Some(identity) = identity {
                emit(OperationEvent::ItemTrashed {
                    request_id: request.id,
                    location: location.clone(),
                    identity,
                });
            }
        }
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: completed,
        });
    });
    operation_handle(cancellable)
}
