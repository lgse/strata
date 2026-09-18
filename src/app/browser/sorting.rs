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
        let retry_metadata = staging.metadata_incomplete
            && matches!(preferences.sort_key, SortKey::Size | SortKey::Modified);
        let ordering_preferences = if retry_metadata {
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
            self.finish_staged_sort(task, sorted);
            return;
        }
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let sorted =
                gio::spawn_blocking(move || sort_entries(entries, task.plan.ordering_preferences))
                    .await;
            let Some(browser) = weak.upgrade() else {
                return;
            };
            match sorted {
                Ok(sorted) => browser.finish_staged_sort(task, sorted),
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

    pub(super) fn finish_staged_sort(self: &Rc<Self>, task: SortTask, sorted: Vec<FileEntry>) {
        let Some(sorting) = self.take_sorting_load(task) else {
            return;
        };
        if !self.install_sorted_snapshot(task, sorted, sorting) {
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
        &self,
        task: SortTask,
        sorted: Vec<FileEntry>,
        sorting: SortingLoad,
    ) -> bool {
        let mut state = self.state.borrow_mut();
        if state.request_id_for_depth(task.depth) != Some(task.request_id)
            || state.install_snapshot(task.request_id, sorted).is_none()
        {
            return false;
        }
        // No delta events: the UI model is still empty until staged publication.
        for (watched, change) in sorting.deltas {
            if !matches!(change, DirectoryChange::Rescan) {
                let _applied = state.apply_directory_change(task.depth, &watched, change);
            }
        }
        true
    }

    fn resort_changed_preferences(self: &Rc<Self>, task: SortTask, current: ViewPreferences) {
        if matches!(current.sort_key, SortKey::Size | SortKey::Modified)
            && self
                .state
                .borrow()
                .column_unknown_metadata(task.depth)
                .is_some()
        {
            self.finish_sorted_load(task);
            self.emit(BrowserEvent::LoadFinished {
                depth: task.depth,
                truncated: task.plan.completion.truncated,
            });
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
            let state = self.state.borrow();
            let column = state.columns.get(task.depth);
            PublicationPlan {
                request_id: task.request_id,
                total: column.map_or(0, |column| column.entries.len()),
                focused: column.and_then(|column| column.selected),
                positions: state.selected_positions(task.depth),
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
            .fail(task.request_id, "Sorting the directory failed.".to_owned())
            .is_some()
        {
            drop(state);
            self.emit(BrowserEvent::LoadFailed {
                depth: task.depth,
                message: "Sorting the directory failed.".to_owned(),
            });
        }
    }
}
