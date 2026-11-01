// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn paste(request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let operation_cancellable = cancellable.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let destination = gio_file_for_location(&request.destination);
        let request = in_copy_order(request, &destination);
        let mut affected_locations = HashSet::from([request.destination.clone()]);
        for parent in request.items.iter().filter_map(|item| item.source.parent()) {
            affected_locations.insert(parent);
        }
        let moved = if request.move_sources {
            release::natives(request.items.iter().map(|item| &item.source))
        } else {
            Vec::new()
        };
        let operation = Rc::new(ActiveOperation::begin(release::parents(&moved)));
        operation.include_folder(&request.destination);
        let sources = request
            .items
            .iter()
            .map(|item| gio_file_for_location(&item.source))
            .collect::<Vec<_>>();
        let names = NamePolicy::new(NameMode::Fix);
        let pasted_into = names
            .destination(&destination, &sources, &operation_cancellable)
            .await;
        let max_file_size = pasted_into.profile.max_file_size;
        let TransferSizes {
            sizes: item_sizes,
            total: total_bytes,
            files: item_files,
            total_files,
            kinds,
            filesystems,
        } = match transfer_sizes(&sources, &operation_cancellable, max_file_size).await {
            Ok(sizes) => sizes,
            Err(error) if was_cancelled(&error) => {
                emit(cancelled_event(
                    request.id,
                    Vec::new(),
                    Vec::new(),
                    request
                        .items
                        .iter()
                        .map(|item| item.source.clone())
                        .collect(),
                    affected_locations,
                ));
                return;
            }
            // Too large for FAT32, which the error says.
            Err(error) => {
                emit(OperationEvent::TransferFailed {
                    request_id: request.id,
                    completed_locations: Vec::new(),
                    message: error.message().to_owned(),
                });
                return;
            }
        };
        let rules = names.applied(pasted_into.profile.names, pasted_into.profile.strictness);
        let crossing = pasted_into.fixing(&filesystems);
        let duplicate = duplicates(&sources, &destination, request.move_sources);
        let planned = match planned_paste(
            &request.items,
            &sources,
            &rules,
            (&crossing, &duplicate),
            &kinds,
        ) {
            Ok(planned) => planned,
            Err(error) => {
                emit(OperationEvent::TransferFailed {
                    request_id: request.id,
                    completed_locations: Vec::new(),
                    message: placement_reason(error),
                });
                return;
            }
        };
        let targets = planned
            .iter()
            .map(|planned| {
                planned
                    .as_ref()
                    .map(|planned| destination.child(&planned.target))
            })
            .collect::<Vec<_>>();
        let targets_of = |conflict: TransferConflict| {
            request
                .items
                .iter()
                .zip(&targets)
                .zip(&duplicate)
                .filter(move |((item, _), duplicate)| item.conflict == conflict && !**duplicate)
                .filter_map(|((_, target), _)| target.as_ref())
        };
        operation.include_trees(targets_of(TransferConflict::Merge).filter_map(location_for_file));
        let replaced = targets_of(TransferConflict::ReplaceExisting).filter_map(gio::File::path);
        operation
            .release(
                moved.into_iter().chain(replaced).collect(),
                &operation_cancellable,
            )
            .await;
        let mut variants = CaseVariants::new(
            planned
                .iter()
                .enumerate()
                .filter_map(|(index, planned)| Some((index, planned.as_ref()?.target.clone()))),
        );
        let progress =
            TransferProgressTracker::new(request.id, total_bytes, total_files, emit.clone());
        progress.emit();
        let mut settling = Settling::paste(&request, emit.clone(), progress.clone());
        let copying = Copying {
            progress: Some(progress.clone()),
            names: names.clone(),
            ..Copying::default()
        };
        if let Some((plans, worker_count)) = parallel_copy_plans(
            &request,
            &sources,
            &destination,
            &planned,
            &kinds,
            pasted_into.profile.windows_disk(),
        )
        .await
        {
            for plan in &plans {
                affected_locations.insert(request.items[plan.index].source.clone());
                affected_locations.insert(plan.target_location.clone());
            }
            let outcome = execute_parallel_copy_plans(
                &plans,
                (&item_sizes, &item_files),
                (worker_count, request.move_sources),
                &operation_cancellable,
                &copying,
                (&mut settling, &mut variants),
            )
            .await;
            settling.finish().await;
            if !outcome.failed.is_empty()
                || outcome.next < plans.len()
                || settling.failure().is_some()
            {
                let (cut_off, failures): (Vec<_>, Vec<_>) = outcome
                    .failed
                    .iter()
                    .partition(|(_, error)| was_cancelled(error));
                let mut failed = settling.failed();
                failed.extend(
                    failures
                        .iter()
                        .map(|(index, _)| request.items[*index].source.clone()),
                );
                let failure_error = failures.first().map(|(_, error)| error);
                let (failure, skip_flush) = failure_error.map_or((None, false), |error| {
                    copy_failure_on_fat32(error, max_file_size.is_some())
                });
                stop_transfer(
                    &outcome.written_paths,
                    &emit,
                    request.id,
                    TransferStop {
                        completed: settling.completed(),
                        failed,
                        not_attempted: cut_off
                            .iter()
                            .map(|(index, _)| &request.items[*index])
                            .chain(&request.items[outcome.next..])
                            .map(|item| item.source.clone())
                            .collect(),
                        affected_locations,
                        failure: joined_failures(failure, settling.failure()),
                        skip_flush,
                    },
                )
                .await;
                return;
            }
            if !flush_removable_writes(
                outcome.written_paths,
                &operation_cancellable,
                &emit,
                request.id,
                &settling.completed(),
                affected_locations,
            )
            .await
            {
                return;
            }
            emit(OperationEvent::Pasted {
                request_id: request.id,
                locations: settling.completed(),
            });
            return;
        }
        // Undo deletes what a paste with a merge copied beside it where there is no Trash.
        let records_copies = !request.move_sources
            && request
                .items
                .iter()
                .any(|item| item.conflict == TransferConflict::Merge)
            && match native_path(&destination) {
                Some(path) => !can_trash(path).await,
                None => false,
            };
        let mut written_paths = Vec::new();
        for (index, item) in request.items.iter().enumerate() {
            if operation_cancellable.is_cancelled() || settling.failure().is_some() {
                settling.finish().await;
                stop_transfer(
                    &written_paths,
                    &emit,
                    request.id,
                    TransferStop {
                        completed: settling.completed(),
                        failed: settling.failed(),
                        not_attempted: request.items[index..]
                            .iter()
                            .map(|item| item.source.clone())
                            .collect(),
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
            let Some(target) = targets[index].clone() else {
                settling.finish().await;
                let flush_error = flush_written_roots(&written_paths, &emit, request.id)
                    .await
                    .err();
                let flush_error = joined_failures(settling.failure(), flush_error);
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: match flush_error {
                        Some(error) => rust_i18n::t!(
                            "A clipboard item has no file name. %{error}",
                            error = error
                        )
                        .into_owned(),
                        None => crate::i18n::tr("A clipboard item has no file name"),
                    },
                    password_failure: None,
                });
                return;
            };
            let is_duplicate = !request.move_sources && source.equal(&target);
            let needs_unique_target = is_duplicate || item.conflict == TransferConflict::KeepBoth;
            let is_noop = if is_duplicate {
                item.conflict == TransferConflict::ReplaceExisting
            } else {
                let (source, destination, target) =
                    (source.clone(), destination.clone(), target.clone());
                gio::spawn_blocking(move || transfer_is_noop(&source, &destination, &target))
                    .await
                    .unwrap_or(false)
            };
            let last = index + 1 == request.items.len();
            if is_noop {
                let done = Done {
                    index,
                    moved_to: None,
                };
                settling.owe(done, Owed::default(), 0, last).await;
                progress.finish_item(
                    progress.reported().since(started),
                    item_sizes[index],
                    item_files[index],
                    None,
                );
                continue;
            }
            let mut taken = if needs_unique_target {
                Taken::Number
            } else {
                Taken::Fail
            };
            let mut aliased = false;
            affected_locations.insert(item.source.clone());
            let default_location = location_for_file(&target);
            if let Some(target) = default_location.clone() {
                affected_locations.insert(target);
            }
            if let Some(path) = target.path() {
                written_paths.push(path);
            }
            let replaced_permanently = Rc::new(Cell::new(false));
            let undone =
                item.conflict != TransferConflict::Merge && undone_when_cancelled(&source, &target);
            let mut copied_ownership = None;
            // A destination known to hold no links is not asked to make
            // one; a move within its filesystem makes none.
            let result = if kinds[index] == Some(gio::FileType::SymbolicLink)
                && pasted_into.profile.symlinks == Some(false)
                && (crossing[index] || !request.move_sources)
            {
                Err(glib::Error::new(Unheld::Symlink, ""))
            } else if is_duplicate {
                copy_new_with_ownership(
                    source,
                    target,
                    taken,
                    operation_cancellable.clone(),
                    Some(progress.clone()),
                    names.clone(),
                    None,
                )
                .await
                .map(|(target, ownership)| {
                    copied_ownership = ownership;
                    (target, Owed::default())
                })
            } else if item.conflict == TransferConflict::Merge {
                merge_owing(
                    source,
                    target.clone(),
                    request.move_sources,
                    &operation_cancellable,
                    Some(&mut affected_locations),
                    &copying.placed_entries,
                    progress_merge_hooks(
                        Some(progress.clone()),
                        names.clone(),
                        Some(operation.clone()),
                        &|plan| {
                            for (location, identity) in plan.originals {
                                emit(OperationEvent::ItemTrashed {
                                    request_id: request.id,
                                    location,
                                    identity,
                                });
                            }
                            emit(OperationEvent::Merged {
                                request_id: request.id,
                                source: item.source.clone(),
                                created: plan.created,
                                overwritten: plan.overwritten,
                                placed: plan.placed,
                            });
                        },
                    ),
                )
                .await
                .map(|owed| (target, owed))
            } else if item.conflict == TransferConflict::ReplaceExisting {
                let replaced_target = default_location;
                let emit = emit.clone();
                let replaced_permanently = replaced_permanently.clone();
                replace_owing(
                    source,
                    target.clone(),
                    request.move_sources,
                    &operation_cancellable,
                    Some(&mut affected_locations),
                    copying.clone(),
                    &move |original, placed| {
                        let identity = match original {
                            ReplacedOriginal::Trashed(identity) => identity,
                            ReplacedOriginal::Deleted => {
                                replaced_permanently.set(true);
                                return;
                            }
                        };
                        if let (Some(target), Some(identity)) = (&replaced_target, identity) {
                            emit(OperationEvent::ItemTrashed {
                                request_id: request.id,
                                location: target.clone(),
                                identity,
                            });
                        }
                        // Only copies get the restore-the-original undo
                        // entry; a replaced move keeps the move-back
                        // record and leaves the original in Trash.
                        if request.move_sources {
                            return;
                        }
                        if let Some(target) = &replaced_target {
                            emit(OperationEvent::Merged {
                                request_id: request.id,
                                source: item.source.clone(),
                                created: Vec::new(),
                                overwritten: vec![target.clone()],
                                placed: placed
                                    .map(|placed| HashMap::from([(target.clone(), placed)]))
                                    .unwrap_or_default(),
                            });
                        }
                    },
                )
                .await
                .map(|owed| (target, owed))
            } else {
                if taken == Taken::Fail
                    && case_variants::taken_by(&target, variants.earlier(index)).await
                {
                    taken = Taken::Number;
                    aliased = true;
                }
                if request.move_sources {
                    move_owing(
                        source.clone(),
                        target.clone(),
                        taken,
                        operation_cancellable.clone(),
                        copying.clone(),
                    )
                    .await
                    .map(|moved| (moved.target, moved.owed))
                } else {
                    copy_new_with_ownership(
                        source.clone(),
                        target.clone(),
                        taken,
                        operation_cancellable.clone(),
                        Some(progress.clone()),
                        names.clone(),
                        None,
                    )
                    .await
                    .map(|(target, ownership)| {
                        copied_ownership = ownership;
                        (target, Owed::default())
                    })
                }
            };
            if let Err(error) = &result
                && let Some(kind) = unheld(error)
            {
                progress.report(left_out(
                    &item.source,
                    &destination,
                    kind,
                    request.move_sources,
                ));
                progress.finish_item(
                    progress.reported().since(started),
                    item_sizes[index],
                    item_files[index],
                    None,
                );
                continue;
            }
            let (target, owed) = match result {
                Ok(result) => result,
                Err(error) => {
                    settling.finish().await;
                    let (failure, skip_flush) =
                        copy_failure_on_fat32(&error, max_file_size.is_some());
                    let untouched = undone && was_cancelled(&error);
                    let mut failed = settling.failed();
                    if !untouched {
                        failed.push(item.source.clone());
                    }
                    stop_transfer(
                        &written_paths,
                        &emit,
                        request.id,
                        TransferStop {
                            completed: settling.completed(),
                            failed,
                            not_attempted: request.items[index + usize::from(!untouched)..]
                                .iter()
                                .map(|item| item.source.clone())
                                .collect(),
                            affected_locations,
                            failure: joined_failures(failure, settling.failure()),
                            skip_flush,
                        },
                    )
                    .await;
                    return;
                }
            };
            let target_location = location_for_file(&target);
            if let Some(target) = target_location.clone() {
                affected_locations.insert(target);
            }
            variants.saved(index, &target).await;
            let cause = planned[index]
                .as_ref()
                .and_then(|planned| planned.cause)
                .or(aliased.then_some(RenameCause::Alias));
            if let (Some(cause), Some(name)) = (cause, transfer_source_name(&sources[index])) {
                // Keep Both's numbers go unreported; the engine reports a
                // name the mount refused from the planned one.
                let saved_as = match (taken, &planned[index]) {
                    (Taken::Fail, Some(planned)) => destination.child(&planned.target),
                    _ => target.clone(),
                };
                progress.report(TransferReport {
                    renamed: renamed_item(&destination.child(name), &saved_as, cause)
                        .into_iter()
                        .collect(),
                    ..TransferReport::default()
                });
            }
            // A merge reports its written paths through Merged instead:
            // copy undo trashes recorded locations, and the merged folder
            // held pre-existing contents that must not be trashed.
            // Undoing a copy deletes it, which must never remove the only
            // copy left after an original was replaced without Trash.
            let created_location = target_location.clone().filter(|_| {
                !request.move_sources
                    && item.conflict != TransferConflict::Merge
                    && !replaced_permanently.get()
            });
            if records_copies
                && item.conflict != TransferConflict::ReplaceExisting
                && let Some(location) = &created_location
                && let Some(path) = location.native_path()
                && let Some(ownership) = copied_ownership
                && let Some(placed) = copy_engine::placed_proven(
                    path.to_path_buf(),
                    ownership,
                    copying.placed_entries.clone(),
                    &operation_cancellable,
                )
                .await
            {
                emit(OperationEvent::Merged {
                    request_id: request.id,
                    source: item.source.clone(),
                    created: Vec::new(),
                    overwritten: Vec::new(),
                    placed: HashMap::from([(location.clone(), placed)]),
                });
            }
            let done = Done {
                index,
                moved_to: target_location,
            };
            let reported = progress.reported().since(started);
            progress.finish_item(
                reported,
                item_sizes[index],
                item_files[index],
                created_location,
            );
            settling.owe(done, owed, reported.bytes, last).await;
        }
        settling.finish().await;
        if let Some(failure) = settling.failure() {
            stop_transfer(
                &written_paths,
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
            written_paths,
            &operation_cancellable,
            &emit,
            request.id,
            &completed,
            affected_locations,
        )
        .await
        {
            return;
        }
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: completed,
        });
    });
    operation_handle(cancellable)
}
