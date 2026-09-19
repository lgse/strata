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
    cycle_goto_path(input, current, home, &listing, reverse, cycle)
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

    let mut cycle = None;
    assert_eq!(
        complete("nested/de", Some(&current), &home, &[], false, &mut cycle).as_deref(),
        Some("nested/deep")
    );
    assert_eq!(
        complete("nested/deep", Some(&current), &home, &[], false, &mut cycle).as_deref(),
        Some("nested/deeper")
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
