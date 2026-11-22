// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{
    app::navigation::sort_entries,
    model::{FileEntry, SortKey, ViewPreferences},
    services::{DirectoryChange, RequestId},
};

use super::{
    Browser, BrowserEvent, SortingLoad,
    loading::LoadCompletion,
    publication::{PublicationPlan, PublishTerminal},
};

const SORT_INLINE_LIMIT: usize = 2048;

#[derive(Clone, Copy)]
pub(super) struct SortTask {
    pub(super) depth: usize,
    pub(super) request_id: RequestId,
    pub(super) plan: SortPlan,
}

#[derive(Clone, Copy)]
pub(super) struct SortPlan {
    pub(super) ordering_preferences: ViewPreferences,
    pub(super) staged_preferences: ViewPreferences,
    pub(super) retry_metadata: bool,
    pub(super) completion: LoadCompletion,
}

impl Browser {
    /// Keep the loading state up until the sorted, reconciled snapshot is published.
    pub(super) fn finish_staged_load(
        self: &Rc<Self>,
        depth: usize,
        request_id: RequestId,
        completion: LoadCompletion,
    ) {
        let staging = self.staging.borrow_mut().remove(&depth);
        let Some(staging) = staging.filter(|staged| staged.request_id == request_id) else {
            return;
        };
        let preferences = self.sort_preferences(depth);
        let mut entries = staging.entries;
        entries.retain(|entry| !staging.removed.contains(&entry.location));
        let incomplete_stat = staging.metadata_incomplete && preferences.sort_key.uses_file_stat();
        let retry_metadata = incomplete_stat || preferences.sort_key.uses_media_metadata();
        let ordering_preferences = if incomplete_stat {
            ViewPreferences {
                sort_key: SortKey::Name,
                ..preferences
            }
        } else {
            preferences
        };
        self.sorting.borrow_mut().insert(
            depth,
            SortingLoad {
                request_id,
                deltas: staging.deltas,
            },
        );
        self.run_sort_task(
            SortTask {
                depth,
                request_id,
                plan: SortPlan {
                    ordering_preferences,
                    staged_preferences: preferences,
                    retry_metadata,
                    completion,
                },
            },
            entries,
        );
    }

    fn sort_preferences(&self, depth: usize) -> ViewPreferences {
        self.state
            .borrow()
            .column_preferences(depth)
            .unwrap_or_else(|| self.preferences.get())
    }

    fn run_sort_task(self: &Rc<Self>, task: SortTask, entries: Vec<FileEntry>) {
        if entries.len() <= SORT_INLINE_LIMIT {
            let sorted = sort_entries(entries, task.plan.ordering_preferences);
            let unique = prove_unique_locations(&sorted, task.plan.ordering_preferences);
            self.finish_staged_sort_with_uniqueness(task, sorted, unique);
            return;
        }
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let sorted = gio::spawn_blocking(move || {
                let sorted = sort_entries(entries, task.plan.ordering_preferences);
                let unique = prove_unique_locations(&sorted, task.plan.ordering_preferences);
                (sorted, unique)
            })
            .await;
            let Some(browser) = weak.upgrade() else {
                return;
            };
            match sorted {
                Ok((sorted, unique)) => {
                    browser.finish_staged_sort_with_uniqueness(task, sorted, unique)
                }
                Err(_) => browser.fail_staged_sort(task),
            }
        });
    }

    fn take_sorting_load(&self, task: SortTask) -> Option<SortingLoad> {
        let mut sorting = self.sorting.borrow_mut();
        if sorting.get(&task.depth)?.request_id != task.request_id {
            return None;
        }
        sorting.remove(&task.depth)
    }

    #[cfg(test)]
    pub(super) fn finish_staged_sort(self: &Rc<Self>, task: SortTask, sorted: Vec<FileEntry>) {
        self.finish_staged_sort_with_uniqueness(task, sorted, None);
    }

    fn finish_staged_sort_with_uniqueness(
        self: &Rc<Self>,
        task: SortTask,
        sorted: Vec<FileEntry>,
        unique_locations: Option<bool>,
    ) {
        let Some(sorting) = self.take_sorting_load(task) else {
            return;
        };
        if !self.install_sorted_snapshot(
            task,
            SortedSnapshot {
                entries: sorted,
                unique_locations,
            },
            sorting,
        ) {
            return;
        }
        let current = self.sort_preferences(task.depth);
        if current != task.plan.staged_preferences {
            self.resort_changed_preferences(task, current);
        } else {
            self.publish_sorted_snapshot(task);
        }
    }

    fn install_sorted_snapshot(
        self: &Rc<Self>,
        task: SortTask,
        snapshot: SortedSnapshot,
        sorting: SortingLoad,
    ) -> bool {
        let mut state = self.state.borrow_mut();
        if state.request_id_for_depth(task.depth) != Some(task.request_id)
            || state
                .install_snapshot_with_uniqueness(
                    task.request_id,
                    snapshot.entries,
                    snapshot.unique_locations,
                )
                .is_none()
        {
            return false;
        }
        // No delta events: the UI model is still empty until staged publication.
        for (watched, change) in sorting.deltas {
            if !matches!(change, DirectoryChange::Rescan) {
                let _applied = state.apply_directory_change(task.depth, &watched, change);
            }
        }
        drop(state);
        self.apply_resolved_location_reveal(task.depth, task.request_id);
        true
    }

    fn resort_changed_preferences(self: &Rc<Self>, task: SortTask, current: ViewPreferences) {
        if self
            .state
            .borrow()
            .column_unknown_sort_metadata(task.depth, current.sort_key)
            .next()
            .is_some()
        {
            self.finish_sorted_load(task);
            self.note_listing_finished(task.depth);
            self.emit(BrowserEvent::LoadFinished {
                depth: task.depth,
                truncated: task.plan.completion.truncated,
            });
            self.report_unresolved_location_reveal(task.depth, task.request_id);
            self.ensure_sorted_after_load(task.depth);
        } else {
            self.resort_installed_column(task, current);
        }
    }

    fn finish_sorted_load(&self, task: SortTask) {
        let completion = task.plan.completion;
        self.state.borrow_mut().finish(
            task.request_id,
            completion.truncated,
            completion.can_trash,
            completion.can_delete,
        );
    }

    fn publish_sorted_snapshot(self: &Rc<Self>, task: SortTask) {
        let plan = {
            let mut state = self.state.borrow_mut();
            let take_focus = state.take_selection_from_reveal(task.depth);
            let column = state.columns.get(task.depth);
            PublicationPlan {
                request_id: task.request_id,
                total: column.map_or(0, |column| column.entries.len()),
                focused: column.and_then(|column| column.selected),
                positions: state.selected_positions(task.depth),
                take_focus,
                terminal: PublishTerminal::LoadFinished {
                    truncated: task.plan.completion.truncated,
                    retry_metadata: task.plan.retry_metadata,
                },
            }
        };
        self.finish_sorted_load(task);
        self.publish_staged(task.depth, plan);
    }

    fn resort_installed_column(self: &Rc<Self>, task: SortTask, preferences: ViewPreferences) {
        let Some(entries) = self
            .state
            .borrow()
            .columns
            .get(task.depth)
            .map(|column| column.entries.clone())
        else {
            return;
        };
        self.sorting.borrow_mut().insert(
            task.depth,
            SortingLoad {
                request_id: task.request_id,
                deltas: Vec::new(),
            },
        );
        self.run_sort_task(
            SortTask {
                plan: SortPlan {
                    ordering_preferences: preferences,
                    staged_preferences: preferences,
                    retry_metadata: false,
                    completion: task.plan.completion,
                },
                ..task
            },
            entries,
        );
    }

    pub(super) fn fail_staged_sort(self: &Rc<Self>, task: SortTask) {
        // A superseded worker must not remove the replacement load's delta queue.
        if self.take_sorting_load(task).is_none() {
            return;
        }
        let mut state = self.state.borrow_mut();
        if state
            .fail(
                task.request_id,
                "Sorting the directory failed.".to_owned(),
                None,
            )
            .is_some()
        {
            drop(state);
            self.emit(BrowserEvent::LoadFailed {
                depth: task.depth,
                message: "Sorting the directory failed.".to_owned(),
                administrator: None,
            });
        }
    }
}

struct SortedSnapshot {
    entries: Vec<FileEntry>,
    unique_locations: Option<bool>,
}

/// Proves whether every entry in a sorted listing has a distinct `Location` without
/// allocating an O(N) `HashSet` when entries are native children sorted by name.
fn prove_unique_locations(entries: &[FileEntry], preferences: ViewPreferences) -> Option<bool> {
    if entries.len() <= 1 {
        return Some(true);
    }
    if preferences.sort_key != SortKey::Name {
        return None;
    }
    let first_parent = entries[0].location.native_path()?.parent()?;
    if !all_native_children_named_by_leaf(entries, first_parent) {
        return None;
    }
    if !preferences.folders_first {
        return Some(no_adjacent_duplicate_locations(entries));
    }
    prove_partitioned_unique(entries)
}

fn prove_partitioned_unique(entries: &[FileEntry]) -> Option<bool> {
    let dirs = entries.partition_point(FileEntry::is_directory);
    let (dir_slice, file_slice) = entries.split_at(dirs);
    if !(no_adjacent_duplicate_locations(dir_slice) && no_adjacent_duplicate_locations(file_slice))
    {
        return Some(false);
    }
    if dir_slice.is_empty() || file_slice.is_empty() {
        return Some(true);
    }
    disjoint_locations(dir_slice, file_slice)
}

fn disjoint_locations(left: &[FileEntry], right: &[FileEntry]) -> Option<bool> {
    let (smaller, larger) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    if smaller.len() > 4096 {
        return None;
    }
    let small_set: std::collections::HashSet<&crate::model::Location> =
        smaller.iter().map(|entry| &entry.location).collect();
    Some(
        larger
            .iter()
            .all(|entry| !small_set.contains(&entry.location)),
    )
}

fn all_native_children_named_by_leaf(entries: &[FileEntry], parent: &std::path::Path) -> bool {
    entries.iter().all(|entry| {
        let Some(path) = entry.location.native_path() else {
            return false;
        };
        path.parent() == Some(parent)
            && path.file_name().and_then(std::ffi::OsStr::to_str)
                == Some(entry.display_name.as_str())
    })
}

fn no_adjacent_duplicate_locations(entries: &[FileEntry]) -> bool {
    entries
        .windows(2)
        .all(|pair| pair[0].location != pair[1].location)
}
