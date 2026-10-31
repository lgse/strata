// SPDX-License-Identifier: MIT
use super::*;

#[test]
fn coalescing_preserves_each_scope_revision_and_all_reply_slots() {
    let updates = Updates::default();
    for (path, revision) in [("/account-a", 1), ("/account-b", 50), ("/account-b", 100)] {
        updates.publish(Reply {
            version: 1,
            event: Some("invalidate".into()),
            paths: Some(vec![path.into()]),
            revision: Some(revision),
            ..Reply::default()
        });
    }
    updates.publish(Reply {
        version: 1,
        id: Some(9),
        revision: Some(1),
        decorations: Some(Vec::new()),
        ..Reply::default()
    });
    let (batch, closed) = updates.drain();
    assert!(!closed);
    let events: Vec<_> = batch
        .iter()
        .filter_map(|update| match update {
            Update::Reply(reply) if reply.event.is_some() => {
                Some((reply.paths.clone(), reply.revision))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        events,
        [
            (Some(vec!["/account-a".into()]), Some(1)),
            (Some(vec!["/account-b".into()]), Some(100))
        ]
    );
    assert!(matches!(
        batch.last(),
        Some(Update::Reply(reply)) if reply.id == Some(9)
    ));
}

#[test]
fn events_after_a_snapshot_stay_after_it_even_when_the_host_is_slow() {
    let updates = Updates::default();
    for revision in 1..=32 {
        updates.publish(Reply {
            version: 1,
            event: Some("invalidate".into()),
            revision: Some(revision),
            ..Reply::default()
        });
        updates.publish(Reply {
            version: 1,
            id: Some(revision),
            revision: Some(revision),
            decorations: Some(Vec::new()),
            ..Reply::default()
        });
    }
    assert!(!updates.has_room());
    for revision in 33..=100 {
        updates.publish(Reply {
            version: 1,
            event: Some("invalidate".into()),
            revision: Some(revision),
            ..Reply::default()
        });
    }
    let (batch, _) = updates.drain();
    let mut before = None;
    let mut snapshots = 0;
    for update in batch {
        let Update::Reply(reply) = update else {
            panic!("healthy provider went offline")
        };
        if reply.event.is_some() {
            before = reply.revision;
        } else {
            assert_eq!(reply.revision, before);
            snapshots += 1;
        }
    }
    assert_eq!(snapshots, 32);
    assert_eq!(before, Some(100));
    assert!(updates.has_room());
}
