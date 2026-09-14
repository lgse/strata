// SPDX-License-Identifier: MIT

use super::super::*;
use std::cell::RefCell;

#[test]
fn progress_is_cumulative_across_nested_branches_and_reported_in_batches() {
    let root = tempfile::tempdir().expect("fixture");
    for directory in ["one", "two", ".hidden/visible"] {
        let directory = root.path().join(directory);
        std::fs::create_dir_all(&directory).expect("directory");
        for index in 0..100 {
            std::fs::write(directory.join(format!("file-{index}")), b"abc").expect("file");
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
    assert_eq!(summary.total_size, 900);
    assert_eq!(summary.visible_file_count, 201);
    assert_eq!(summary.visible_folder_count, 2);
    assert!(!summary.truncated());
    assert_eq!(updates.first(), Some(&DirectorySummary::default()));
    assert_eq!(updates.last(), Some(&summary));
    assert!(
        updates
            .iter()
            .any(|total| total.total_size > 0 && total.total_size < 900)
    );
    assert!(
        updates
            .iter()
            .any(|total| total.visible_file_count > 0 && total.visible_file_count < 201)
    );
    assert!(updates.windows(2).all(|pair| {
        pair[0].item_count < pair[1].item_count
            && pair[0].total_size <= pair[1].total_size
            && pair[0].visible_file_count <= pair[1].visible_file_count
            && pair[0].visible_folder_count <= pair[1].visible_folder_count
    }));
    assert!(updates.len() < 200, "report batches, not individual files");
}

#[test]
fn zero_byte_entries_still_report_count_progress() {
    for folders in [false, true] {
        let root = tempfile::tempdir().expect("fixture");
        for index in 0..200 {
            let path = root.path().join(index.to_string());
            if folders {
                std::fs::create_dir(path).expect("empty folder");
            } else {
                std::fs::write(path, b"").expect("empty file");
            }
        }
        let updates = Rc::new(RefCell::new(Vec::new()));
        let observed = updates.clone();
        let summary = glib::MainContext::new()
            .block_on(summarize_directory_with_progress(
                &gio::File::for_path(root.path()),
                move |total| observed.borrow_mut().push(total),
            ))
            .expect("summary");
        let count = |total: &DirectorySummary| {
            if folders {
                total.visible_folder_count
            } else {
                total.visible_file_count
            }
        };
        let updates = updates.borrow();
        assert_eq!(count(&summary), 200);
        assert_eq!(updates.last(), Some(&summary));
        assert!(updates.iter().all(|total| total.total_size == 0));
        assert!(
            updates
                .iter()
                .any(|total| count(total) > 0 && count(total) < 200)
        );
        assert!(
            updates
                .windows(2)
                .all(|pair| count(&pair[0]) < count(&pair[1]))
        );
    }
}

#[test]
fn truncated_measurements_finish_at_the_last_reported_size() {
    let root = tempfile::tempdir().expect("fixture");
    std::fs::create_dir(root.path().join("nested")).expect("nested folder");
    std::fs::write(
        root.path().join("nested/excluded"),
        b"hidden by depth guard",
    )
    .expect("nested file");
    for index in 0..5 {
        std::fs::write(root.path().join(index.to_string()), b"abc").expect("file");
    }
    let last_size = Rc::new(Cell::new(DirectorySummary::default()));
    let observed = last_size.clone();
    let summary = glib::MainContext::new()
        .block_on(summarize_directory_with_budget(
            &gio::File::for_path(root.path()),
            0,
            TIME_BUDGET,
            move |total| observed.set(total),
        ))
        .expect("bounded summary");
    assert!(summary.truncated());
    assert!(summary.issues.depth_limited);
    assert!(!summary.issues.timed_out);
    assert!(!summary.issues.unreadable);
    assert_eq!(summary.total_size, 15);
    assert_eq!(summary.total_size, last_size.get().total_size);
    assert_eq!(summary.visible_file_count, 5);
    assert_eq!(
        summary.visible_file_count,
        last_size.get().visible_file_count
    );
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
        deadline: Instant::now() + TIME_BUDGET,
        max_depth: MAX_DEPTH,
        total: Cell::default(),
        reported: Cell::default(),
        on_progress: Box::new(move |_| {
            let (closed, error) = closed_enumerator.close(gio::Cancellable::NONE);
            assert!(closed, "close enumerator: {error:?}");
        }),
    });
    let summary = context
        .block_on(measure_children(
            &file,
            enumerator,
            0,
            false,
            budget.clone(),
        ))
        .expect("partial summary");
    assert!(summary.truncated());
    assert!(summary.issues.unreadable);
    assert!(!summary.issues.timed_out);
    assert!(!summary.issues.depth_limited);
    assert!(summary.total_size > 0 && summary.total_size < 600);
    assert_eq!(summary.total_size, budget.reported.get().total_size);
    assert_eq!(
        summary.visible_file_count,
        budget.reported.get().visible_file_count
    );
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
            while updates
                .borrow()
                .last()
                .is_none_or(|total| total.item_count == 0)
            {
                assert!(Instant::now() < deadline, "measurement did not start");
                context.iteration(false);
            }
            let before = updates.borrow().clone();
            assert!(before.last().expect("progress").total_size < 600);
            task.abort();
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert_eq!(*updates.borrow(), before);
        })
        .expect("main context");
}
