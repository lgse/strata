// SPDX-License-Identifier: MIT

//! The rows' metadata fills: a column asks the browser for what its rows lack, and a folder's
//! item count only while sizes show, as GTK's Columns rows ask (`request_metadata` in
//! `src/ui/browser/columns/rows/bind.rs`).

use strata_core::model::{FileEntry, MetadataValue};

use super::{MODULE, fixture, open, rest, save_settings};
use crate::{
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted_after},
};

/// The first column's entry named `name`, as the browser holds it.
fn entry(host: &Offscreen, name: &str) -> Option<FileEntry> {
    host.app()
        .browser()
        .with_column_entries(0, |entries| {
            entries
                .iter()
                .find(|entry| entry.display_name == name)
                .cloned()
        })
        .flatten()
}

fn filled(host: &Offscreen, name: &str) -> bool {
    entry(host, name).is_some_and(|entry| {
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
                filled(host, "readme.md") && filled(host, "documents")
            });
            assert!(reached, "the column's rows were never filled");
            rest(&mut host);

            let file = entry(&host, "readme.md").expect("readme.md");
            assert_ne!(file.size, MetadataValue::Unknown, "a file's size");
            let count = entry(&host, "documents").expect("documents").child_count;
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
