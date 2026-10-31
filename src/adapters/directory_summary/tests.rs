// SPDX-License-Identifier: MIT

mod bounds;
mod progress;

use super::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

async fn summarize_directory_with_budget(
    root: &gio::File,
    max_depth: usize,
    time_budget: Duration,
) -> Result<DirectorySummary, glib::Error> {
    super::summarize_directory_with_budget(root, max_depth, time_budget, |_| {}).await
}

fn unique_fixture_root(label: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("strata-directory-{label}-{unique}"))
}

#[test]
fn directory_summary_reports_truncated_once_the_time_budget_is_exceeded() {
    let root = unique_fixture_root("time-budget");
    std::fs::create_dir_all(root.join("sub")).expect("the directory fixture should be created");
    std::fs::write(root.join("sub").join("file.txt"), b"content")
        .expect("the directory fixture file should be written");

    let summary = glib::MainContext::new().block_on(summarize_directory_with_budget(
        &gio::File::for_path(&root),
        MAX_DEPTH,
        Duration::from_nanos(1),
    ));
    std::fs::remove_dir_all(&root).expect("the directory fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert!(
        summary.truncated(),
        "an exhausted time budget should stop measurement and report truncation"
    );
    assert!(summary.issues.timed_out);
    assert!(!summary.issues.depth_limited);
    assert!(!summary.issues.unreadable);
}

#[test]
fn directory_summary_does_not_descend_past_the_depth_budget() {
    let root = unique_fixture_root("depth-budget");
    std::fs::create_dir_all(root.join("sub/nested"))
        .expect("the directory fixture should be created");
    std::fs::write(root.join("sub/nested/deep.txt"), b"content")
        .expect("the directory fixture file should be written");

    let summary = glib::MainContext::new().block_on(summarize_directory_with_budget(
        &gio::File::for_path(&root),
        1,
        TIME_BUDGET,
    ));
    std::fs::remove_dir_all(&root).expect("the directory fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert!(
        summary.truncated(),
        "descending past the depth budget should be reported"
    );
    assert!(summary.issues.depth_limited);
    assert!(!summary.issues.timed_out);
    assert!(!summary.issues.unreadable);
    assert_eq!(
        summary.item_count, 2,
        "entries past the depth budget should not be counted (root/sub and sub/nested only)"
    );
}

#[test]
fn directory_summary_treats_an_inaccessible_subdirectory_as_truncated_not_fatal() {
    use std::os::unix::fs::PermissionsExt;

    let root = unique_fixture_root("inaccessible");
    std::fs::create_dir_all(root.join("blocked")).expect("the directory fixture should be created");
    std::fs::create_dir_all(root.join("visible/deep/inner"))
        .expect("the directory fixture should be created");
    std::fs::write(root.join("visible/needle.txt"), b"content")
        .expect("the directory fixture file should be written");
    std::fs::set_permissions(root.join("blocked"), std::fs::Permissions::from_mode(0o000))
        .expect("the fixture directory's permissions should be restrictable");
    let running_as_root = std::fs::read_dir(root.join("blocked")).is_ok();

    let summary = glib::MainContext::new().block_on(summarize_directory_with_budget(
        &gio::File::for_path(&root),
        1,
        TIME_BUDGET,
    ));
    let _ = std::fs::set_permissions(root.join("blocked"), std::fs::Permissions::from_mode(0o755));
    std::fs::remove_dir_all(&root).expect("the directory fixture should be removed");

    let summary = summary.expect(
        "an inaccessible subdirectory should degrade gracefully, not fail the whole measurement",
    );
    if !running_as_root {
        assert!(
            summary.truncated(),
            "the inaccessible branch should be reported as truncated"
        );
        assert!(summary.issues.unreadable);
        assert!(!summary.issues.timed_out);
        assert!(summary.issues.depth_limited);
        assert_eq!(summary.item_count, 4);
    }
}

#[test]
fn directory_summary_treats_a_directory_removed_before_measurement_as_truncated_not_fatal() {
    let root = unique_fixture_root("changing-tree");
    let vanishing = root.join("vanishing");
    std::fs::create_dir_all(&vanishing).expect("the directory fixture should be created");
    std::fs::write(vanishing.join("inner.txt"), b"content")
        .expect("the directory fixture file should be written");

    // Capture the directory's FileInfo the way the parent-level enumeration in
    // `summarize_directory_with_budget` would, then remove it from under that handle. This models a
    // directory that changes or disappears between being observed and being measured.
    let file = gio::File::for_path(&vanishing);
    let context = glib::MainContext::new();
    let info = context
        .block_on(file.query_info_future(
            DIRECTORY_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        ))
        .expect("querying the fixture directory's info should succeed while it still exists");
    std::fs::remove_dir_all(&vanishing).expect("the fixture directory should be removable");

    let result = context.block_on(measure_entry(
        file,
        info,
        0,
        false,
        Rc::new(MeasurementBudget {
            deadline: Instant::now() + TIME_BUDGET,
            max_depth: MAX_DEPTH,
            total: Cell::default(),
            reported: Cell::default(),
            stopped: Default::default(),
            on_progress: Box::new(|_| {}),
        }),
    ));
    std::fs::remove_dir_all(&root).expect("the directory fixture root should be removed");

    let summary = result.expect(
        "a directory removed after being observed should degrade gracefully, not fail the whole measurement",
    );
    assert!(
        summary.truncated(),
        "measuring an entry that vanished before recursion should be reported as truncated"
    );
    assert!(summary.issues.unreadable);
    assert!(!summary.issues.timed_out);
    assert!(!summary.issues.depth_limited);
}

#[test]
fn aborting_a_directory_measurement_stops_it_mid_flight() {
    let root = unique_fixture_root("abort-mid-flight");
    std::fs::create_dir_all(&root).expect("the directory fixture should be created");
    let total_files = ENUMERATION_BATCH_SIZE as usize + 88;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the directory fixture file should be written");
    }

    // `spawn_local` and manual `iteration()` polling (unlike `block_on`) require this thread to
    // own the context, hence the explicit acquire via `with_thread_default` below.
    let context = glib::MainContext::new();
    let (progress_before_abort, progress_after_abort) = context
        .with_thread_default(|| {
            let info = context
                .block_on(gio::File::for_path(&root).query_info_future(
                    DIRECTORY_ATTRIBUTES,
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                ))
                .expect("querying the fixture directory's info should succeed");

            let budget = Rc::new(MeasurementBudget {
                deadline: Instant::now() + TIME_BUDGET,
                max_depth: MAX_DEPTH,
                total: Cell::default(),
                reported: Cell::default(),
                stopped: Default::default(),
                on_progress: Box::new(|_| {}),
            });
            let task = context.spawn_local(measure_entry(
                gio::File::for_path(&root),
                info,
                0,
                false,
                budget.clone(),
            ));

            for _ in 0..1_000 {
                if budget.total.get().item_count > 1 {
                    break;
                }
                context.iteration(true);
            }
            let progress_before_abort = budget.total.get().item_count;

            task.abort();
            for _ in 0..20 {
                context.iteration(false);
            }

            (progress_before_abort, budget.total.get().item_count)
        })
        .expect("a freshly created main context should be acquirable as thread-default");
    std::fs::remove_dir_all(&root).expect("the directory fixture should be removed");

    assert!(
        progress_before_abort > 1 && progress_before_abort < 1 + total_files,
        "the walk should have made partial, not complete, progress before it is aborted"
    );
    assert_eq!(
        progress_after_abort, progress_before_abort,
        "aborting mid-flight should stop the walk from making any further progress"
    );
}

#[test]
fn directory_summary_stops_enumerating_the_root_once_the_measurement_budget_is_reached() {
    let root = unique_fixture_root("root-budget-stop");
    std::fs::create_dir_all(&root).expect("the directory fixture should be created");
    let total_files = ENUMERATION_BATCH_SIZE as usize + 88;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the directory fixture file should be written");
    }

    let summary = glib::MainContext::new()
        .block_on(summarize_directory_with_budget(
            &gio::File::for_path(&root),
            MAX_DEPTH,
            Duration::ZERO,
        ))
        .expect("a plain directory tree should measure without error");
    std::fs::remove_dir_all(&root).expect("the directory fixture should be removed");

    assert!(
        summary.truncated(),
        "exceeding the measurement budget should still be reported"
    );
    assert_eq!(
        summary.item_count, 0,
        "root enumeration should stop as soon as the budget is spent, not keep requesting \
         further `next_files_future` batches for the remaining {total_files} entries"
    );
}

#[test]
fn directory_summary_does_not_stop_enumerating_siblings_after_one_branch_is_depth_truncated() {
    let root = unique_fixture_root("sibling-depth-truncation");
    let sibling_count = ENUMERATION_BATCH_SIZE as usize + 16;
    for index in 0..sibling_count {
        std::fs::create_dir_all(
            root.join("parent")
                .join(format!("sub-{index:03}"))
                .join("inner"),
        )
        .expect("the directory fixture should be created");
    }

    let summary = glib::MainContext::new().block_on(summarize_directory_with_budget(
        &gio::File::for_path(&root),
        1,
        TIME_BUDGET,
    ));
    std::fs::remove_dir_all(&root).expect("the directory fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert_eq!(
        summary.item_count,
        1 + sibling_count,
        "every sibling should be counted (1 for \"parent\" plus one per sub-N directory), \
         not just the first next_files_future batch"
    );
}

#[test]
fn measure_entry_reads_no_unset_optional_attributes() {
    let root = unique_fixture_root("missing-attributes");
    std::fs::create_dir_all(&root).expect("the directory fixture should be created");
    let critical_logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let recorded = critical_logs.clone();
    let handler = glib::log_set_handler(
        Some("GLib-GIO"),
        glib::LogLevels::LEVEL_CRITICAL,
        false,
        false,
        move |_, _, message| {
            if message.contains("standard::") || message.contains("gfileinfo.c") {
                recorded
                    .lock()
                    .expect("the critical log should be lockable")
                    .push(message.to_owned());
            }
        },
    );

    let measure = |file_type| {
        let info = gio::FileInfo::new();
        info.set_name("entry");
        info.set_file_type(file_type);
        glib::MainContext::new()
            .block_on(measure_entry(
                gio::File::for_path(&root),
                info,
                0,
                false,
                Rc::new(MeasurementBudget {
                    deadline: Instant::now() + TIME_BUDGET,
                    max_depth: MAX_DEPTH,
                    total: Cell::default(),
                    reported: Cell::default(),
                    stopped: Default::default(),
                    on_progress: Box::new(|_| {}),
                }),
            ))
            .expect("an entry without optional attributes should be measured")
    };
    let file = measure(gio::FileType::Regular);
    let folder = measure(gio::FileType::Directory);
    glib::log_remove_handler(Some("GLib-GIO"), handler);
    std::fs::remove_dir_all(&root).expect("the directory fixture should be removed");

    assert_eq!(
        critical_logs
            .lock()
            .expect("the critical log should be lockable")
            .as_slice(),
        &[] as &[String]
    );
    assert_eq!((file.visible_file_count, file.total_size), (1, 0));
    assert_eq!(folder.visible_folder_count, 1);
    assert!(!folder.truncated());
}

#[test]
fn a_file_operation_ends_the_measurement_of_a_folder_it_changes_as_incomplete() {
    for row in [
        "changing when the walk reaches it",
        "changed while it is walked",
    ] {
        let root = unique_fixture_root("released");
        std::fs::create_dir_all(root.join("sub")).expect("the directory fixture should be created");
        for index in 0..100 {
            std::fs::write(root.join(format!("file-{index}.txt")), b"content")
                .expect("the directory fixture file should be written");
        }
        std::fs::write(root.join("sub/inner.txt"), b"content")
            .expect("the directory fixture file should be written");
        let operation = crate::services::file_activity::begin(Default::default());
        let fences = Rc::new(std::cell::RefCell::new(Vec::new()));
        let changing = fences.clone();
        let fenced = root.clone();
        let on_progress: Box<dyn Fn(DirectorySummary)> = match row {
            "changing when the walk reaches it" => {
                fences
                    .borrow_mut()
                    .push(operation.fence([root.join("sub")]));
                Box::new(|_| {})
            }
            _ => Box::new(move |partial: DirectorySummary| {
                if partial.item_count > 0 && changing.borrow().is_empty() {
                    let fence = operation.fence([fenced.clone()]);
                    fence.stop_reads();
                    changing.borrow_mut().push(fence);
                }
            }),
        };

        let summary = glib::MainContext::new()
            .block_on(super::summarize_directory_with_budget(
                &gio::File::for_path(&root),
                MAX_DEPTH,
                TIME_BUDGET,
                on_progress,
            ))
            .expect("a plain directory tree should measure without error");
        fences.borrow_mut().clear();
        std::fs::remove_dir_all(&root).expect("the directory fixture should be removed");

        assert!(summary.issues.unreadable, "{row}: {summary:?}");
        match row {
            "changing when the walk reaches it" => assert!(
                summary.item_count < 102,
                "a refusal ends the walk before the folder's content: {summary:?}"
            ),
            _ => assert!(summary.item_count < 100, "the walk ends: {summary:?}"),
        }
    }
}
