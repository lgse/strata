// SPDX-License-Identifier: MIT

use std::{os::unix::fs::symlink, task::Poll};

use super::*;

const COMPLETE: Option<MetadataOutcome> = Some(MetadataOutcome::Complete);

fn listable_by_location<'a>(
    pairs: impl Iterator<Item = (&'a Location, &'a MetadataValue<bool>)>,
) -> HashMap<Location, MetadataValue<bool>> {
    pairs
        .map(|(location, listable)| (location.clone(), listable.clone()))
        .collect()
}

fn ready_after_one_yield() -> impl std::future::Future<Output = ()> {
    let mut yielded = false;
    std::future::poll_fn(move |cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
}

/// Waits, polled at least every 20 ms, until `events` holds a finished fill or `limit` passes.
async fn finished_or_timed_out(events: &Events, limit: Duration) {
    let waker: Rc<RefCell<Option<std::task::Waker>>> = Rc::default();
    let wake = waker.clone();
    let ticker = glib::timeout_add_local(Duration::from_millis(20), move || {
        if let Some(waker) = wake.borrow_mut().take() {
            waker.wake();
        }
        glib::ControlFlow::Continue
    });
    let deadline = Instant::now() + limit;
    std::future::poll_fn(|cx| {
        let done = events
            .borrow()
            .iter()
            .any(|event| matches!(event, DirectoryEvent::MetadataFinished { .. }));
        if done || Instant::now() >= deadline {
            Poll::Ready(())
        } else {
            *waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }
    })
    .await;
    ticker.remove();
}

#[test]
fn metadata_marks_folders_the_user_cannot_open() {
    let root = FixtureDir::new("folder-access");
    let locked = root.path().join("locked");
    let open = root.path().join("open");
    fs::create_dir_all(&locked).expect("the locked folder should be created");
    fs::create_dir_all(&open).expect("the open folder should be created");
    let file = root.write("file.txt", b"content");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))
        .expect("the locked folder's permissions should be set");

    let scanned = run_enumerate(DirectoryRequest {
        include_metadata: true,
        ..directory_request(root.location())
    });
    let filled = run_fill(MetadataRequest {
        id: RequestId(2),
        ..metadata_request([&locked, &open, &file].map(Location::local).to_vec())
    });
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))
        .expect("the locked folder's permissions should be restored");
    root.remove();

    let scanned_entries = batched_entries(&scanned);
    let scanned = listable_by_location(
        scanned_entries
            .iter()
            .map(|entry| (&entry.location, &entry.listable)),
    );
    let filled = listable_by_location(
        filled_updates(&filled).map(|update| (&update.location, &update.listable)),
    );
    // Root reads every folder, so nothing is locked for it.
    let locked_access = MetadataValue::Known(rustix::process::geteuid().is_root());
    for listable in [scanned, filled] {
        assert_eq!(listable[&Location::local(&locked)], locked_access);
        assert_eq!(
            listable[&Location::local(&open)],
            MetadataValue::Known(true)
        );
        assert_eq!(
            listable[&Location::local(&file)],
            MetadataValue::Unavailable
        );
    }
}

#[test]
fn fill_empty_entries_completes_without_chunks() {
    let events = run_fill(MetadataRequest {
        full: true,
        ..metadata_request(Vec::new())
    });

    assert_eq!(fill_summary(&events), (COMPLETE, 0));
}

#[test]
fn sequential_fill_with_no_time_remaining_is_truncated() {
    let events = run_fill(MetadataRequest {
        time_budget: Duration::ZERO,
        ..metadata_request(vec![Location::local("/fixture/file.txt")])
    });

    assert_eq!(fill_summary(&events), (Some(MetadataOutcome::Truncated), 0));
}

#[test]
fn fill_all_vanished_entries_reports_failed() {
    let root = unique_fixture_root("fill-vanished");
    let events = run_fill(MetadataRequest {
        full: true,
        ..metadata_request(vec![
            Location::local(root.join("gone-0.txt")),
            Location::local(root.join("gone-1.txt")),
        ])
    });

    assert_eq!(fill_outcome(&events), Some(MetadataOutcome::Failed));
}

#[test]
fn fill_unreachable_remote_reports_failed() {
    let events = run_fill(metadata_request(vec![Location::uri(
        "sftp://host/share/photo.jpg",
    )]));

    assert_eq!(fill_summary(&events), (Some(MetadataOutcome::Failed), 1));
}

#[test]
fn fill_file_uri_stats_through_the_uri_form() {
    let root = FixtureDir::new("fill-uri");
    let path = root.write("photo.jpg", b"content");

    let uri = format!("file://{}", path.display());
    let events = run_fill(metadata_request(vec![Location::uri(uri)]));

    assert_eq!(fill_summary(&events), (COMPLETE, 1));
}

#[test]
fn fill_live_file_completes_with_a_chunk() {
    let root = FixtureDir::new("fill-live");
    let path = root.write("photo.jpg", b"content");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
        .expect("the fixture permissions should be set");

    let events = run_fill(metadata_request(vec![Location::local(&path)]));

    assert_eq!(fill_summary(&events), (COMPLETE, 1));
    assert_eq!(
        first_filled(&events, |update| (update.size.clone(), update.mode.clone())),
        Some((MetadataValue::Known(7), MetadataValue::Known(0o100640)))
    );
}

#[test]
fn native_viewport_metadata_streams_multiple_chunks() {
    let root = FixtureDir::new("fill-streaming");
    let entries: Vec<_> = (0..20)
        .map(|index| Location::local(root.write(&format!("file-{index}.txt"), b"content")))
        .collect();

    let events = run_fill(metadata_request(entries));

    assert_eq!(fill_outcome(&events), COMPLETE);
    assert!(fill_chunk_count(&events) > 1);
    root.remove();
}

#[test]
fn parallel_fill_follows_symlinks_like_enumeration() {
    let root = FixtureDir::new("fill-symlink");
    let target = root.write("target.txt", b"0123456789");
    let link = root.path().join("link.txt");
    symlink(&target, &link).expect("the symlink should be created");
    let subdir = root.path().join("sub");
    fs::create_dir_all(&subdir).expect("the subdir should be created");
    let dir_link = root.path().join("dir-link");
    symlink(&subdir, &dir_link).expect("the dir symlink should be created");

    let events = run_fill(MetadataRequest {
        full: true,
        ..metadata_request([&link, &dir_link, &target].map(Location::local).to_vec())
    });
    assert_eq!(fill_outcome(&events), COMPLETE);
    let by_location = |wanted: &Path, what: &str| {
        let wanted = Location::local(wanted);
        filled_updates(&events)
            .find(|update| update.location == wanted)
            .cloned()
            .unwrap_or_else(|| panic!("the {what} should fill"))
    };
    let link_update = by_location(&link, "link");
    let target_update = by_location(&target, "target");
    assert_eq!(
        (&link_update.size, &link_update.modified_unix_seconds),
        (&target_update.size, &target_update.modified_unix_seconds)
    );
    assert_eq!(link_update.size, MetadataValue::Known(10));
    let dir_update = by_location(&dir_link, "dir link");
    assert_eq!(
        (&dir_update.size, &dir_update.child_count),
        (&MetadataValue::Unknown, &MetadataValue::Unknown)
    );
    assert!(matches!(
        dir_update.modified_unix_seconds,
        MetadataValue::Known(_)
    ));
}

#[test]
fn parallel_fill_cancellation_reports_cancelled_without_chunks() {
    let _serial = serial();
    let root = FixtureDir::new("fill-cancel");
    let count = 20_000;
    for index in 0..100 {
        root.write(&format!("file-{index:04}.txt"), b"content");
    }
    let entries: Vec<Location> = (0..count)
        .map(|index| Location::local(root.path().join(format!("file-{index:05}.txt"))))
        .collect();
    let events = Events::default();
    let emit = recorder(&events);
    glib::MainContext::default().block_on(async {
        let handle = fill_parallel_with(
            8,
            RequestId(1),
            entries,
            false,
            Duration::from_secs(60),
            emit,
        );
        ready_after_one_yield().await;
        drop(handle);
        finished_or_timed_out(&events, Duration::from_secs(1)).await;
    });
}
