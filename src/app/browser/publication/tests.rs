// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};

use super::*;
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue, SortKey, ViewPreferences},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};

struct PendingSource;

impl FileSource for PendingSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }
    fn enumerate(&self, _: DirectoryRequest, _: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        LoadHandle::new(|| {})
    }
}

struct Fixture {
    browser: Rc<Browser>,
    events: Rc<RefCell<Vec<BrowserEvent>>>,
}

impl Fixture {
    fn new(count: usize) -> Self {
        let browser = Browser::new(Rc::new(PendingSource));
        browser.navigate(Location::local("/fixture"));
        let request = browser
            .state
            .borrow()
            .request_id_for_depth(0)
            .expect("request");
        {
            let mut state = browser.state.borrow_mut();
            state.install_snapshot(request, (0..count).map(entry).collect());
            state.finish(request, false, None, None);
            if let Some(last) = count.checked_sub(1) {
                state.select(0, last);
            }
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        Self { browser, events }
    }

    fn plan(&self, terminal: PublishTerminal) -> PublicationPlan {
        let state = self.browser.state.borrow();
        PublicationPlan {
            request_id: state.request_id_for_depth(0).expect("request"),
            total: state.columns[0].entries.len(),
            focused: state.columns[0].selected,
            positions: state.selected_positions(0),
            terminal,
        }
    }

    fn publish(&self) {
        self.browser
            .publish_staged(0, self.plan(PublishTerminal::SortingFinished));
    }

    fn wait_for_completion(&self) {
        pump_until(|| self.browser.staged_publishes.borrow().is_empty());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.browser.clear_observer();
        self.browser.cancel_deferred_work();
    }
}

fn entry(index: usize) -> FileEntry {
    let name = format!("entry-{index:05}");
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.clone().into(),
        display_name: name,
        thumbnail_path: None,
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn pump_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "publication did not converge");
        context.iteration(false);
    }
}

fn assert_selection_then_sorting(events: &[BrowserEvent], focused: usize) {
    assert!(matches!(&events[events.len() - 2],
        BrowserEvent::SelectionSetChanged { depth: 0, positions, focused: actual, take_focus: false }
        if *actual == focused && positions == &[focused]));
    assert!(matches!(
        events.last(),
        Some(BrowserEvent::SortingFinished { depth: 0 })
    ));
}

#[test]
fn chunks_are_bounded_by_budget_snapshot_and_current_model() {
    for (published, total, available, expected) in [
        (128, 5000, 5000, 2048),
        (128, 700, 900, 572),
        (128, 900, 700, 572),
        (700, 700, 900, 0),
        (700, 900, 700, 0),
        (700, 900, 600, 0),
    ] {
        let staged = StagedPublish {
            published,
            plan: PublicationPlan {
                request_id: RequestId(1),
                total,
                focused: None,
                positions: vec![],
                terminal: PublishTerminal::SortingFinished,
            },
        };
        assert_eq!(staged.next_chunk(available), (published, expected));
    }
}

#[test]
fn inline_boundary_preserves_row_selection_and_terminal_order() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for count in [0, STAGE_INLINE_LIMIT] {
        let fixture = Fixture::new(count);
        fixture.publish();
        let events = fixture.events.borrow();
        assert!(
            matches!(events.first(), Some(BrowserEvent::EntriesReplaced { depth: 0, count: actual }) if *actual == count)
        );
        assert_eq!(events.len(), if count == 0 { 2 } else { 3 });
        if count > 0 {
            assert_selection_then_sorting(&events, count - 1);
        }
        assert!(matches!(
            events.last(),
            Some(BrowserEvent::SortingFinished { depth: 0 })
        ));
        assert!(fixture.browser.staged_publishes.borrow().is_empty());
        assert!(fixture.browser.publish_timer.borrow().is_none());
    }
}

#[test]
fn staged_boundary_defers_selection_and_completion_until_tails() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let fixture = Fixture::new(STAGE_INLINE_LIMIT + 1);
    fixture.publish();
    assert!(matches!(
        fixture.events.borrow().as_slice(),
        [BrowserEvent::EntriesReplaced {
            depth: 0,
            count: FIRST_PUBLISH_COUNT
        }]
    ));
    assert!(fixture.browser.publish_timer.borrow().is_some());
    fixture.wait_for_completion();
    let events = fixture.events.borrow();
    assert_eq!(events.len(), 4);
    assert!(
        matches!(events[1], BrowserEvent::EntriesPublished { depth: 0, position: FIRST_PUBLISH_COUNT, count } if count == STAGE_INLINE_LIMIT + 1 - FIRST_PUBLISH_COUNT)
    );
    assert_selection_then_sorting(&events, STAGE_INLINE_LIMIT);
    assert!(fixture.browser.publish_timer.borrow().is_none());
}

#[test]
fn slow_observers_yield_between_chunks_without_early_completion() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let total = FIRST_PUBLISH_COUNT + PUBLISH_TAIL_CHUNK * 2 + 9;
    let fixture = Fixture::new(total);
    let delayed = Rc::new(Cell::new(false));
    let observed = delayed.clone();
    fixture.browser.observe(move |event| {
        if matches!(event, BrowserEvent::EntriesPublished { .. }) && !observed.replace(true) {
            // Consume the slice deliberately; this is not a wait for async state.
            std::thread::sleep(PUBLISH_SLICE_BUDGET);
        }
    });
    fixture.publish();
    pump_until(|| delayed.get());
    assert_eq!(fixture.events.borrow().len(), 2);
    assert_eq!(
        fixture.browser.staged_publishes.borrow()[&0].published,
        FIRST_PUBLISH_COUNT + PUBLISH_TAIL_CHUNK
    );
    assert!(fixture.browser.publish_timer.borrow().is_some());
    fixture.wait_for_completion();
    let events = fixture.events.borrow();
    let chunks: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::EntriesPublished {
                position, count, ..
            } => Some((*position, *count)),
            _ => None,
        })
        .collect();
    assert_eq!(chunks, vec![(128, 2048), (2176, 2048), (4224, 9)]);
    assert_selection_then_sorting(&events, total - 1);
}

#[test]
fn draining_uses_current_rows_and_completes_only_once() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let fixture = Fixture::new(700);
    fixture.browser.publish_staged(
        0,
        fixture.plan(PublishTerminal::LoadFinished {
            truncated: true,
            retry_metadata: false,
        }),
    );
    fixture.browser.state.borrow_mut().columns[0]
        .entries
        .extend((700..703).map(entry));
    fixture.events.borrow_mut().clear();
    fixture.browser.drain_publish(0);
    {
        let events = fixture.events.borrow();
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events[0],
            BrowserEvent::EntriesPublished {
                depth: 0,
                position: 128,
                count: 575
            }
        ));
        assert!(
            matches!(&events[1], BrowserEvent::SelectionSetChanged { focused: 699, positions, take_focus: false, .. } if positions == &[699])
        );
        assert!(matches!(
            events[2],
            BrowserEvent::LoadFinished {
                depth: 0,
                truncated: true
            }
        ));
    }
    fixture.browser.drain_publish(0);
    pump_until(|| fixture.browser.publish_timer.borrow().is_none());
    assert_eq!(fixture.events.borrow().len(), 3);
}

#[test]
fn stale_request_discards_queued_rows_and_terminal() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let fixture = Fixture::new(700);
    fixture.publish();
    fixture.events.borrow_mut().clear();
    fixture
        .browser
        .state
        .borrow_mut()
        .navigate(Location::local("/replacement"), RequestId(99));
    fixture.wait_for_completion();
    assert!(fixture.events.borrow().is_empty());
    assert!(fixture.browser.publish_timer.borrow().is_none());
    assert!(fixture.browser.state.borrow().columns[0].entries.is_empty());
}

#[test]
fn metadata_retry_starts_only_after_load_completion() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for count in [1, 700] {
        for retry_metadata in [false, true] {
            let fixture = Fixture::new(count);
            fixture.browser.state.borrow_mut().apply_sort_preferences(
                0,
                ViewPreferences {
                    sort_key: SortKey::Size,
                    ..ViewPreferences::default()
                },
            );
            fixture.browser.publish_staged(
                0,
                fixture.plan(PublishTerminal::LoadFinished {
                    truncated: false,
                    retry_metadata,
                }),
            );
            fixture.wait_for_completion();
            let events = fixture.events.borrow();
            let finished = events
                .iter()
                .position(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
                .expect("load completion");
            let retry = events
                .iter()
                .position(|event| matches!(event, BrowserEvent::SortingStarted { .. }));
            assert_eq!(retry.is_some(), retry_metadata);
            assert!(retry.is_none_or(|position| position > finished));
        }
    }
}

#[test]
fn sort_paths_capture_selection_before_preference_observers() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for awaited in [false, true] {
        let fixture = Fixture::new(700);
        let weak = Rc::downgrade(&fixture.browser);
        fixture.browser.observe_preferences(move |_| {
            weak.upgrade()
                .expect("browser")
                .state
                .borrow_mut()
                .select(0, 0);
        });
        fixture.browser.pending_sort.set(Some((17, 0)));
        if awaited {
            fixture
                .browser
                .finish_awaited_sort(0, 17, ViewPreferences::default());
        } else {
            fixture.browser.apply_debounced_sort(0, 17, |_| {});
        }
        assert_eq!(fixture.browser.state.borrow().columns[0].selected, Some(0));
        {
            let staged = fixture.browser.staged_publishes.borrow();
            assert_eq!(staged[&0].plan.focused, Some(699));
            assert_eq!(staged[&0].plan.positions, vec![699]);
            assert_eq!(staged[&0].plan.total, 700);
        }
        fixture.wait_for_completion();
        assert_selection_then_sorting(&fixture.events.borrow(), 699);
    }
}

#[test]
fn cancelling_last_publication_removes_the_idle_source() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let fixture = Fixture::new(700);
    fixture.publish();
    fixture.events.borrow_mut().clear();
    fixture.browser.cancel_publish(0);
    fixture.browser.cancel_publish(0);
    assert!(fixture.browser.staged_publishes.borrow().is_empty());
    assert!(fixture.browser.publish_timer.borrow().is_none());
    assert!(fixture.events.borrow().is_empty());
}

#[test]
fn tail_observer_can_cancel_publication_without_borrowing_or_completion() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let fixture = Fixture::new(5000);
    let weak = Rc::downgrade(&fixture.browser);
    fixture.browser.observe(move |event| {
        if matches!(event, BrowserEvent::EntriesPublished { .. }) {
            let browser = weak.upgrade().expect("browser");
            assert_eq!(browser.state.borrow_mut().columns.len(), 1);
            browser.cancel_publish(0);
        }
    });
    fixture.publish();
    fixture.wait_for_completion();
    assert!(matches!(
        fixture.events.borrow().as_slice(),
        [
            BrowserEvent::EntriesReplaced { .. },
            BrowserEvent::EntriesPublished { .. }
        ]
    ));
    assert!(fixture.browser.publish_timer.borrow().is_none());
}
