// SPDX-License-Identifier: MIT

//! The rows' metadata fills: each column asks the browser for what its rows lack, and a
//! folder's item count only while sizes show, as GTK's Columns rows ask (`request_metadata` in
//! `src/ui/browser/columns/rows/bind.rs`).

use strata_core::model::{FileEntry, MetadataValue};

use super::{MODULE, click, fixture, folder_name, open, rest, save_settings};
use crate::{
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted_after},
};

/// Column `depth`'s entry named `name`, as the browser holds it.
fn entry(host: &Offscreen, depth: usize, name: &str) -> Option<FileEntry> {
    host.app()
        .browser()
        .with_column_entries(depth, |entries| {
            entries
                .iter()
                .find(|entry| entry.display_name == name)
                .cloned()
        })
        .flatten()
}

fn filled(host: &Offscreen, depth: usize, name: &str) -> bool {
    entry(host, depth, name).is_some_and(|entry| {
        entry.modified_unix_seconds != MetadataValue::Unknown
            && entry.mode != MetadataValue::Unknown
    })
}

fn sizes_follow_the_setting(name: &str, show_sizes: bool) {
    let seed = format!("columns_show_sizes = {show_sizes}\n");
    hosted_after(
        MODULE,
        name,
        move || save_settings(&seed),
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());
            let reached = host.run_until(PATIENCE, |host| {
                filled(host, 0, "readme.md") && filled(host, 0, "documents")
            });
            assert!(reached, "the first column's rows were never filled");
            click(&mut host, &folder_name(folder.path()), "documents");
            let reached = host.run_until(PATIENCE, |host| filled(host, 1, "notes.txt"));
            assert!(reached, "an opened column's rows were never filled");
            rest(&mut host);

            let file = entry(&host, 0, "readme.md").expect("readme.md");
            assert_ne!(file.size, MetadataValue::Unknown, "a file's size");
            let count = entry(&host, 0, "documents").expect("documents").child_count;
            assert_eq!(
                count != MetadataValue::Unknown,
                show_sizes,
                "a folder's item count with sizes shown {show_sizes}: {count:?}"
            );
        },
    );
}

#[test]
fn a_column_fills_its_rows_and_a_folders_count_while_sizes_show() {
    sizes_follow_the_setting(
        "fills::a_column_fills_its_rows_and_a_folders_count_while_sizes_show",
        true,
    );
}

#[test]
fn a_column_asks_no_folder_count_while_sizes_are_hidden() {
    sizes_follow_the_setting(
        "fills::a_column_asks_no_folder_count_while_sizes_are_hidden",
        false,
    );
}
