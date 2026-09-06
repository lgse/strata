// SPDX-License-Identifier: GPL-3.0-or-later

mod bounds;

use super::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

fn unique_fixture_root(label: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("strata-trash-{label}-{unique}"))
}

#[test]
fn trash_summary_reports_truncated_once_the_entry_budget_is_exceeded() {
    let root = unique_fixture_root("entry-budget");
    std::fs::create_dir_all(root.join("sub")).expect("the trash fixture should be created");
    for index in 0..5 {
        std::fs::write(
            root.join("sub").join(format!("file-{index}.txt")),
            b"content",
        )
        .expect("the trash fixture file should be written");
    }

    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        1,
        MAX_TRASH_DEPTH,
        TRASH_TIME_BUDGET,
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert!(
        summary.truncated,
        "exceeding the entry budget should be reported"
    );
    assert_eq!(
        summary.item_count, 1,
        "measurement should stop counting once the entry budget is reached"
    );
}

#[test]
fn trash_summary_reports_truncated_once_the_time_budget_is_exceeded() {
    let root = unique_fixture_root("time-budget");
    std::fs::create_dir_all(root.join("sub")).expect("the trash fixture should be created");
    std::fs::write(root.join("sub").join("file.txt"), b"content")
        .expect("the trash fixture file should be written");

    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        usize::MAX,
        MAX_TRASH_DEPTH,
        Duration::from_nanos(1),
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert!(
        summary.truncated,
        "an exhausted time budget should stop measurement and report truncation"
    );
}

#[test]
fn trash_summary_does_not_descend_past_the_depth_budget() {
    let root = unique_fixture_root("depth-budget");
    std::fs::create_dir_all(root.join("sub/nested")).expect("the trash fixture should be created");
    std::fs::write(root.join("sub/nested/deep.txt"), b"content")
        .expect("the trash fixture file should be written");

    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        usize::MAX,
        1,
        TRASH_TIME_BUDGET,
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert!(
        summary.truncated,
        "descending past the depth budget should be reported"
    );
    assert_eq!(
        summary.item_count, 2,
        "entries past the depth budget should not be counted (root/sub and sub/nested only)"
    );
}

#[test]
fn trash_summary_treats_an_inaccessible_subdirectory_as_truncated_not_fatal() {
    use std::os::unix::fs::PermissionsExt;

    let root = unique_fixture_root("inaccessible");
    std::fs::create_dir_all(root.join("blocked")).expect("the trash fixture should be created");
    std::fs::create_dir_all(root.join("visible")).expect("the trash fixture should be created");
    std::fs::write(root.join("visible/needle.txt"), b"content")
        .expect("the trash fixture file should be written");
    std::fs::set_permissions(root.join("blocked"), std::fs::Permissions::from_mode(0o000))
        .expect("the fixture directory's permissions should be restrictable");
    let running_as_root = std::fs::read_dir(root.join("blocked")).is_ok();

    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        MAX_TRASH_ENTRIES,
        MAX_TRASH_DEPTH,
        TRASH_TIME_BUDGET,
    ));
    let _ = std::fs::set_permissions(root.join("blocked"), std::fs::Permissions::from_mode(0o755));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect(
        "an inaccessible subdirectory should degrade gracefully, not fail the whole measurement",
    );
    if !running_as_root {
        assert!(
            summary.truncated,
            "the inaccessible branch should be reported as truncated"
        );
        assert_eq!(
            summary.item_count, 3,
            "blocked (uncounted contents) + visible + needle.txt"
        );
    }
}

#[test]
fn trash_summary_treats_a_directory_removed_before_measurement_as_truncated_not_fatal() {
    let root = unique_fixture_root("changing-tree");
    let vanishing = root.join("vanishing");
    std::fs::create_dir_all(&vanishing).expect("the trash fixture should be created");
    std::fs::write(vanishing.join("inner.txt"), b"content")
        .expect("the trash fixture file should be written");

    // Capture the directory's FileInfo the way the parent-level enumeration in
    // `summarize_trash_with_budget` would, then remove it from under that handle. This models a
    // directory that changes or disappears between being observed and being measured.
    let file = gio::File::for_path(&vanishing);
    let context = glib::MainContext::new();
    let info = context
        .block_on(file.query_info_future(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        ))
        .expect("querying the fixture directory's info should succeed while it still exists");
    std::fs::remove_dir_all(&vanishing).expect("the fixture directory should be removable");

    let result = context.block_on(measure_trash_entry(
        file,
        info,
        0,
        Rc::new(MeasurementBudget {
            visited: Cell::new(0),
            deadline: Instant::now() + TRASH_TIME_BUDGET,
            max_entries: MAX_TRASH_ENTRIES,
            max_depth: MAX_TRASH_DEPTH,
        }),
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture root should be removed");

    let summary = result.expect(
        "a directory removed after being observed should degrade gracefully, not fail the whole measurement",
    );
    assert!(
        summary.truncated,
        "measuring an entry that vanished before recursion should be reported as truncated"
    );
}

#[test]
fn aborting_a_trash_measurement_stops_it_mid_flight() {
    let root = unique_fixture_root("abort-mid-flight");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // `next_files_future` batches 64 entries at a time, so 200 files force several suspension
    // points, giving a real window to observe partial progress before the walk would finish.
    let total_files = 200;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    // `spawn_local` and manual `iteration()` polling (unlike `block_on`) require this thread to
    // own the context, hence the explicit acquire via `with_thread_default` below.
    let context = glib::MainContext::new();
    let (progress_before_abort, progress_after_abort) = context
        .with_thread_default(|| {
            let info = context
                .block_on(gio::File::for_path(&root).query_info_future(
                    TRASH_ATTRIBUTES,
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                ))
                .expect("querying the fixture directory's info should succeed");

            let budget = Rc::new(MeasurementBudget {
                visited: Cell::new(0),
                deadline: Instant::now() + TRASH_TIME_BUDGET,
                max_entries: MAX_TRASH_ENTRIES,
                max_depth: MAX_TRASH_DEPTH,
            });
            let task = context.spawn_local(measure_trash_entry(
                gio::File::for_path(&root),
                info,
                0,
                budget.clone(),
            ));

            // Drive the loop only until the walk has made some real progress (at least one batch
            // beyond the root directory itself), then abort immediately -- this is genuinely
            // mid-flight since one batch (64) is far short of the full tree (1 + 200), regardless
            // of exactly how many main-loop iterations it took to get there.
            for _ in 0..1_000 {
                if budget.visited.get() > 1 {
                    break;
                }
                context.iteration(true);
            }
            let progress_before_abort = budget.visited.get();

            task.abort();
            for _ in 0..20 {
                context.iteration(false);
            }

            (progress_before_abort, budget.visited.get())
        })
        .expect("a freshly created main context should be acquirable as thread-default");
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

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
fn trash_summary_stops_enumerating_the_root_once_the_measurement_budget_is_reached() {
    let root = unique_fixture_root("root-budget-stop");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // More than one `next_files_future` batch (64 entries), so a walk that kept fetching
    // further batches after the budget was spent would still show up as a much larger count.
    let total_files = 150;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    let max_entries = 5;
    let summary = glib::MainContext::new()
        .block_on(summarize_trash_with_budget(
            &gio::File::for_path(&root),
            max_entries,
            MAX_TRASH_DEPTH,
            TRASH_TIME_BUDGET,
        ))
        .expect("a plain directory tree should measure without error");
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    assert!(
        summary.truncated,
        "exceeding the measurement budget should still be reported"
    );
    assert_eq!(
        summary.item_count, max_entries,
        "root enumeration should stop as soon as the budget is spent, not keep requesting \
         further `next_files_future` batches for the remaining {total_files} entries"
    );
}

#[test]
fn empty_trash_deletes_every_top_level_entry_in_bounded_batches() {
    let root = unique_fixture_root("empty-trash-streaming");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // More than one `next_files_future` batch, so this also exercises the batch-to-batch loop.
    let total_files = 150;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    let last_progress = Rc::new(Cell::new(0_usize));
    let progress_tick = last_progress.clone();
    let outcome = glib::MainContext::new()
        .block_on(empty_trash(&gio::File::for_path(&root), move |processed| {
            progress_tick.set(processed);
        }))
        .expect("a plain directory tree should empty without error");
    std::fs::remove_dir_all(&root).expect("the trash fixture root should be removed");

    assert_eq!(
        outcome.deleted, total_files,
        "every top-level entry must be deleted, independent of any prior measurement budget"
    );
    assert_eq!(outcome.failed, 0);
    assert_eq!(
        last_progress.get(),
        total_files,
        "progress should account for every entry once the walk finishes"
    );
}

#[test]
fn aborting_empty_trash_stops_deletion_mid_flight() {
    let root = unique_fixture_root("abort-empty-trash-mid-flight");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // More than one `next_files_future` batch (64 entries), so aborting after the first batch's
    // progress callback is genuinely mid-flight, not the whole walk finishing in one step.
    let total_files = 200;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    let context = glib::MainContext::new();
    let (progress_before_abort, progress_after_abort) = context
        .with_thread_default(|| {
            let progress = Rc::new(Cell::new(0_usize));
            let progress_tick = progress.clone();
            let trash_root = gio::File::for_path(&root);
            let task = context.spawn_local(async move {
                empty_trash(&trash_root, move |processed| progress_tick.set(processed)).await
            });

            // Drive the loop only until the first batch has reported progress, then abort
            // immediately -- with 200 files and a 64-entry batch size, that's genuinely
            // mid-flight regardless of exactly how many main-loop iterations it took to get there.
            for _ in 0..1_000 {
                if progress.get() > 0 {
                    break;
                }
                context.iteration(true);
            }
            let progress_before_abort = progress.get();

            task.abort();
            for _ in 0..20 {
                context.iteration(false);
            }

            (progress_before_abort, progress.get())
        })
        .expect("a freshly created main context should be acquirable as thread-default");
    let remaining = std::fs::read_dir(&root)
        .expect("the fixture root should still exist")
        .count();
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    assert!(
        progress_before_abort > 0 && progress_before_abort < total_files,
        "the deletion should have made partial, not complete, progress before it is aborted"
    );
    assert_eq!(
        progress_after_abort, progress_before_abort,
        "aborting mid-flight should stop the deletion from making any further progress"
    );
    assert!(
        remaining > 0,
        "aborting mid-flight should leave undeleted entries behind, not finish the walk anyway"
    );
}

#[test]
fn trash_summary_does_not_stop_enumerating_siblings_after_one_branch_is_depth_truncated() {
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
        .expect("the trash fixture should be created");
    }

    // With max_depth 1, every "sub-N" directory individually hits the depth cap when deciding
    // whether to recurse into its own "inner" child -- that is a branch-local condition, unrelated
    // to its siblings. It used to be conflated with the shared budget being spent, which stopped
    // scanning further `next_files_future` batches entirely: with 80 siblings and a 64-entry batch
    // size, that undercounted "parent" to 1 (itself) + 64 (first batch only) = 65.
    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        usize::MAX,
        1,
        TRASH_TIME_BUDGET,
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert_eq!(
        summary.item_count,
        1 + sibling_count,
        "every sibling should be counted (1 for \"parent\" plus one per sub-N directory), \
         not just the first next_files_future batch"
    );
}
