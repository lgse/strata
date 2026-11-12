// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::{Rc, Weak},
    time::Duration,
};

use crate::{
    model::{FileEntry, Location, SortKey},
    services::{
        DirectoryRequest, DirectoryStamp, LoadHandle, RequestId, StampAnswer, WatchRequest,
        Watching,
    },
};

use super::{
    Browser, BrowserEvent, ColumnMonitor, DIRECTORY_LOAD_TIME_BUDGET, MAX_DIRECTORY_ENTRIES,
    MonitorState, NATIVE_DIRECTORY_BATCH_SIZE, REMOTE_DIRECTORY_BATCH_SIZE, SelectionUpdate,
    StagingLoad,
};

impl Browser {
    pub(super) fn start_load(
        self: &Rc<Self>,
        depth: usize,
        location: Location,
        request_id: RequestId,
    ) {
        if depth == 0
            && self
                .root_identity
                .borrow()
                .as_ref()
                .is_some_and(|(known, _)| *known != location)
        {
            self.root_identity.take();
        }
        self.monitors.borrow_mut().push(ColumnMonitor::default());
        let handle = self.request_directory(DirectoryLoad {
            depth,
            location,
            request_id,
            background: false,
        });
        self.loads.borrow_mut().push(handle);
    }

    /// A folder that vanished before its reload keeps the identity it loaded with.
    pub(in crate::app) fn record_stamp(&self, request_id: RequestId, stamp: DirectoryStamp) {
        let depth = self.state.borrow().depth_for_request(request_id);
        if let Some(depth) = depth {
            self.record_baseline(depth, request_id, &stamp);
        }
        if depth == Some(0)
            && let (Some(identity), Some(location)) = (stamp.identity, self.location_at(0))
        {
            self.root_identity.replace(Some((location, identity)));
        }
    }

    pub(in crate::app) fn watch_answered_folder(
        self: &Rc<Self>,
        request_id: RequestId,
        baseline: Option<&DirectoryStamp>,
    ) {
        let Some(depth) = self.state.borrow().depth_for_request(request_id) else {
            return;
        };
        let Some(location) = self.location_at(depth) else {
            return;
        };
        let (state, gap_check) = {
            let mut monitors = self.monitors.borrow_mut();
            let Some(monitor) = monitors.get_mut(depth).filter(|monitor| !monitor.requested) else {
                return;
            };
            monitor.requested = true;
            (monitor.state.clone(), monitor.gap_check.clone())
        };
        let weak: Weak<Self> = Rc::downgrade(self);
        let watched = location.clone();
        let notify = Rc::new(move |change| {
            if let Some(browser) = weak.upgrade() {
                browser.handle_directory_change(depth, &watched, change);
            }
        });
        let weak: Weak<Self> = Rc::downgrade(self);
        let gap = MonitorGap {
            depth,
            state: state.clone(),
            gap_check,
            location: location.clone(),
        };
        let baseline = baseline.cloned();
        // Hidden entries remain in the model so revealing them never needs a rescan.
        let request = WatchRequest {
            location,
            include_hidden: true,
            began_on: baseline.as_ref().and_then(|baseline| baseline.identity),
        };
        let report = Rc::new(move |watching| {
            let browser = weak.upgrade();
            match (watching, &browser, &baseline) {
                (Watching::Unavailable, ..) => gap.state.set(MonitorState::Unavailable),
                (Watching::Live, Some(browser), Some(baseline)) => {
                    browser.check_monitor_gap(&gap, baseline);
                }
                (Watching::Live, ..) => gap.state.set(MonitorState::Active),
            }
            if let Some(browser) = browser {
                browser.reschedule_freshness();
            }
        });
        let watch = self.source.start_watch(request, notify, report);
        if let Some(monitor) = self
            .monitors
            .borrow_mut()
            .get_mut(depth)
            .filter(|monitor| Rc::ptr_eq(&monitor.state, &state))
        {
            monitor.watch = Some(watch);
        }
    }

    /// A monitor that went live after the listing may have missed changes in
    /// between; a moved stamp reloads the column.
    fn check_monitor_gap(self: &Rc<Self>, gap: &MonitorGap, baseline: &DirectoryStamp) {
        let weak: Weak<Self> = Rc::downgrade(self);
        let depth = gap.depth;
        let (state, checked, baseline) =
            (gap.state.clone(), gap.location.clone(), baseline.clone());
        let probe = self.source.probe_stamp(
            gap.location.clone(),
            Rc::new(move |answer| {
                state.set(MonitorState::Active);
                let Some(browser) = weak.upgrade() else {
                    return;
                };
                browser.reschedule_freshness();
                let current = browser.monitor_still_watches(depth, &state, &checked);
                if current && stamp_moved(&answer, &baseline) {
                    browser.refresh_column_preserving_entries(depth);
                }
            }),
        );
        gap.gap_check.replace(Some(probe));
    }

    fn monitor_still_watches(
        &self,
        depth: usize,
        state: &Rc<Cell<MonitorState>>,
        location: &Location,
    ) -> bool {
        self.monitors
            .borrow()
            .get(depth)
            .is_some_and(|monitor| Rc::ptr_eq(&monitor.state, state))
            && self.location_at(depth).as_ref() == Some(location)
    }

    pub(super) fn apply_owned_batch(
        self: &Rc<Self>,
        request_id: RequestId,
        entries: Vec<FileEntry>,
    ) {
        let install_started = std::time::Instant::now();
        let mut state = self.state.borrow_mut();
        let batch_len = entries.len();
        let Some((depth, insertions)) = state.apply_batch(request_id, entries) else {
            return;
        };
        tracing::debug!(
            request_id = request_id.0,
            location = %state.columns[depth].location.diagnostic_path(),
            entries = batch_len,
            "directory batch accepted"
        );
        let selected = state.columns[depth].selected;
        let take_focus = selected.is_some() && state.take_selection_from_reveal(depth);
        drop(state);
        self.apply_resolved_location_reveal(depth, request_id);
        crate::metrics::record_stage(
            "state-install",
            install_started.elapsed().as_millis() as u64,
        );
        self.emit(BrowserEvent::EntriesInserted { depth, insertions });
        // The full-column scan is the most expensive per-batch work after the merge.
        if let Some(focused) = selected {
            let positions = self.state.borrow().selected_positions(depth);
            let current = (request_id, positions.clone(), focused);
            let mut last = self.last_batch_selection.borrow_mut();
            if last.get(&depth) != Some(&current) {
                last.insert(depth, current);
                drop(last);
                self.emit(BrowserEvent::SelectionSetChanged {
                    depth,
                    selection: SelectionUpdate::Positions(positions),
                    focused,
                    take_focus,
                });
            }
        }
    }

    pub(super) fn stage_batch(
        self: &Rc<Self>,
        request_id: RequestId,
        depth: usize,
        entries: Vec<FileEntry>,
    ) {
        let mut staging = self.staging.borrow_mut();
        let slot = staging.entry(depth).or_insert_with(|| StagingLoad {
            request_id,
            entries: Vec::new(),
            removed: HashSet::new(),
            deltas: Vec::new(),
            metadata_incomplete: false,
        });
        if slot.request_id != request_id {
            *slot = StagingLoad {
                request_id,
                entries,
                removed: HashSet::new(),
                deltas: Vec::new(),
                metadata_incomplete: false,
            };
            return;
        }
        if slot.entries.is_empty() {
            slot.entries = entries;
        } else {
            slot.entries.extend(entries);
        }
    }

    pub(super) fn request_directory(self: &Rc<Self>, load: DirectoryLoad) -> LoadHandle {
        let DirectoryLoad {
            depth,
            location,
            request_id,
            background,
        } = load;
        self.note_listing_started(depth, request_id, background);
        let weak: Weak<Self> = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            if let Some(browser) = weak.upgrade() {
                browser.handle_directory_event(event);
            }
        });
        let batch_size = if location.native_path().is_some() {
            NATIVE_DIRECTORY_BATCH_SIZE
        } else {
            REMOTE_DIRECTORY_BATCH_SIZE
        };
        // Size/date loads stat inline: sorting placeholders and re-sorting afterwards
        // costs more than one stat per file up front.
        let sort_key = self
            .state
            .borrow()
            .column_preferences(depth)
            .map(|preferences| preferences.sort_key)
            .unwrap_or_else(|| self.preferences.get().sort_key);
        let include_metadata = matches!(
            sort_key,
            SortKey::Size | SortKey::Modified | SortKey::Created
        );
        let (max_entries, time_budget) = if location.is_camera_photo_root() {
            (usize::MAX, Duration::MAX)
        } else {
            (MAX_DIRECTORY_ENTRIES, DIRECTORY_LOAD_TIME_BUDGET)
        };
        self.source.enumerate(
            DirectoryRequest {
                id: request_id,
                location,
                batch_size,
                include_metadata,
                max_entries,
                time_budget,
                background,
            },
            emit,
        )
    }
}

/// One column listing to start.
pub(super) struct DirectoryLoad {
    pub(super) depth: usize,
    pub(super) location: Location,
    pub(super) request_id: RequestId,
    pub(super) background: bool,
}

/// The monitor of one column, checked for changes it may have missed while starting.
struct MonitorGap {
    depth: usize,
    state: Rc<Cell<MonitorState>>,
    gap_check: Rc<RefCell<Option<LoadHandle>>>,
    location: Location,
}

fn stamp_moved(answer: &StampAnswer, baseline: &DirectoryStamp) -> bool {
    match answer {
        StampAnswer::Stamp(stamp) => stamp.moved_since(baseline) || baseline.may_hide_changes(),
        StampAnswer::Missing => true,
        _ => false,
    }
}
