// SPDX-License-Identifier: MIT

use super::*;

type Pending = HashMap<PendingMonitorKey, PendingMonitorChange>;

fn flushed(pending: Pending) -> Vec<DirectoryChange> {
    let (changes, notify) = change_recorder();
    flush_monitor_changes(&RefCell::new(pending), &notify, &Rc::new(Cell::new(false)));
    changes.take()
}

fn queue_upsert(pending: &mut Pending, location: &Location) -> bool {
    queue_monitor_change(
        pending,
        Some(location.clone()),
        PendingMonitorChange::Upsert(location.clone()),
    )
}

fn queue_remove(pending: &mut Pending, location: &Location) -> bool {
    queue_monitor_change(
        pending,
        Some(location.clone()),
        PendingMonitorChange::Remove(location.clone()),
    )
}

fn queue_move(pending: &mut Pending, from: &Location, to: &Location) -> bool {
    queue_monitor_change(
        pending,
        Some(to.clone()),
        PendingMonitorChange::Move {
            from: from.clone(),
            to: to.clone(),
        },
    )
}

#[test]
fn coalescing_preserves_a_move_when_metadata_follows_it() {
    let change = merge_pending_change(
        PendingMonitorChange::Move {
            from: Location::local("/fixture/old"),
            to: Location::local("/fixture/new"),
        },
        PendingMonitorChange::Upsert(Location::local("/fixture/new")),
    );

    assert!(matches!(change, PendingMonitorChange::Move { .. }));
}

#[test]
fn hidden_monitor_changes_are_skipped_but_atomic_publication_stays_visible() {
    let watched = Location::local("/fixture");
    let hidden = Location::local("/fixture/.strata-replacement-123");
    let visible = Location::local("/fixture/report.pdf");

    assert!(
        visible_monitor_change(
            &watched,
            PendingMonitorChange::Upsert(hidden.clone()),
            false
        )
        .is_none()
    );
    assert!(matches!(
        visible_monitor_change(
            &watched,
            PendingMonitorChange::Move {
                from: hidden,
                to: visible.clone(),
            },
            false,
        ),
        Some(PendingMonitorChange::Upsert(location)) if location == visible
    ));
    assert!(
        visible_monitor_change(
            &watched,
            PendingMonitorChange::Upsert(Location::local("/fixture/.env")),
            true,
        )
        .is_some()
    );
    let hidden_folder = Location::local("/fixture/.config");
    assert!(matches!(
        visible_monitor_change(
            &hidden_folder,
            PendingMonitorChange::Remove(hidden_folder.clone()),
            false,
        ),
        Some(PendingMonitorChange::Remove(location)) if location == hidden_folder
    ));
}

#[test]
fn recent_monitor_events_rescan_without_querying_virtual_children() {
    let watched = Location::uri("recent:///");
    let child = Location::uri("recent:///virtual-child");
    let events = [
        gio::FileMonitorEvent::Created,
        gio::FileMonitorEvent::Changed,
        gio::FileMonitorEvent::Deleted,
        gio::FileMonitorEvent::Moved,
        gio::FileMonitorEvent::Renamed,
        gio::FileMonitorEvent::AttributeChanged,
        gio::FileMonitorEvent::ChangesDoneHint,
        gio::FileMonitorEvent::PreUnmount,
        gio::FileMonitorEvent::Unmounted,
    ];

    let mut pending = HashMap::new();
    for (index, event) in events.into_iter().enumerate() {
        let change = pending_monitor_change(
            &watched,
            Some(child.clone()),
            Some(Location::uri(format!("recent:///other-{index}"))),
            event,
        )
        .expect("Recent monitor activity should invalidate the collection");
        assert!(matches!(change, PendingMonitorChange::Rescan));
        assert_eq!(queue_monitor_change(&mut pending, None, change), index == 0);
    }

    assert!(matches!(
        flushed(pending).as_slice(),
        [DirectoryChange::Rescan]
    ));
}

#[test]
fn metadata_update_followed_by_move_preserves_source_removal() {
    let mut pending = HashMap::new();
    let temp = Location::local("/fixture/file.tmp");
    let dest = Location::local("/fixture/file");

    let queued = [
        queue_upsert(&mut pending, &temp),
        queue_move(&mut pending, &Location::local("/fixture/file.tmp"), &dest),
    ];

    assert_eq!(queued, [true, true]);
    assert_eq!(pending.len(), 1);
    assert!(matches!(
        pending.get(&Some(dest)),
        Some(PendingMonitorChange::Move { from, .. })
            if *from == Location::local("/fixture/file.tmp")
    ));
}

#[test]
fn move_does_not_discard_a_pending_source_removal() {
    let mut pending = HashMap::new();
    let from = Location::local("/fixture/source");
    let to = Location::local("/fixture/destination");
    queue_remove(&mut pending, &from);
    queue_move(&mut pending, &from, &to);
    assert!(matches!(
        pending.get(&Some(from)),
        Some(PendingMonitorChange::Remove(_))
    ));
    assert_eq!(pending.len(), 2);
}

#[test]
fn large_monitor_bursts_collapse_to_one_rescan() {
    let mut pending = HashMap::new();
    for index in 0..=MAX_PENDING_MONITOR_CHANGES {
        assert!(queue_upsert(
            &mut pending,
            &Location::local(format!("/fixture/{index}"))
        ));
    }

    assert_eq!(pending.len(), 1);
    assert!(matches!(
        pending.get(&None),
        Some(PendingMonitorChange::Rescan)
    ));
    assert!(!queue_remove(
        &mut pending,
        &Location::local("/fixture/ignored")
    ));
}

#[test]
fn conflicting_move_events_fall_back_to_a_rescan() {
    let _serial = serial();
    for recreate_source in [false, true] {
        let from = Location::local("/fixture/old");
        let to = Location::local("/fixture/new");
        let mut pending = HashMap::new();
        queue_move(&mut pending, &from, &to);
        if recreate_source {
            queue_upsert(&mut pending, &from);
        } else {
            queue_remove(&mut pending, &to);
        }

        assert!(
            matches!(flushed(pending).as_slice(), [DirectoryChange::Rescan]),
            "recreate source: {recreate_source}"
        );
    }
}

#[test]
fn uri_monitor_changes_keep_their_uri_locations() {
    let mut pending = HashMap::new();
    let trashed = Location::uri("trash:///report.txt");
    assert!(queue_remove(&mut pending, &trashed));

    let changes = flushed(pending);
    assert!(
        matches!(changes.as_slice(), [DirectoryChange::Remove(location)] if location == &trashed),
        "the trash URI should survive the flush unchanged"
    );
}

#[test]
fn a_directory_reporting_changes_against_itself_is_not_its_own_child() {
    let watched = Location::uri("trash:///");
    let child = Location::uri("trash:///report.txt");

    for (changed, event, target) in [
        (&watched, gio::FileMonitorEvent::Changed, None),
        (&watched, gio::FileMonitorEvent::Deleted, Some(&watched)),
        (&child, gio::FileMonitorEvent::Created, Some(&child)),
    ] {
        assert_eq!(
            monitored_change_target(&watched, Some(changed.clone()), event),
            target.cloned(),
            "{event:?}"
        );
    }
}
