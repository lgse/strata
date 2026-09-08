// SPDX-License-Identifier: GPL-3.0-or-later

use super::super::*;
use std::cell::RefCell;

#[test]
fn progress_is_cumulative_across_nested_branches_and_reported_in_batches() {
    let root = tempfile::tempdir().expect("fixture");
    for directory in ["one", "two"] {
        let directory = root.path().join(directory);
        std::fs::create_dir(&directory).expect("directory");
        for index in 0..100 {
            std::fs::write(directory.join(format!(".file-{index}")), b"abc").expect("file");
        }
    }
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).expect("symlink");
    let updates = Rc::new(RefCell::new(Vec::new()));
    let observed = updates.clone();
    let summary = glib::MainContext::new()
        .block_on(summarize_directory_with_progress(
            &gio::File::for_path(root.path()),
            move |total| observed.borrow_mut().push(total),
        ))
        .expect("summary");
    let updates = updates.borrow();
    assert_eq!(summary.total_size, 600);
    assert!(!summary.truncated);
    assert_eq!(updates.first(), Some(&0));
    assert_eq!(updates.last(), Some(&summary.total_size));
    assert!(updates.iter().any(|total| *total > 0 && *total < 600));
    assert!(updates.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(updates.len() < 200, "report batches, not individual files");
}

#[test]
fn truncated_measurements_finish_at_the_last_reported_size() {
    let root = tempfile::tempdir().expect("fixture");
    for index in 0..100 {
        std::fs::write(root.path().join(index.to_string()), b"abc").expect("file");
    }
    let last_size = Rc::new(Cell::new(0));
    let observed = last_size.clone();
    let summary = glib::MainContext::new()
        .block_on(summarize_directory_with_budget(
            &gio::File::for_path(root.path()),
            5,
            MAX_DEPTH,
            TIME_BUDGET,
            move |total| observed.set(total),
        ))
        .expect("bounded summary");
    assert!(summary.truncated);
    assert_eq!(summary.total_size, 15);
    assert_eq!(summary.total_size, last_size.get());
}

#[test]
fn an_enumeration_failure_preserves_bytes_already_reported() {
    let root = tempfile::tempdir().expect("fixture");
    for index in 0..200 {
        std::fs::write(root.path().join(index.to_string()), b"abc").expect("file");
    }
    let context = glib::MainContext::new();
    let file = gio::File::for_path(root.path());
    let enumerator = context
        .block_on(enumerate_children(&file))
        .expect("enumerator");
    let closed_enumerator = enumerator.clone();
    let budget = Rc::new(MeasurementBudget {
        visited: Cell::new(0),
        deadline: Instant::now() + TIME_BUDGET,
        max_entries: MAX_ENTRIES,
        max_depth: MAX_DEPTH,
        total_size: Cell::new(0),
        reported_size: Cell::new(0),
        on_progress: Box::new(move |_| {
            let (closed, error) = closed_enumerator.close(gio::Cancellable::NONE);
            assert!(closed, "close enumerator: {error:?}");
        }),
    });
    let summary = context
        .block_on(measure_children(&file, enumerator, 0, budget.clone()))
        .expect("partial summary");
    assert!(summary.truncated);
    assert!(summary.total_size > 0 && summary.total_size < 600);
    assert_eq!(summary.total_size, budget.reported_size.get());
}

#[test]
fn aborting_measurement_stops_progress_callbacks() {
    let root = tempfile::tempdir().expect("fixture");
    for index in 0..200 {
        std::fs::write(root.path().join(index.to_string()), b"abc").expect("file");
    }
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let updates = Rc::new(RefCell::new(Vec::new()));
            let observed = updates.clone();
            let file = gio::File::for_path(root.path());
            let task = context.spawn_local(async move {
                summarize_directory_with_progress(&file, move |total| {
                    observed.borrow_mut().push(total)
                })
                .await
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            while updates.borrow().last().is_none_or(|size| *size == 0) {
                assert!(Instant::now() < deadline, "measurement did not start");
                context.iteration(false);
            }
            let before = updates.borrow().clone();
            assert!(*before.last().expect("progress") < 600);
            task.abort();
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert_eq!(*updates.borrow(), before);
        })
        .expect("main context");
}
