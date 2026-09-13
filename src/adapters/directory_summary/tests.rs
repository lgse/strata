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
    // `next_files_future` batches 64 entries at a time, so 200 files force several suspension
    // points, giving a real window to observe partial progress before the walk would finish.
    let total_files = 200;
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
                on_progress: Box::new(|_| {}),
            });
            let task = context.spawn_local(measure_entry(
                gio::File::for_path(&root),
                info,
                0,
                false,
                budget.clone(),
            ));

            // Drive the loop only until the walk has made some real progress (at least one batch
            // beyond the root directory itself), then abort immediately -- this is genuinely
            // mid-flight since one batch (64) is far short of the full tree (1 + 200), regardless
            // of exactly how many main-loop iterations it took to get there.
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
    // More than one `next_files_future` batch (64 entries), so a walk that kept fetching
    // further batches after the budget was spent would still show up as a much larger count.
    let total_files = 150;
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
    let sibling_count = 80;
    // Nested one level under "parent" so these are children of a directory that
    // `measure_children` recurses into, not top-level entries of `root` itself (which
    // are always fully enumerated regardless of budget after the earlier deletion-worklist fix).
    for index in 0..sibling_count {
        std::fs::create_dir_all(
            root.join("parent")
                .join(format!("sub-{index:03}"))
                .join("inner"),
        )
        .expect("the directory fixture should be created");
    }

    // With max_depth 1, every "sub-N" directory individually hits the depth cap when deciding
    // whether to recurse into its own "inner" child -- that is a branch-local condition, unrelated
    // to its siblings. It used to be conflated with the shared budget being spent, which stopped
    // scanning further `next_files_future` batches entirely: with 80 siblings and a 64-entry batch
    // size, that undercounted "parent" to 1 (itself) + 64 (first batch only) = 65.
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

fn selection_entry(
    path: &std::path::Path,
    name: &str,
    size: u64,
    is_dir: bool,
) -> crate::model::FileEntry {
    use crate::model::{EntryKind, FileEntry, Location, MetadataValue};
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: std::ffi::OsString::from(name),
        display_name: name.to_owned(),
        kind: if is_dir {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        size: MetadataValue::Known(size),
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn summarize_selection_aggregates_files_and_folders() {
    let root = tempfile::tempdir().expect("fixture");
    std::fs::write(root.path().join("a.txt"), b"aaa").expect("file a");
    std::fs::write(root.path().join("b.txt"), b"bb").expect("file b");
    std::fs::create_dir(root.path().join("folder")).expect("folder");
    std::fs::write(root.path().join("folder/inner"), b"12345").expect("inner file");

    let entries = vec![
        selection_entry(&root.path().join("a.txt"), "a.txt", 3, false),
        selection_entry(&root.path().join("b.txt"), "b.txt", 2, false),
        selection_entry(&root.path().join("folder"), "folder", 0, true),
    ];

    let summary = glib::MainContext::new()
        .block_on(summarize_selection(&entries, |_| {}))
        .expect("selection summary");
    assert_eq!(summary.item_count, 3, "item_count is the selection size");
    assert_eq!(
        summary.total_size, 10,
        "total_size = 3 + 2 + 5 (recursive folder)"
    );
    assert!(!summary.truncated());
}

#[test]
fn summarize_selection_reports_progress_with_running_total() {
    let root = tempfile::tempdir().expect("fixture");
    std::fs::write(root.path().join("a.txt"), b"aaa").expect("file a");
    std::fs::create_dir(root.path().join("folder")).expect("folder");
    std::fs::write(root.path().join("folder/inner"), b"12345").expect("inner file");

    let entries = vec![
        selection_entry(&root.path().join("a.txt"), "a.txt", 3, false),
        selection_entry(&root.path().join("folder"), "folder", 0, true),
    ];

    let last_total = Rc::new(Cell::new(0u64));
    let captured = last_total.clone();
    let summary = glib::MainContext::new()
        .block_on(summarize_selection(&entries, move |partial| {
            captured.set(partial.total_size);
        }))
        .expect("selection summary");
    assert_eq!(summary.total_size, 8);
    assert!(
        last_total.get() >= 8,
        "progress callback should have seen the final total"
    );
}
