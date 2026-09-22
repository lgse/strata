// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;

use super::{GotoCycle, cycle_goto_path};

fn complete(
    input: &str,
    current: Option<&Path>,
    home: &Path,
    listing: &[&str],
    reverse: bool,
    cycle: &mut Option<GotoCycle>,
) -> Option<String> {
    let listing: Vec<String> = listing.iter().map(|name| (*name).to_owned()).collect();
    glib::MainContext::new()
        .block_on(cycle_goto_path(
            input, current, home, &listing, reverse, cycle,
        ))
        .expect("folder completion")
}

#[test]
fn bare_prefix_cycles_listing_folders_and_skips_files() {
    let home = Path::new("/home/example");
    let current = Path::new("/work");
    let folders = ["docs", "documents", "sub"];
    let mut cycle = None;
    assert_eq!(
        complete("d", Some(current), home, &folders, false, &mut cycle).as_deref(),
        Some("docs")
    );
    assert_eq!(
        complete("docs", Some(current), home, &folders, false, &mut cycle).as_deref(),
        Some("documents")
    );
    assert_eq!(
        complete(
            "documents",
            Some(current),
            home,
            &folders,
            false,
            &mut cycle
        )
        .as_deref(),
        Some("docs"),
        "wraps to the first match"
    );
    assert_eq!(
        complete("a", Some(current), home, &folders, false, &mut cycle),
        None,
        "files are not in the listing folders"
    );
}

#[test]
fn shift_tab_cycles_in_reverse_and_starts_at_the_last_match() {
    let home = Path::new("/home/example");
    let current = Path::new("/work");
    let folders = ["docs", "documents"];
    let mut cycle = None;
    assert_eq!(
        complete("d", Some(current), home, &folders, true, &mut cycle).as_deref(),
        Some("documents")
    );
    assert_eq!(
        complete("documents", Some(current), home, &folders, true, &mut cycle).as_deref(),
        Some("docs")
    );
    assert_eq!(
        complete("docs", Some(current), home, &folders, false, &mut cycle).as_deref(),
        Some("documents")
    );
}

#[test]
fn no_match_keeps_the_typed_text() {
    let home = Path::new("/home/example");
    let mut cycle = None;
    assert_eq!(
        complete(
            "zzz",
            Some(Path::new("/work")),
            home,
            &["sub"],
            false,
            &mut cycle
        ),
        None
    );
    assert_eq!(cycle, None);
}

#[test]
fn slash_completes_in_the_parent_and_tilde_expands_home() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempfile::tempdir()?;
    let current = root.path().join("cwd");
    let home = root.path().join("home");
    fs::create_dir_all(current.join("nested").join("deep"))?;
    fs::create_dir_all(current.join("nested").join("deeper"))?;
    fs::write(current.join("nested").join("deny.txt"), b"file")?;
    fs::create_dir_all(home.join("Downloads"))?;
    fs::create_dir_all(home.join("Documents"))?;
    fs::write(home.join("Download.txt"), b"file")?;
    fs::create_dir_all(home.join(".cache"))?;
    std::os::unix::fs::symlink(
        current.join("nested/deep"),
        current.join("nested/deep-link"),
    )?;
    std::os::unix::fs::symlink(current.join("missing"), current.join("nested/dead-link"))?;

    let mut cycle = None;
    assert_eq!(
        complete("nested/de", Some(&current), &home, &[], false, &mut cycle).as_deref(),
        Some("nested/deep")
    );
    assert_eq!(
        complete("nested/deep", Some(&current), &home, &[], false, &mut cycle).as_deref(),
        Some("nested/deep-link")
    );

    assert_eq!(
        complete(
            "nested/deep-link",
            Some(&current),
            &home,
            &[],
            false,
            &mut cycle
        )
        .as_deref(),
        Some("nested/deeper"),
        "matching directory links participate; dangling links and files do not"
    );
    cycle = None;
    assert_eq!(
        complete("~/dow", Some(&current), &home, &[], false, &mut cycle).as_deref(),
        Some("~/Downloads")
    );
    cycle = None;
    assert_eq!(
        complete("~", Some(&current), &home, &[], false, &mut cycle).as_deref(),
        Some("~/Documents"),
        "~ expands to home and cycles its folders"
    );

    cycle = None;
    assert_eq!(
        complete("~/c", Some(&current), &home, &[], false, &mut cycle),
        None,
        "hidden folders require a '.' prefix"
    );
    assert_eq!(
        complete("~/.", Some(&current), &home, &[], false, &mut cycle).as_deref(),
        Some("~/.cache")
    );
    Ok(())
}

#[test]
fn uris_and_other_homes_are_left_alone() {
    let home = Path::new("/home/example");
    let mut cycle = None;
    assert_eq!(
        complete(
            "smb://host/share",
            Some(Path::new("/work")),
            home,
            &["share"],
            false,
            &mut cycle
        ),
        None
    );
    assert_eq!(
        complete(
            "~other/docs",
            Some(Path::new("/work")),
            home,
            &[],
            false,
            &mut cycle
        ),
        None
    );
}

#[test]
fn large_directory_completion_yields_and_reports_errors_and_limits() {
    use std::{
        cell::Cell,
        rc::Rc,
        time::{Duration, Instant},
    };
    let root = tempfile::tempdir().expect("completion fixture");
    for index in 0..4_096 {
        fs::write(root.path().join(format!("unrelated-{index}")), b"").expect("file");
    }
    fs::create_dir(root.path().join("matching-folder")).expect("matching directory");
    let context = glib::MainContext::new();
    let ticks = Rc::new(Cell::new(0));
    let tick_count = ticks.clone();
    let heartbeat = context.spawn_local(async move {
        loop {
            glib::timeout_future(Duration::ZERO).await;
            tick_count.set(tick_count.get() + 1);
        }
    });
    let start = Instant::now();
    let input = format!("{}/matching", root.path().display());
    let mut cycle = None;
    let result = context
        .block_on(async {
            let result = cycle_goto_path(&input, None, root.path(), &[], false, &mut cycle).await;
            heartbeat.abort();
            result
        })
        .expect("completion");
    assert_eq!(
        result.as_deref(),
        Some(
            root.path()
                .join("matching-folder")
                .to_str()
                .expect("UTF-8 fixture path")
        )
    );
    assert!(
        ticks.get() > 0,
        "main context must run while enumeration is pending"
    );
    eprintln!(
        "local completion: 4096 unrelated files, elapsed {:?}, {} main-context heartbeats",
        start.elapsed(),
        ticks.get()
    );
    assert!(
        context
            .block_on(cycle_goto_path(
                "missing/",
                Some(root.path()),
                root.path(),
                &[],
                false,
                &mut None
            ))
            .is_err()
    );
    let listing = (0..=super::MAX_MATCHES)
        .map(|i| format!("folder-{i}"))
        .collect::<Vec<_>>();
    assert!(
        context
            .block_on(cycle_goto_path(
                "folder",
                None,
                root.path(),
                &listing,
                false,
                &mut None
            ))
            .is_err()
    );
    for name in &listing {
        fs::create_dir(root.path().join(name)).expect("matching folder");
    }
    let input = format!("{}/folder", root.path().display());
    assert!(
        context
            .block_on(cycle_goto_path(
                &input,
                None,
                root.path(),
                &[],
                false,
                &mut None
            ))
            .is_err()
    );
}

#[test]
fn pending_completion_is_cancelled_on_edit_leave_replacement_reset_and_drop() {
    use crate::ui::minimal_mode::{MinimalPrompt, MinimalState};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        time::{Duration, Instant},
    };
    let context = glib::MainContext::new();
    for transition in ["edit", "leave", "replace", "reset", "drop"] {
        let state = Rc::new(RefCell::new(MinimalState::new()));
        state.borrow_mut().enter_prompt(MinimalPrompt::Goto);
        let (send, receive) = futures_channel::oneshot::channel::<()>();
        let started = Rc::new(Cell::new(false));
        let started_worker = started.clone();
        let delivered = Rc::new(Cell::new(false));
        let delivered_worker = delivered.clone();
        state
            .borrow_mut()
            .set_goto_request(context.spawn_local(async move {
                started_worker.set(true);
                if receive.await.is_ok() {
                    delivered_worker.set(true);
                }
            }));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !started.get() {
            assert!(
                Instant::now() < deadline,
                "completion must start before {transition}"
            );
            context.iteration(false);
        }
        match transition {
            "edit" => state.borrow_mut().clear_goto_completion(),
            "leave" => state.borrow_mut().leave_prompt(),
            "replace" => state.borrow_mut().enter_prompt(MinimalPrompt::FindNext),
            "reset" => state.borrow_mut().reset(),
            "drop" => drop(state),
            _ => unreachable!(),
        }
        while !send.is_canceled() {
            assert!(
                Instant::now() < deadline,
                "completion must cancel after {transition}"
            );
            context.iteration(false);
        }
        assert!(
            send.send(()).is_err(),
            "transition {transition} must drop the pending operation"
        );
        assert!(!delivered.get(), "cancelled completion cannot publish");
    }
}

#[test]
fn relative_goto_input_joins_the_current_folder() {
    let current = Path::new("/work/cwd");
    assert_eq!(
        super::resolve_goto_input("documents", Some(current)),
        "/work/cwd/documents"
    );
    assert_eq!(
        super::resolve_goto_input("~/Downloads", Some(current)),
        "~/Downloads"
    );
    assert_eq!(
        super::resolve_goto_input("/tmp/out", Some(current)),
        "/tmp/out"
    );
    assert_eq!(
        super::resolve_goto_input("smb://host/share", Some(current)),
        "smb://host/share"
    );
}
