// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use super::super::super::ViewportFill;
use super::super::tests::Fixture;
use super::*;
use crate::{
    model::MetadataValue,
    services::{DirectoryChange, DirectoryEvent},
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};

fn loaded() -> Fixture {
    let fixture = Fixture::new(Location::local("/fixture"));
    fixture.batch(vec![fixture.entry("beta"), fixture.entry("alpha")]);
    fixture.finish();
    fixture.events.borrow_mut().clear();
    fixture
}

fn install_fill(fixture: &Fixture, full_sort: bool) -> RequestId {
    let request_id = fixture.browser.new_request_id();
    fixture.browser.fill_tokens.borrow_mut().insert(
        request_id,
        ViewportFill {
            depth: 0,
            directory_request: fixture.request(),
            tokens: vec![(1, fixture.entry("beta").location)],
        },
    );
    if full_sort {
        fixture.browser.sort_awaiting_fill.replace(Some(SortFill {
            generation: 1,
            depth: 0,
            fill_request: request_id,
            directory_request: fixture.request(),
            preferences: fixture.browser.preferences(),
        }));
    }
    request_id
}

fn update(fixture: &Fixture, name: &str) -> MetadataUpdate {
    MetadataUpdate {
        location: fixture.entry(name).location,
        size: MetadataValue::Known(41),
        modified_unix_seconds: MetadataValue::Known(123),
        mode: MetadataValue::Known(0o644),
    }
}

#[test]
fn shifted_viewport_tokens_are_rejected_but_sort_fills_follow_locations() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for full_sort in [false, true] {
        let fixture = loaded();
        let request_id = install_fill(&fixture, full_sort);
        fixture.browser.handle_directory_change(
            0,
            &fixture.location,
            DirectoryChange::Upsert(fixture.entry("aardvark")),
        );
        fixture.events.borrow_mut().clear();
        fixture
            .browser
            .handle_directory_event(DirectoryEvent::MetadataFilled {
                request_id,
                updates: vec![update(&fixture, "beta"), update(&fixture, "unrequested")],
            });
        let beta = fixture.browser.entry_at(0, 2).expect("shifted beta row");
        assert_eq!(beta.display_name, "beta");
        if full_sort {
            assert_eq!(beta.size, MetadataValue::Known(41));
            assert!(
                matches!(fixture.events.borrow().as_slice(), [BrowserEvent::MetadataFilled { depth: 0, updates }] if updates.len() == 1 && updates[0].0 == 2)
            );
        } else {
            assert_eq!(beta.size, MetadataValue::Unknown);
            assert!(fixture.events.borrow().is_empty());
        }
    }
}

#[test]
fn metadata_observers_can_navigate_during_both_fill_routes() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for full_sort in [false, true] {
        let fixture = loaded();
        let request_id = install_fill(&fixture, full_sort);
        let weak = Rc::downgrade(&fixture.browser);
        fixture.browser.observe(move |event| {
            if matches!(event, BrowserEvent::MetadataFilled { .. }) {
                weak.upgrade()
                    .expect("observed browser")
                    .navigate(Location::local("/replacement"));
            }
        });
        fixture
            .browser
            .handle_directory_event(DirectoryEvent::MetadataFilled {
                request_id,
                updates: vec![update(&fixture, "beta")],
            });
        assert_eq!(
            fixture.browser.active_location(),
            Some(Location::local("/replacement"))
        );
        assert!(fixture.browser.fill_tokens.borrow().is_empty());
        assert!(fixture.browser.sort_awaiting_fill.borrow().is_none());
        assert_eq!(
            fixture
                .browser
                .column_snapshot(0)
                .expect("new column")
                .count,
            0
        );
    }
}

#[test]
fn superseded_directory_identity_rejects_both_metadata_routes() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for full_sort in [false, true] {
        let fixture = loaded();
        let request_id = install_fill(&fixture, full_sort);
        let replacement = fixture.browser.new_request_id();
        fixture
            .browser
            .state
            .borrow_mut()
            .navigate(fixture.location.clone(), replacement);
        fixture
            .browser
            .handle_directory_event(DirectoryEvent::MetadataFilled {
                request_id,
                updates: vec![update(&fixture, "beta")],
            });
        assert!(fixture.events.borrow().is_empty());
        assert_eq!(
            fixture
                .browser
                .column_snapshot(0)
                .expect("replacement column")
                .count,
            0
        );
    }
}

#[test]
fn positioned_updates_keep_token_membership_and_update_order() {
    let fixture_location = Location::local("/fixture/beta");
    let update = MetadataUpdate {
        location: fixture_location.clone(),
        size: MetadataValue::Known(41),
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
    };
    let other = MetadataUpdate {
        location: Location::local("/fixture/other"),
        ..update.clone()
    };
    let later = MetadataUpdate {
        size: MetadataValue::Known(42),
        ..update.clone()
    };
    let positioned = position_updates(
        &[(1, fixture_location.clone()), (3, fixture_location)],
        vec![update, other, later],
    );
    assert_eq!(positioned.len(), 2);
    assert_eq!((positioned[0].0, positioned[1].0), (3, 3));
    assert_eq!(positioned[0].1.size, MetadataValue::Known(41));
    assert_eq!(positioned[1].1.size, MetadataValue::Known(42));
}
